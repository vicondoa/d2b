# ADR 0049: Store-owned mutation seal for verified-only resource writes

- Status: Proposed
- Date: 2026-08-01
- Related: [ADR 0046](0046-d2b-3-provider-control-plane.md) (d2b 3.0 provider
  control plane) and its normative decision D106 in
  [`docs/specs/ADR-046-decision-register.md`](../specs/ADR-046-decision-register.md);
  [ADR 0015](0015-daemon-only-clean-break.md) for the daemon-only control
  plane the resource plane sits inside
- Scope: `packages/d2b-resource-store`, `packages/d2b-resource-store-redb`,
  `packages/d2b-resource-api`

## Context

D106 requires that a mutating resource commit be reachable only through
evidence that a successful native authorization evaluation produced, bound to
exactly one store instance. The same decision fixes a dependency invariant that
the solution must respect: `d2b-resource-store` and `d2b-resource-store-redb`
never depend on `d2b-resource-api`, never import the evaluator, and contain no
`Role` or `RoleBinding` symbol.

Those two requirements pull against each other. The verifier that may mint
evidence lives in the upper crate. The backend that must consume it lives in a
crate that may not name the upper crate. Wave 5 resolved that tension by
putting a public view trait in the lower crate:

```rust
// packages/d2b-resource-store-redb/src/lib.rs at dc145025
pub trait VerifiedMutationView: Send {
    type Prepared: VerifiedPreparedMutationView;
    fn authorization(&self) -> &AdmittedAuthorization;
    fn policy_snapshot(&self) -> PolicySnapshot;
    fn operation(&self) -> &StoreOperationContext;
    fn mutations(&self) -> &[Self::Prepared];
}

pub struct RedbResourceStore<V> { /* ... */ }

impl<V> RedbResourceStore<V> where V: VerifiedMutationView + 'static {
    pub async fn commit_verified(&self, mutation: V) -> Result<StoreCommitResult, StoreError>;
}
```

The trait is public and unsealed, so any crate in the graph can implement it
for a local type and hand the writer a fully attacker-chosen
`AdmittedAuthorization`, `PolicySnapshot`, and mutation list. Every field the
backend reads to decide what to persist is forgeable. `AdmittedAuthorization`,
`PolicySnapshot`, `StoreOperationContext`, and `StoreMutation` are already
public structs with public fields in `d2b-resource-store`, so the forgery needs
no privilege at all.

W5fu1 (`f73c4157`) claimed to close this with a runtime guard in `start()`:

```rust
#[cfg(not(test))]
if std::any::type_name::<V>() != "d2b_resource_api::admission::VerifiedMutation" {
    return Err(transaction::integrity("verified-mutation-type-binding-invalid"));
}
```

That guard does not close the hole, and this ADR records the measurement
rather than the argument, because the argument had already been made and
believed once.

**Measured bypass 1: the type name is a namespace claim, not an identity.**
A minimal reproduction of the shape above, with a hostile crate declaring
`[lib] name = "d2b_resource_api"` and `mod admission { pub struct
VerifiedMutation; }`, passes the guard exactly and commits:

```text
$ cargo test -p hostile-downstream -- --nocapture
COMMITTED: forged-authorization: subject=root verb=delete targets=*
test downstream_forges_verified_evidence_and_commits ... ok
```

Cargo forbids two libraries with the same name in one build, so the collision
cannot happen inside this workspace. It happens in exactly the population the
seal exists for: a downstream crate that depends on `d2b-resource-store-redb`
and not on the real `d2b-resource-api`. Nothing stops such a crate from
choosing that library name.

**Measured bypass 2: `#[cfg(not(test))]` erases the guard under the harness
this repository already runs.** `packages/d2b-resource-api/tests/external_seals.rs`
deliberately forces `--cfg test` onto a dependency through a rustc shim, so
that a test-only escape hatch cannot open a hole the seal claims is closed.
Applying that same shim to the backend crate in the reproduction:

```text
start() with a non-matching type name -> true
rejected = false
```

An unrelated type is accepted. A seal that the repository's own strongest
negative-test harness switches off is not a seal.

Two further properties make the shape wrong independent of the two bypasses.
The check runs at `start()`, so it binds a type parameter once at open and
never binds the evidence to the mutation, the store, or the commit. And a
runtime string comparison cannot be asserted by the compile-fail and
API-surface census legs that this repository uses for every other capability
boundary, so the guard is invisible to the machinery that would otherwise
notice it regressing.

The W5 branch is parked at `dc145025` with the attempted follow-up deliberately
uncommitted. Every claim about current behaviour in this record is taken from
committed code at that tip; the parked work is named only where a reader would
otherwise go looking for a symbol that is not there.

## Decision

Move the mutation authorization capability **down** into
`packages/d2b-resource-store`, the crate both sides already depend on, and make
it a private concrete type rather than a trait. The verifier mints; the backend
consumes; nothing else can do either.

### 1. Which crate owns the capability

`d2b-resource-store` owns it, in a new module
`packages/d2b-resource-store/src/mutation_seal.rs`.

Exactly two crates are common to both sides. Measured from the three manifests
at `dc145025`: `d2b-resource-api` depends on `d2b-contracts`,
`d2b-core-controller`, `d2b-resource-store`, and `d2b-resource-store-redb`;
`d2b-resource-store-redb` depends on `d2b-contracts`,
`d2b-controller-toolkit`, and `d2b-resource-store`. So the candidates are
`d2b-contracts` and `d2b-resource-store`.

`d2b-contracts` is rejected: it is the wire-contract crate, its types are
serialized, schema-generated, and drift-gated, and a process-local capability
that has no wire form and must never acquire one does not belong in the crate
whose whole purpose is the wire form.

`d2b-resource-store` is the right home. Placing the capability there needs no
new dependency edge, creates no cycle, and leaves D106's "never depend on
`d2b-resource-api`" invariant untouched. It already owns every payload type the
evidence carries (`StoreMutation`, `AdmittedAuthorization`, `PolicySnapshot`,
`StoreOperationContext`), so the evidence wrapper was the one piece living in
the wrong crate.

### 2. What representation the capability takes

A private concrete struct. Not a trait, sealed or otherwise; not a closure;
not a type parameter.

```rust
// packages/d2b-resource-store/src/mutation_seal.rs
mod authority {
    pub struct SealAuthority;
}

/// Declared identity of exactly one provisioned store, plus the bounded
/// correlator an operator uses to find it in their configuration.
///
/// `zone` and `store_uuid` mirror what `StoreIdentity` actually holds and are
/// the only two components compared. The store UUID is the discriminator:
/// two stores in one Zone share `zone` and `zone_uid` and differ only here, so
/// keying on `zone_uid` would make the declared check a no-op for the most
/// plausible multi-store case. `slot` is section 2c's correlator; it is never
/// compared, because it names a position in configuration and not a store.
///
/// This type implements no comparison trait. The one comparison lives in a
/// private function that names `zone` and `store_uuid` and cannot silently
/// acquire `slot`; section 2c says why that matters and section 6 seals it.
#[derive(Clone)]
pub struct StoreSealIdentity {
    slot: StoreSlot,
    zone: ZoneId,
    store_uuid: ResourceUid,
}

impl StoreSealIdentity {
    pub fn new(slot: StoreSlot, zone: ZoneId, store_uuid: ResourceUid) -> Self;
    /// The only readable identity component. There is deliberately no
    /// `store_uuid` accessor, matching `StoreIdentity`, which exposes `zone`
    /// and `zone_uid` and has never exposed its UUID.
    pub const fn zone(&self) -> &ZoneId;
    /// The authorized operator-facing correlator. Renderable; section 2c.
    pub const fn slot(&self) -> StoreSlot;
}

/// Payload the verifier prepares. Data, not evidence.
pub struct MutationSealBody {
    pub mutations: Vec<PreparedStoreMutation>,
    pub authorization: AdmittedAuthorization,
    pub policy_snapshot: PolicySnapshot,
    pub operation: StoreOperationContext,
}

/// The evidence. Private fields, no public constructor, no Clone, no Copy.
pub struct SealedMutation { /* authority, store, body: all private */ }

/// Payload after acceptance. Only an acceptor can produce one.
pub struct OpenedMutation { /* private */ }

impl OpenedMutation {
    /// The writer needs all four payloads, so acceptance has to hand them
    /// back. Reachable only by whoever could open the evidence.
    pub fn body(&self) -> &MutationSealBody;
    pub fn into_body(self) -> MutationSealBody;
}

pub struct MutationSealIssuer { /* private */ }
pub struct MutationSealAcceptor { /* private */ }

/// The single governed mint surface. Both halves leave together.
pub fn mutation_seal_pair(
    store: StoreSealIdentity,
) -> (MutationSealIssuer, MutationSealAcceptor);

impl MutationSealIssuer {
    /// The only constructor of SealedMutation in the dependency graph.
    pub fn seal(&self, body: MutationSealBody) -> SealedMutation;
}

impl MutationSealAcceptor {
    /// Field-wise comparison of `zone` then `store_uuid`, never `slot`, for
    /// the operator message and for the reason-code selection in
    /// `open_owned`. Never an authorization gate on its own.
    pub fn diagnose(&self, store: &StoreSealIdentity)
        -> Result<(), SealIdentityMismatch>;
    /// The slot this acceptor was minted for, so `open_owned` can refuse an
    /// acceptor whose correlator disagrees with the store's. Section 2c.
    pub const fn declared_slot(&self) -> StoreSlot;
    /// Consumes the evidence. Succeeds only for this acceptor's paired issuer.
    pub fn open(&self, sealed: SealedMutation) -> Result<OpenedMutation, StoreError>;
}
```

A trait is the wrong representation here for a reason that generalises: a
trait is an invitation to implement, and the only way to withdraw that
invitation is a private supertrait, which requires the sealing crate to be able
to name every permitted implementor. The lower crate structurally cannot name
the upper crate's type. A concrete type has no such requirement, because
privacy of fields is enforced without naming anyone.

The identity mechanism is the one already committed one layer up, not a new
one. `AdmittedMutation` in `packages/d2b-resource-api/src/admission.rs` carries
`Arc<AdmissionAuthority>` and `Arc<StoreIdentityAuthority>`, and
`StoreAdmissionBinding::verify` already decides on two `Arc::ptr_eq` tests.
`Arc<SealAuthority>` is the same construction moved down a crate. That it works
for a zero-sized authority was measured rather than assumed: `Arc` allocates an
`ArcInner` regardless of payload size, and 10000 concurrently live
`Arc<SealAuthority>` values yielded 10000 distinct addresses on the pinned
toolchain. A `SealedMutation` holds its authority by `Arc`, so the allocation
cannot be freed and its address cannot be reused while evidence referencing it
exists.

`mutation_seal_pair` is public and that is a cost, not a feature. The committed
analogue is `pub(crate) fn admission_pair()` in `admission.rs`, and
`packages/d2b-resource-store/tests/d106_policy.rs` asserts that
`pub fn admission_pair` never appears. That assertion cannot be reproduced
here: the issuer is used by `d2b-resource-api` and the acceptor by
`d2b-resource-store-redb`, two crates that are not the defining crate, and Rust
has no friend-crate visibility. `pub` is forced by the placement, so the
placement has to pay for it in section 6.

What `pub` does not buy an attacker is the production store. A downstream crate
can call `mutation_seal_pair`, obtain an issuer, and mint a `SealedMutation`.
Against the store d2bd wired, that value is inert: `open` accepts only evidence
carrying the same `Arc<SealAuthority>` the acceptor holds, and no downstream
can obtain the production acceptor's authority. It also cannot read one: a real
`SealedMutation` presented to a foreign acceptor fails `open`, so its body
never becomes an `OpenedMutation`. What `pub` does leave open is a store the
attacker builds for itself, which section "Consequences" states plainly rather
than leaving implied.

`d2b-session` takes the same posture with `SessionAcceptor`, whose
`from_verified_adapter` constructor is public: the security comes from instance
identity and consumption, not from the absence of a constructor.

The public mint also buys back something the `type_name` guard had to steal.
That guard was written `#[cfg(not(test))]` because `d2b-resource-store-redb`'s
own unit tests must construct a store and commit through it, and under a
crate-private mint they could not. With `mutation_seal_pair` public, those
tests pair their own issuer and acceptor and exercise the real write path in
the real configuration, so `mutation_seal.rs` needs no `cfg` at all. That is
invariant 5, and it is the same property measured bypass 2 destroyed.

The same applies one crate up. A test double implementing
`ResourceStoreBackend` can no longer inspect what it was asked to commit
unless it holds the acceptor paired with the issuer the authorizer used, since
a `SealedMutation` it cannot open never becomes an `OpenedMutation`. Tests that
assert on committed content therefore take the acceptor from
`NativeAuthorizer::take_store_seal` and hand it to the double, which is the
production wiring with a different backend on the end, not a bypass of it.

### 2a. What the seal types must not implement

No type declared in `mutation_seal.rs` implements `Debug`, `Display`,
`Serialize`, `Deserialize`, `JsonSchema`, `PartialEq`, or `Eq`. That covers
`SealedMutation`, `OpenedMutation`, `MutationSealIssuer`,
`MutationSealAcceptor`, `MutationSealBody`, `StoreSealIdentity`, and
`SealAuthority`. The first five are this section's rule; the two comparison
traits are section 2c's, and both are enforced by the same scan. The approved
capability trait-impl allowlist at
`packages/d2b-bus/tests/approved-capability-trait-impls.txt` therefore gains
zero rows, not four.

An earlier revision of this record planned a redacted `Debug` on the four
capability types, on the precedent of `SessionAcceptor`, whose `Debug` writes
`SessionAcceptor(<redacted>)`. That precedent is real but it is the wrong one
for this crate, and the measurement says so. In the resource plane at
`dc145025`, `VerifiedMutation` - the exact type `SealedMutation` replaces -
implements nothing, and neither do `RedbResourceStore`, `NativeAuthorizer`, or
`ResourceService`, the three types that would hold a seal. Nothing in the graph
needs `Debug` on any of them, so adding one buys no diagnosis and costs a live
impl a later author can widen. Denial is the smaller closed surface, and it is
the one already committed here.

Denial is also stronger than redaction, because it moves the leak from
forbidden to unrepresentable. `tracing`'s `?` sigil requires `Debug` and its
`%` sigil requires `Display`; a struct implementing neither cannot be an event
field, a span attribute, or a label at all. Measured on rustc 1.97.0:

| Attempt | Diagnostic |
| --- | --- |
| `{:?}` on a non-`Debug` type | `error[E0277]`, `` `SealedMutation` doesn't implement `Debug` `` |
| `{}` on a non-`Display` type | `error[E0277]`, `` `StoreSealIdentity` doesn't implement `std::fmt::Display` `` |

A constant-string `Debug` would instead make `tracing::debug!(?sealed)`
compile, emit nothing useful, and read to the next author as sanctioned. The
three crates emit no telemetry today - `d2b-resource-store`,
`d2b-resource-store-redb`, and `d2b-resource-api` contain zero `tracing::` and
zero metric call sites at `dc145025` - so this closes the surface before the
first emitter arrives rather than after it has shipped.

There is a second, independent layer that this decision does not rely on but
should be recorded so a future reader does not mistake it for the guard. Both
components of a seal identity are already redaction-safe in `d2b-contracts`:
`ZoneId` comes from the `label_identity!` macro, whose generated `Debug` and
`Display` both render `ZoneId(<redacted>)`, and `ResourceUid` carries the same
pair of hand-written impls. Even a reintroduced derive could not print either
value. The primary guard is still the absent impl, because the payload types a
`SealedMutation` carries are not all so protected.

The one thing a caller may learn about the identities themselves is which
declared component disagreed, named and never valued. That type lives in
`packages/d2b-resource-store/src/error.rs` alongside `StoreError`, not in
`mutation_seal.rs`, so the no-`Debug` rule for that file stays absolute:

```rust
/// Which declared component of a seal identity disagreed. Names the
/// component; the values themselves are never rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealIdentityMismatch {
    /// The declared Zones disagree. Selected first, so a `Store` result
    /// always means the Zones already agreed.
    Zone,
    /// The Zones agree and the declared store UUIDs disagree.
    Store,
}

impl SealIdentityMismatch {
    /// The stable acceptor-install reason code this mismatch selects.
    pub const fn reason_code(self) -> &'static str;
}
```

An earlier revision of this record made this a struct of two booleans,
`zone_matches` and `store_uuid_matches`, with `is_match` and an
`Option<&'static str>` reason code. Four representable states for a two-state
domain: `(true, true)` duplicates the `Ok` arm that `diagnose` already
expresses, and `(false, true)` claims two stores in different Zones collided on
a UUIDv4, which is not a diagnosis anyone should be asked to read. `diagnose`
therefore returns `Result<(), SealIdentityMismatch>` and the type carries only
the two states that can actually occur. `Result` rather than `Option` because
the caller's use is `map_err` onto the reason code, and because `Ok(())` reads
as "the identities agree" where `None` reads as "nothing to say". The
consequences are that `is_match` disappears into `Result::is_ok`, the reason
code becomes total, and a future third identity component cannot be added
without adding a variant here, which is the point.

This is the shape the crate already uses to diagnose without disclosing:
`AdmittedAuthorization`'s `Debug` prints `target_count` and nothing else, and
`StoreOperationContext`'s prints `has_idempotency_key` and `has_trace_id` while
withholding `operation_id` and `correlation_id`, which are plausibly harmless
and still withheld.

Stated as a prohibition, because a future reader will look for one: the store
UUID, the `SealAuthority` pointer value, and the host database path must never
appear in an error message, a log line, a metric label, a span attribute, or an
audit record. There is no authorized rendering surface for any of the three.
`ResourceUid::as_str` and `ResourceUid::to_canonical_string` are documented for
"an authorized encoding or key surface"; a diagnostic is not one. The
operator-actionable substitute is the reason code, the disagreeing component,
and the bounded store slot that section 2c defines; section 4 gives the
remediation for each code.

### 2b. What identifies a store

`StoreSealIdentity` holds `store_uuid: ResourceUid`, not `String`.
`ResourceUid` already exists in `d2b-contracts` at
`packages/d2b-contracts/src/v3/identity.rs`, `d2b-resource-store` already
depends on `d2b-contracts` and nothing else, and `d2b-resource-store` already
names `ResourceUid` in `AdmittedAuthorization::subject_uid`. So this costs no
new type, no new dependency, and no new census identity: `ResourceUid` is
already a `claim_roots` entry in `tests/golden/api-surface/roots.json`.

A new `StoreUuid` newtype was rejected for the same reason a new dependency
was: it would duplicate validation that is already committed, already schema-
generated, and already governed, and two UUID types in one crate is a bug
waiting for a `From` impl.

The three semantics an implementer needs, measured rather than described:

- **Validation.** `ResourceUid::parse` accepts exactly a canonical lowercase
  RFC 9562 UUIDv4: 36 bytes, hyphens at indices 8, 13, 18, 23, ASCII hex
  elsewhere with uppercase rejected, version nibble `4` at index 14, and
  variant nibble in `{8, 9, a, b}` at index 19. Anything else is
  `IdentityError::{Empty, TooLong, InvalidShape}`.
- **Normalization.** There is none, deliberately. `parse` rejects uppercase
  rather than folding it, so the canonical form is the only accepted form.
  Adding a normalizing transform would widen the accepted set and turn two
  distinct byte strings into one identity; do not add one.
- **Equality.** Derived byte equality over the canonical string, with `Eq`,
  `Ord`, and `Hash` derived alongside. Because only the canonical form parses,
  byte equality is UUID equality. It is **not** constant-time and must not be
  made so: this comparison is the diagnosable check, never the authorization
  check. The unforgeable check is `Arc::ptr_eq` on `SealAuthority`, which runs
  first and denies before any UUID is compared. Introducing a constant-time
  comparison here would imply the UUID is a secret that gates the write, which
  is exactly the false belief this record exists to prevent.

### 2c. What identifies a store to an operator

Section 2a is right about every value it withholds and silent about what that
leaves an operator holding. A daemon serving more than one Zone opens more than
one store, and every refusal in section 4 denies with a stable reason code and
no identity at all. In a single-store deployment that is complete. In a
multi-store startup the operator learns that a store refused its acceptor and
has no way to say which one: the store UUID is prohibited, the Zone renders as
`ZoneId(<redacted>)`, and the database path falls under the same prohibition as
the UUID. Redaction that is correct about identity leaves a multi-instance
failure unlocatable unless a non-secret correlator is designed in beside it.

The correlator is a bounded ordinal the composition root assigns. It lives in
`packages/d2b-resource-store/src/error.rs` beside `SealIdentityMismatch`, for
the same reason: it is renderable, and `mutation_seal.rs` renders nothing.

```rust
/// Upper bound on the stores one composition root may open.
pub const MAX_STORE_SLOTS: usize = 64;

/// Zero-based position of one store in the composition root's declared store
/// list. A correlator, never an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreSlot(u8);

impl StoreSlot {
    pub fn new(index: u32) -> Result<Self, StoreSlotError>;
    pub const fn get(self) -> u32;
}

/// Slot index exceeded the composition bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreSlotError;
```

`StoreSlot` implements `Display` as the decimal index and nothing else, and
that is the whole of its authorized rendering. The shape is deliberately the
mirror of `MutationOrdinal`, which the same file already uses to name the
failing mutation in a bounded batch without disclosing the mutation:
`MutationOrdinal::new` rejects at `MAX_BATCH_MUTATIONS`, `StoreSlot::new`
rejects at `MAX_STORE_SLOTS`, and `StoreError`'s own doc comment already reads
"only API-safe optional metadata". The problem section 2a leaves open is the
problem this crate already solved one level down, so it gets the same answer
rather than a new one. `MAX_STORE_SLOTS` lives in `d2b-resource-store` and not
in `d2b-contracts` where `MAX_BATCH_MUTATIONS` lives, because a batch bound is
a protocol bound a client must know and a composition bound is process-local
with no wire form; section 1 rejected `d2b-contracts` on exactly that test.

**Assignment.** The slot is the zero-based index of the store in the
composition root's declared store list, under a total order fixed by the
configuration itself: declaration order for a sequence, lexicographic order of
the configuration key for a mapping, and nothing else. It is a pure function of
the declared configuration, so it is stable across restarts and identical on
two hosts given identical configuration. It is never derived from the store
UUID, the database path, the `SealAuthority` address, a hash, or the order in
which opens happen to complete. A configuration declaring more than
`MAX_STORE_SLOTS` stores fails closed at assignment with `StoreSlotError` and
the daemon does not start.

**How an operator maps it back.** Slot `N` is the `N`-th entry, counting from
zero, of the store list the operator wrote. There is no lookup table, no
runtime registry, and deliberately no emitted mapping line: a mapping line
would have to name the entry, and the entry's name is operator-authored text,
which is the unrestricted label section 2a prohibits. Counting entries in one's
own configuration is the entire mapping, and it works precisely because the
operator is the only party who holds both halves.

**What it discloses.** The position of the failing store, and an upper bound on
how many stores the daemon was configured with. That is not nothing, and it is
stated rather than implied. It discloses no Zone, no UUID, no path, no
credential, and no operator-authored name; section 2a's prohibitions are
extended by this section, never relaxed.

**Where it is attached.** Three typed surfaces carry it, and no others:

- `StoreSealIdentity` carries it, so a store that can be sealed has a slot by
  construction. There is no constructor that omits one.
- `StoreError` gains `store_slot: Option<StoreSlot>`, an accessor, and one more
  row in its hand-written `Debug`, exactly as `mutation_ordinal` is carried
  today. Both existing constructors set `None`, so no committed call site
  changes; a new `const fn with_store_slot` sets it. The value is always the
  slot of **the store that produced the error**, never a counterparty's.
- `StoreSealHandoffError` carries it as a typed field on both variants and
  renders it in the `Display` sentence, because a composition-time refusal that
  names no store is the exact failure this section exists to close.

**Telemetry.** These three crates emit nothing today, measured in section 2a,
so this decision adds no log line, span, or metric. The obligation it creates
binds the composition root that wires a store: every startup diagnostic emitted
for a store carries `store_slot` with that store's value on every path,
including the success path, so an operator correlating a refusal against the
lines around it is reading one field and not two. `store_slot` is bounded at
`MAX_STORE_SLOTS`, so it is safe as a span attribute, a log field, and a metric
label if a later wave adds one. Nothing else from this decision may become any
of the three.

**What it is not.** A slot is never persisted. `StoreMeta` does not gain one,
because a slot is a property of a composition and a durable record carrying it
would be wrong the first time an operator reorders their configuration. It
never appears on the wire, in a schema, or in an audit record. And it is never
compared for identity: `diagnose` compares `zone`, then `store_uuid`, and
stops.

**The one guard the correlator needs.** `StoreSealIdentity::new` takes the slot
from its caller, so an acceptor minted for the entry at slot 1 can be installed
into the store at slot 3, after which every error that store produces names
slot 1. A correlator that can name the wrong configuration entry is worse than
none, because the operator then edits a store that was working. `open_owned`
therefore compares `MutationSealAcceptor::declared_slot` against the store's
own slot after it compares identity, and refuses a disagreement with
`mutation-seal-acceptor-slot-mismatch`. This is not an authorization check and
must never be described as one; it is what makes the slot that `open` reports
provably the store's own, which is the property every message in section 4
depends on. `declared_slot` is the one accessor the acceptor exposes beyond
`diagnose` and `open`, and it exposes a correlator, not a component of the
identity.

### 3. How the direction works

Unchanged edges: `d2b-resource-api` depends on `d2b-resource-store` and on
`d2b-resource-store-redb`; `d2b-resource-store-redb` depends on
`d2b-resource-store`. No new edge, no cycle, no inversion.

The capability flows down, not up. `NativeAuthorizer` gains a once-only
handoff that keeps the issuer and yields the acceptor:

```rust
// packages/d2b-resource-api
impl NativeAuthorizer {
    /// Once only, per authorizer. Retains the issuer privately.
    pub fn take_store_seal(
        &self,
        store: StoreSealIdentity,
    ) -> Result<MutationSealAcceptor, StoreSealHandoffError>;
}

/// Why a store-seal handoff was refused. Carries the bounded store slot from
/// section 2c, no store UUID, and no authority value; see section 2a.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreSealHandoffError {
    /// This authorizer already yielded its one acceptor.
    AlreadyTaken { slot: StoreSlot, zone: ZoneId },
    /// A previous handoff panicked and the seal state is unrecoverable.
    AuthorizerUnavailable { slot: StoreSlot, zone: ZoneId },
}
```

The `Display` text is a fixed remediation sentence whose only identifier is the
slot: `AlreadyTaken` renders "native authorizer already yielded its store-seal
acceptor, refused for store slot {slot}; construct one NativeAuthorizer per
resource store", and `AuthorizerUnavailable` renders "native authorizer
store-seal state is poisoned at store slot {slot}; this process must not
continue serving the zone". The slot is what makes both sentences actionable
in a multi-store startup, and it is renderable because section 2c says it
carries no identity. The `zone` field is typed context for a composition root
that has an authorized rendering surface, not message text: `ZoneId`'s own
`Debug` and `Display` are redacted, so the derived `Debug` on this enum
discloses the slot and nothing else.

This is a second entry in the committed `take_store_binding` pattern -
`ResourceService::new` already calls it behind a `Mutex<Option<_>>` - but it
deliberately does not reuse that method's error type. `StoreBindingError` is a
unit struct whose one `Display` line is "native authorizer is already bound to
a store backend", and `take_store_binding` returns it both for a second take
and for a poisoned lock. An operator cannot act on the conflated value: one
case is a composition bug they fix by building a second authorizer, the other
is a process that must not continue. `StoreBindingError` itself is left
untouched, because widening a public unit struct into an enum would break the
committed `assert_eq!(second.unwrap_err(), StoreBindingError)` in
`d2b-resource-api`, and that is a change this decision does not need.

There is no zone check on `take_store_seal`, and the absence is deliberate
rather than an oversight: `NativeAuthorizer` holds `catalog`, `policy`,
`cache`, `admission`, and `store_binding`, and no Zone identity, so it has
nothing to compare the requested `StoreSealIdentity` against. Identity
mismatch is caught where the identity actually lives, at `open_owned` and at
`open`, and section 4 gives both.

Composition order becomes, once per declared store, with the slot taken from
the enumeration and from nothing else:

```rust
for (index, declared) in stores.iter().enumerate() {
    let slot = StoreSlot::new(u32::try_from(index)?)?;
    let identity = StoreIdentity::new(slot, /* store_uuid, zone, ... */);
    let authorizer = Arc::new(NativeAuthorizer::new(/* ... */));
    let acceptor = authorizer.take_store_seal(identity.seal_identity())?;
    let backend = RedbResourceStore::open_owned(file, identity, acceptor).await?;
    let service = ResourceService::new(Arc::new(RedbBackend::new(backend)), authorizer)?;
}
```

One `NativeAuthorizer` per store is not incidental to the loop; section 4's
third quiet failure is what happens when the authorizer is hoisted out of it.
Every diagnostic this loop emits, on the success path as well as the four
fallible ones, carries `store_slot = slot.get()`; that is the section 2c
obligation, and it binds the wave that wires the resource plane into `d2bd`,
which does not exist at this branch tip.

`ResourceService::new` fails closed with `StoreBindingError` when the
authorizer it is handed never issued a seal. That is a composition-time check
on a necessary condition, not a sufficient one: it cannot observe which store
the acceptor reached, so an acceptor that was taken and then dropped, or
installed into a different store, still constructs a service. The guarantee in
that case is denial at the first commit with
`mutation-seal-authority-mismatch`, never a degraded write. Both points are
fail-closed; only the first is early.

The issuer type is never named in `d2b-resource-api`'s public surface. The
acceptor crosses the API-to-redb boundary exactly once, by value.

### 4. Single use, store binding, mutation binding, replay

**Single use is structural.** `SealedMutation` implements neither `Clone` nor
`Copy`, `MutationSealAcceptor::open` takes it by value, and
`RedbResourceStore::commit_verified` takes it by value. A value moved into a
commit cannot be presented again. There is no nonce ledger and there will not
be one: safe Rust already excludes the replay a ledger would catch, and a
ledger would add unbounded per-store state on the path carrying ADR 0046's
normative p95 durable-commit budget.

The whole authorization chain is by-value, so one evaluation yields at most
one commit:

```text
evaluator allow
  -> AdmissionIssuer::record_allow  -> AdmissionPermit    (moved by admit)
  -> AdmissionPermit::admit         -> AdmittedMutation   (moved by verify)
  -> StoreAdmissionBinding::verify  -> MutationSealBody   (moved by seal)
  -> MutationSealIssuer::seal       -> SealedMutation     (moved by commit)
  -> MutationSealAcceptor::open     -> OpenedMutation     (consumed by the txn)
```

No type in that chain is `Clone` or `Copy`, but the chain is only as strong as
its narrowest link, and that link is not `MutationSealBody`. A body is a
public-field struct whose every field type is `Clone`, so any crate can build
or duplicate one; what a crate cannot do is seal it. Single use therefore rests
on two facts and not on the body: `StoreAdmissionBinding::verify` consumes the
`AdmittedMutation` an allow produced, and `SealedMutation` is neither `Clone`
nor `Copy`. The consequence is that `MutationSealIssuer::seal` is itself a mint
call site and has to be counted as one; see invariant 2 and section 6.

**Store identity is bound twice.** `open` first compares
`Arc::ptr_eq` on the private `SealAuthority`, which is the unforgeable check;
it then compares the two declared identity components field-wise through the
private comparison that backs `diagnose`, which is the diagnosable one.
`open_owned` additionally refuses an acceptor whose declared identity does not
match the store it is being installed into, and splits that refusal by which
component disagreed, because the two cases have different remediations; it
then refuses a slot disagreement, per section 2c. Five fail-closed points, each
with a stable reason string and each carrying the slot of the store that
produced it:

| Condition | Reason | Kind | Retry |
| --- | --- | --- | --- |
| Foreign issuer pair | `mutation-seal-authority-mismatch` | `InternalIntegrityFailure` | `Never` |
| Evidence declares another store | `mutation-seal-store-identity-mismatch` | `InternalIntegrityFailure` | `Never` |
| Acceptor installed cross-Zone | `mutation-seal-acceptor-zone-mismatch` | `InternalIntegrityFailure` | `Never` |
| Acceptor installed into a sibling store | `mutation-seal-acceptor-store-mismatch` | `InternalIntegrityFailure` | `Never` |
| Acceptor declares another composition slot | `mutation-seal-acceptor-slot-mismatch` | `InternalIntegrityFailure` | `Never` |

`StoreError::reason_code` is already a `&'static str` chosen from a fixed set,
so these five codes carry no caller data by construction, and the only
non-constant thing any of them carries is `store_slot`, which section 2c bounds
at `MAX_STORE_SLOTS`. The split of the acceptor case is the operator-actionable
payload: the first two acceptor codes are exactly `SealIdentityMismatch::Zone`
and `SealIdentityMismatch::Store` from section 2a projected onto a stable
string, and they disclose only which of two values the operator supplied
disagreed, never either value.

The remediation each code names. The reported slot is where an operator starts
for the three that are composition faults, and is context rather than a
starting point for the two that are code defects:

- `mutation-seal-authority-mismatch`: a second `mutation_seal_pair` or a
  second `MutationSealIssuer::seal` call site exists. Find it; invariant 2
  names both counts and `policy_resource_mutation_seal.rs` enforces them.
- `mutation-seal-store-identity-mismatch`: defence in depth, unreachable
  through the composition root because a matching authority implies a matching
  pair. Treat it as a code defect in `d2b-resource-store`, not as a
  configuration error.
- `mutation-seal-acceptor-zone-mismatch`: the acceptor came from the
  authorizer of a different Zone. One `NativeAuthorizer` serves one Zone;
  check which authorizer the entry at the reported slot passed.
- `mutation-seal-acceptor-store-mismatch`: right Zone, wrong store instance.
  The `StoreSealIdentity` handed to `take_store_seal` and the one the opened
  database declares are different stores; check the entry at the reported slot
  for a store UUID or a database path taken from a neighbouring entry.
- `mutation-seal-acceptor-slot-mismatch`: identity agrees and the correlator
  does not, so the composition root built the identity for one entry and the
  acceptor for another. Check that the slot, the identity, and the database
  path in the loop body all come from the same iteration.

None of those five messages, and no other message either method produces, may
contain the store UUID, a rendered `SealAuthority`, the database path, or any
value derived from them; the bounded slot is the one identifier they carry.
`SealedMutation`, `MutationSealAcceptor`, and `StoreSealIdentity` cannot
be formatted at all, so a violation is a compile error rather than a review
catch.

**Mutation binding is containment, not correlation.** `SealedMutation` owns
the prepared mutations, the authorization, the policy snapshot, and the
operation context. `commit_verified` takes exactly one argument, so there is no
way to present evidence alongside a different mutation list. Nothing needs to
be matched, because nothing can be separated.

The measured behaviour of the shape above, with a hostile crate still declaring
`[lib] name = "d2b_resource_api"`:

```text
COMMITTED to A: ["create/Foo"]                          honest path
forged-pair   -> Err("mutation-seal-authority-mismatch")
cross-store   -> Err("mutation-seal-authority-mismatch")
mismatched-acceptor -> true                             refused at open_owned
```

That reproduction predates the reason-code split in the table above and so
reports the acceptor refusal as a single boolean. It is left as measured rather
than rewritten to match: the property it demonstrates is that `open_owned`
refuses, which the split does not change. A rerun under the split shape
distinguishes `mutation-seal-acceptor-zone-mismatch` from
`mutation-seal-acceptor-store-mismatch` from
`mutation-seal-acceptor-slot-mismatch`, and Wave A's three runtime negatives on
`open_owned` are what assert that, not this block.

### 5. No named trait or port over the write path

There is no `MutationPort` and there will not be one. The name is worth saying
because the parked W5 follow-up reached for exactly that shape, one abstraction
above the type parameter, and it is not a committed symbol anywhere. The write
surface is one inherent method on the concrete backend:

```rust
impl RedbResourceStore {
    pub async fn commit_verified(
        &self,
        sealed: SealedMutation,
    ) -> Result<StoreCommitResult, StoreError>;
}
```

`RedbResourceStore` loses its `V` type parameter, its `PhantomData<fn(V)>`,
`VerifiedMutationView`, `VerifiedPreparedMutationView`, and the `type_name`
check. A commit against a wired store cannot be presented by a holder of that
store, because the sole write method takes a type whose sole constructor is
`MutationSealIssuer::seal` and the only issuer paired with that store's
acceptor is the one `NativeAuthorizer` retains. The stronger phrasing, that an
unverified commit is unrepresentable, would be wrong: `seal` is public, so the
property is "no reachable issuer is this store's" and not "no issuer exists",
which is why invariant 2 counts `seal` call sites rather than trusting the
type.

`ResourceStoreBackend` stays a public trait in `d2b-resource-api`, and that is
correct. A foreign implementation of it is not a forgery path: its only
mutating method demands a `SealedMutation` the foreign crate cannot construct
for any real store, and registering a foreign backend requires the composition
root regardless. A future reviewer should not "fix" this by sealing it.

### 6. Seals

Compile-fail fixtures extend the existing harness at
`packages/d2b-resource-api/tests/external_seals.rs`, whose out-of-workspace
fixture crate gains dependencies on `d2b-resource-store` and
`d2b-resource-store-redb`. One harness, one scratch tree, one cold build:
adding a second harness in another crate would double a cost the existing one
already documents at 767 MB. The harness's rustc shim extends to force
`--cfg test` on `d2b_resource_store` and `d2b_resource_store_redb` as well as
`d2b_resource_api`, so measured bypass 2 cannot recur in any of the three
crates.

Each fixture and the diagnostic it must assert, measured on rustc 1.97.0
against a reduction of the shape in section 2:

| Fixture | Asserted diagnostic |
| --- | --- |
| `implement_mutation_view` | `error[E0432]` and `` no `VerifiedMutationView` in the root `` |
| `forge_sealed_mutation` | `error[E0451]` and `` of struct `SealedMutation` are private `` |
| `open_sealed_without_acceptor` | `error[E0616]` and `` field `body` of struct `SealedMutation` is private `` |
| `clone_sealed_mutation` | `error[E0599]` and `` no method named `clone` found for struct `SealedMutation` `` |
| `reuse_sealed_mutation` | `error[E0382]` and `` use of moved value: `sealed` `` |
| `clone_seal_acceptor` | `error[E0599]` and `` no method named `clone` found for struct `MutationSealAcceptor` `` |
| `name_seal_authority` | `error[E0603]` and `` module `authority` is private `` |
| `debug_format_sealed_mutation` | `error[E0277]` and `` `SealedMutation` doesn't implement `Debug` `` |
| `display_format_seal_identity` | `error[E0277]` and `` `StoreSealIdentity` doesn't implement `std::fmt::Display` `` |
| `compare_seal_identities` | `error[E0369]` and `` binary operation `==` cannot be applied to type `StoreSealIdentity` `` |

`forge_sealed_mutation` asserts a suffix rather than the whole line on purpose.
The measured text is ``fields `authority` and `body` of struct
`SealedMutation` are private``: rustc enumerates every private field in the
struct literal, so the field list changes when the struct gains one and an
assertion over the full sentence would fail on a change that strengthens the
seal rather than weakens it. The suffix is stable under that change and still
fails if any field becomes public, because a partly-public literal reports the
singular `is private` form instead.

`clone_sealed_mutation` must take its argument by value. By reference,
autoref resolves `.clone()` to `<&T as Clone>::clone`, which compiles and then
fails the return type, so the test asserts a misleading `error[E0308]` instead
of the absent-method error it means to prove. Measured both ways.

The three non-forgery fixtures are the compile-fail half of sections 2a and 2c.
They are worth their build cost because the census cannot see them: absence of
a trait impl is not a snapshot row, so nothing else in the harness would notice
a `#[derive(Debug)]` appearing on `SealedMutation`, or a `#[derive(PartialEq)]`
appearing on `StoreSealIdentity` and quietly folding the correlator into the
identity comparison, in a later wave. All three were measured directly rather
than inferred; the `Display` diagnostic in particular names `std::fmt::Display`
rather than `Display`, and `compare_seal_identities` names the owned type
`StoreSealIdentity` rather than `&StoreSealIdentity`, so an assertion on either
short form would not match.

API-surface census: `SealedMutation`, `MutationSealIssuer`,
`MutationSealAcceptor`, and `OpenedMutation` are added to `capability_roots` in
`tests/golden/api-surface/roots.json`, and the four snapshots under
`tests/golden/api-surface/` are regenerated with `make api-surface-pin`.

What that does to
`packages/d2b-bus/tests/approved-capability-trait-impls.txt` is worth stating
exactly, because it is the one part of this wave an implementer cannot guess
from the diff. `capability_fixed_point` in
`packages/d2b-api-surface/src/lib.rs` grows the closure by **referrer, not
referent**: an identity joins when its own definition references something
already in the set. So the four new roots contribute no rows of their own, per
section 2a they implement nothing, and `SealIdentityMismatch`,
`StoreSealIdentity`, `MutationSealBody`, and `StoreSlot` stay outside the
closure entirely, because their definitions reference nothing at all in the
mismatch enum's case and `u8`, `StoreSlot`, `ZoneId`, `ResourceUid`, and the
payload types in the others, and none of those is a capability. What joins are
the holders:
`NativeAuthorizer`, `RedbResourceStore`, `ResourceStoreBackend`, `RedbBackend`,
and `ResourceService`. Of those five, exactly one implements anything today -
`NativeAuthorizer` has a hand-written `Debug` writing
`NativeAuthorizer(<redacted>)` - so that is the one row B2 expects to approve,
and it is already redacted. The seal types themselves must never appear in that
file; if they do, section 2a has been broken.

Trait-solver ambiguity assertions go in
`d2b-resource-store` itself, extending the
`CapabilityMustNotImplementCloneCopyDefaultOrFrom` construction that
`packages/d2b-session/src/admission.rs` already carries for `SessionAcceptor`
with one further arm, `impl<T: core::fmt::Debug, B> ... for T {}`, and renaming
it `CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom`. `Clone`, `Copy`,
`Default`, `Debug`, and `From` for the four types are then rejected in every
compiled configuration. Measured on rustc 1.97.0: the extended construction
compiles unchanged while the types implement none of the five, and adding
`#[derive(Debug)]` to `SealedMutation` turns the assertion into
`error[E0283]: type annotations needed`, naming the blanket impl and the
`Debug` arm as the two candidates. `packages/d2b-bus/tests/public_mint_surface.rs`
is the census that reads those results, not the place the assertions live. Any
later public constructor, public field, or extra trait implementation reachable
from the capability closure then appears as a snapshot diff and fails
`make test-rust-api-surface`.

Four source-level properties a census cannot see are enforced by a new
tree-wide policy lint at
`packages/d2b-contract-tests/tests/policy_resource_mutation_seal.rs`, wired
explicitly into `tests/test-policy.sh`. The existing
`packages/d2b-resource-store/tests/d106_policy.rs` is precedent for the
assertions, but `tests/AGENTS.md` assigns new Rust scans of source and docs to
the contract-test policy layer:

- **Both mint symbols are counted, not just the pair.** `mutation_seal_pair(`
  occurs in exactly one non-test call site in the workspace, and
  `.seal(` on a `MutationSealIssuer` occurs in exactly one, both in
  `d2b-resource-api`. Counting only the pair would leave the mint unbounded:
  `MutationSealIssuer::seal` is `pub`, takes a public-field body, and is
  retained for the authorizer's lifetime, so a second `seal` call site is a
  write that no evaluation gated. This mirrors the committed
  `admission.record_allow(` count and exists for the same reason.
- **Slots have at most one assigner.** `StoreSlot::new(` occurs in no more than
  one non-test call site in the workspace. That count is zero at Wave B, because
  nothing wires a resource store into `d2bd` yet, and becomes one when the
  wiring wave lands; the bound is what matters, not the current value. Two
  assigners means two stores can claim one slot, and every diagnostic naming a
  slot becomes ambiguous without anything failing. The assertion is the only
  mechanical hold this record has on section 2c's assignment rule, since the
  determinism of the order itself is reviewable but not checkable from these
  three crates.
- **`mutation_seal.rs` contains no `cfg(test)` or `cfg(not(test))` token**, and
  it joins the existing `Role`/`RoleBinding` scan, whose `include_str!` set
  covers only `src/lib.rs` today and would otherwise leave the new file
  unscanned.
- **`mutation_seal.rs` contains none of `Debug`, `Display`, `Serialize`,
  `Deserialize`, `JsonSchema`, `PartialEq`, `Eq`, `as_str`, or
  `to_canonical_string`.** The first five are section 2a's rule as a text
  assertion, which catches a hand-written impl that the four-root ambiguity
  assertion does not cover because it applies only to the four capability roots
  and not to `MutationSealBody`, `StoreSealIdentity`, or `SealAuthority`.
  `PartialEq` and `Eq` are section 2c's rule: a derive on `StoreSealIdentity`
  would fold the correlator into the identity comparison silently, and the
  `Eq` token subsumes `PartialEq` as a substring, so both are listed for
  legibility rather than for coverage. The scan is case-sensitive, which is
  what keeps `Arc::ptr_eq` - the one comparison in the file that must stay -
  outside it. The last two catch the other way a UUID
  escapes: rendering it inside the module rather than implementing a formatter.
  The file needs none of them: the private comparison writes `==` on the two
  identity fields, which reaches `ZoneId`'s and `ResourceUid`'s own derived
  equality without naming the trait, and it never converts either to text.

The three counts require walking the workspace, not an `include_str!` list: a
closed list cannot see a call site added in a file nobody remembered to
include. The
walk anchors on `env!("CARGO_MANIFEST_DIR")`, skips `target/` and `.scratch/`,
and fails closed with a distinct message if the workspace root is not found,
because a scan that silently examines nothing reports success. The three
`mutation_seal.rs` scans are file-local and use `include_str!`, which is
correct for them: the file is named in the assertion, so it cannot go missing
without the test failing to compile.

Runtime negatives, all fail-closed on an exact reason string:

- `open_rejects_seal_from_a_foreign_pair`
- `open_rejects_seal_bound_to_another_store_identity`
- `open_owned_rejects_acceptor_bound_to_another_zone`, asserting
  `mutation-seal-acceptor-zone-mismatch`
- `open_owned_rejects_acceptor_bound_to_another_store_in_the_same_zone`,
  asserting `mutation-seal-acceptor-store-mismatch`, which is the case a
  single-Zone deployment actually hits
- `open_owned_rejects_acceptor_declaring_another_slot`, asserting
  `mutation-seal-acceptor-slot-mismatch` for an acceptor whose Zone and store
  UUID both agree, which is the only way the correlator could otherwise lie
- `diagnose_names_the_disagreeing_component_without_rendering_it`, asserting
  `Ok(())` on agreement, `Err(SealIdentityMismatch::Zone)` and
  `Err(SealIdentityMismatch::Store)` on the two disagreements, and the reason
  code each variant selects
- `errors_from_a_multi_store_startup_carry_distinct_slots`, opening two stores
  at slots 0 and 1, refusing both acceptors, and asserting the two `StoreError`
  values differ in `store_slot` and nowhere else; this is the case section 2c
  exists for and the one a single-store test cannot see
- `store_slot_rejects_an_index_at_the_composition_bound`, asserting
  `StoreSlotError` at `MAX_STORE_SLOTS`, mirroring the committed
  `mutation_ordinal_is_zero_based_and_bounded_by_batch_limit`
- `commit_rejects_seal_from_another_store`, end to end across two live redb
  databases
- `take_store_seal_rejects_a_second_call`, asserting
  `StoreSealHandoffError::AlreadyTaken` by pattern rather than by `unwrap_err`,
  since `MutationSealAcceptor` has no `Debug` for `unwrap_err` to use, and
  asserting that the bound `slot` is the one the call passed
- `service_new_rejects_an_authorizer_that_issued_no_seal`

### 7. Migration

**`d2b-resource-store`** gains `mutation_seal.rs` and receives
`PreparedStoreMutation`, moved down from `d2b-resource-api` because it is now
part of the sealed payload. It gains no dependency: its manifest lists
`d2b-contracts` and nothing else, and `ResourceUid` comes from there. Its
public surface grows by one module plus, in `error.rs`, `SealIdentityMismatch`,
`StoreSlot`, `StoreSlotError`, `MAX_STORE_SLOTS`, and two more `StoreError`
members. `StoreError` gains `store_slot: Option<StoreSlot>` with an accessor
and a `with_store_slot` constructor; `StoreError::new` and
`StoreError::batch_conflict` keep their signatures and set the field to `None`,
so no committed call site changes and the hand-written `Debug` gains one row.

That move is not purely mechanical, and the ADR names the cost rather than
leaving it for the implementer to discover. `PreparedStoreMutation` has three
private fields and is constructed today by `prepare_mutation` inside
`d2b-resource-api`. Once the type lives one crate down, that construction is
cross-crate, so `d2b-resource-store` must expose
`PreparedStoreMutation::new(mutation, resource_uid, payload_digest)`. This is
acceptable and it is not a mint: a prepared mutation is inert data, meaningful
only inside a `SealedMutation` that the holder cannot construct. It does move
four census identities from `d2b_resource_api::PreparedStoreMutation` to
`d2b_resource_store::PreparedStoreMutation` and adds a `::method:new` row, so
the snapshot re-pin is part of the same wave, not a later one.

**`d2b-resource-store-redb`** loses `VerifiedMutationView`,
`VerifiedPreparedMutationView`, the `V` parameter on `RedbResourceStore`, the
`PhantomData<fn(V)>` field, and the `type_name` guard.
`provision_owned` and `open_owned` gain a `MutationSealAcceptor` parameter and
refuse a mismatched one. `commit_verified` becomes non-generic.
`transaction::from_view` takes `&OpenedMutation` instead of a generic view, and
`WriterHandle::commit` loses its `V` parameter with it.
`StoreIdentity` gains a private `slot: StoreSlot` and a `slot()` accessor, and
gains `seal_identity()`, returning a `StoreSealIdentity` built from its `slot`,
`zone`, and `store_uuid`. This is a breaking change to a crate with
exactly one consumer.

`StoreIdentity::new` gains the slot as its first parameter, which is the one
public signature this migration widens. The slot has to arrive here rather than
at `open_owned` because a slot passed alongside an identity can disagree with
it; a slot carried inside the identity is chosen once per composition entry and
travels with everything derived from it. `provision_owned` and `open_owned`
compare the acceptor's slot against the store's and refuse a disagreement with
`mutation-seal-acceptor-slot-mismatch`, which is what closes the residual that
`StoreIdentity::new` and `StoreSealIdentity::new` both take the slot from their
caller.

`StoreIdentity`'s private `store_uuid` field changes from `String` to
`ResourceUid` so that `seal_identity()` is infallible. `StoreIdentity::new`
already takes `ResourceUid` and immediately downgrades it with
`store_uuid.as_str().to_owned()`; that downgrade is what this wave deletes, so
no public signature changes and there is no accessor to update, because the
type has never exposed its UUID. Two call sites follow:
`transaction::create_meta` writes `identity.store_uuid.as_str().to_owned()`,
and `transaction::validate_identity` compares
`meta.store_uuid != identity.store_uuid.as_str()`.

`StoreMeta::store_uuid` stays a `String`, and that boundary is deliberate. It
is the decoded durable record, so it must round-trip whatever is actually on
disk; parsing it into `ResourceUid` at decode time would turn a corrupt or
foreign store into a decode error rather than the existing
`store-identity-mismatch`, which is a worse diagnosis of the same fault. A
non-canonical durable value therefore still fails the identity comparison and
still denies, because the in-memory side is canonical by construction and the
comparison is byte equality. `StoreMeta` gains no slot field, for the reason
section 2c gives: a slot describes one composition and a durable record
carrying it would be stale the first time an operator reorders configuration.

**`d2b-resource-api`** deletes `VerifiedMutation` and its public re-export,
re-exports `PreparedStoreMutation` from `d2b-resource-store` for source
compatibility, changes `ResourceStoreBackend::commit_verified` to take
`SealedMutation`, and adds `NativeAuthorizer::take_store_seal` with its own
`StoreSealHandoffError`. `StoreBindingError` and `take_store_binding` are
unchanged. `StoreAdmissionBinding::verify` keeps both existing
pointer-identity checks and now returns a `MutationSealBody` that
`CheckedResourceStore::commit` seals immediately before the backend call.
`AdmissionIssuer`, `AdmissionPermit`, and `AdmittedMutation` are unchanged.

No wire contract, no persisted format, no schema, and no Nix surface changes.
D106's normative properties are strengthened rather than altered, and the
register already reserves that "private implementation type names may change
without changing this contract", so no amendment to
`docs/specs/ADR-046-decision-register.md` is required. One drift does follow
and must be recorded rather than glossed: the register's rationale column names
`VerifiedMutation` verbatim, and after Wave A that type does not exist. The
sentence's substance survives with `SealedMutation` substituted, because the
checked store still matches both authorities, still prepares final identities
and digests, and is still the only thing that can produce the value
`commit_verified` accepts. Wave A records the substitution in its Spec
corrections table under the canon rule.

## Rejected alternatives

**A redacted `Debug` on the seal types.** Section 2a. The precedent exists one
crate over, in `SessionAcceptor`, and it is still the wrong choice here: the
type being replaced has no `Debug`, no holder needs one, a constant string
carries no diagnosis, and keeping the impl converts a compile error into a
silent no-op that reads as permission.

**A `StoreUuid` newtype in `d2b-resource-store`.** Section 2b. It would
duplicate `ResourceUid`'s validation, add a second UUID type to a crate that
already names the first, and invite a `From` impl between them. Reusing
`ResourceUid` costs no dependency because `d2b-contracts` is already the
crate's only one.

**A boolean pair for the identity mismatch.** The first revision of section 2a
made `SealIdentityMismatch` a struct of `zone_matches` and
`store_uuid_matches`, with `is_match` and an optional reason code. Four
representable states for a two-state domain, one of which duplicates the `Ok`
arm and one of which describes a UUIDv4 collision across Zones. The enum plus
`Result<(), SealIdentityMismatch>` costs the same to construct, removes both
unreachable states, and makes the reason code total. Rejected on the same
ground the `type_name` guard was: a representation that admits states the
domain excludes is a representation someone eventually reads wrong.

**Reusing an existing identifier as the operator's correlator.** The store
UUID, the Zone label, the database path, and an operator-authored store name
were each considered and each rejected by section 2a's prohibition, which the
round that produced section 2c did not relax. The name is the closest call,
because it is the thing an operator actually recognises; it is rejected because
it is unbounded operator-authored text, which makes it a poor metric label, a
plausible carrier of tenant or host information, and a value the redaction rule
would have to carve an exception for. A bounded ordinal over configuration
order carries the same locating power for a startup diagnostic and needs no
exception.

**A per-store correlator the store generates for itself.** A random or
monotonic id minted at open would need no composition-root discipline, and it
is rejected because it is meaningless to the operator: nothing in their
configuration carries it, so a refusal naming it is exactly as unlocatable as
one naming nothing. A correlator only works if the party reading it already
holds the other half, which is why the assignment has to come from
configuration order and from nowhere else.

**A sealed trait in `d2b-resource-store-redb`.** The standard private-supertrait
seal requires the sealing crate to implement `Sealed` for each permitted type.
`d2b-resource-store-redb` cannot name `d2b_resource_api::VerifiedMutation`
without depending on `d2b-resource-api`, which D106 forbids and which would
close a cycle against the existing api-to-redb edge. Not available.

**Inverting the dependency so the backend names the concrete type.** This
works technically and is the shortest diff. It is rejected because D106's
dependency invariant is load-bearing for a different property: it is what keeps
`Role`, `RoleBinding`, and the evaluator out of the storage layer, asserted by
policy test. Trading a policy invariant for a type-system one when a third
option satisfies both is a bad trade.

**A doc-hidden or `#[cfg(test)]`-gated constructor.** `#[doc(hidden)]` is
public API with a discouraging label; the rustdoc census in
`tests/golden/api-surface/hidden-public-api.txt` exists precisely because
hidden items are still reachable. A `cfg`-gated seal is measured bypass 2.

**A backend-owned issuer.** Letting `RedbResourceStore` create the pair and
hand the issuer up puts the mint capability in the trusted-but-not-authorizing
layer. A backend could then retain an issuer, or hand the same one to two
stores, and neither is visible to the authorization path. The mint must sit on
the side that evaluates policy.

**A token identifier checked at runtime.** A `u64` or UUID compared inside
`commit_verified` is what the parked branch has, one abstraction up. It is
forgeable by anyone who can construct the carrier, invisible to the
compile-fail and census legs, and it degrades rather than denies when the
comparison is skipped. Rejected on the fail-closed rule.

**Cryptographically re-binding the envelope.** Sealing a digest of the
canonical payload and recomputing it in `open` protects against an attacker
who can mutate the value between mint and accept. Inside one process, holding
the value by move, no such attacker exists. It buys nothing and spends hash
time on the path carrying ADR 0046's normative p95 durable commit budget.

**A nonce ledger for single use.** See section 4: move semantics already
exclude the replay, and the ledger's state is unbounded.

**A process-wide seal root claimed once.** The obvious way to close the
residual in Consequences is to make the store crate own one root that only the
first caller can take: `claim_mutation_seal_root() -> Option<MutationSealRoot>`
returning `Some` exactly once per process, with per-store pairs derived from
it. A hostile crate linked into the same daemon would then get `None` and could
not construct a store at all. It is rejected on two counts. Process-global
one-shot state cannot be reset between tests in one binary, so it would need
exactly the `cfg(test)` reset hatch that invariant 5 forbids and that measured
bypass 2 is about. And a daemon serving more than one Zone legitimately opens
more than one store, so the singleton is wrong about the shape of the system,
not merely awkward to test.

**An `unsafe fn` mint, using the lint as the gate.** Making
`mutation_seal_pair` an `unsafe fn` would force every call site to declare
itself and would put the mint under a lint the repository already runs.
Measured unavailable: `packages/Cargo.toml` sets `unsafe_code = "forbid"` in
`[workspace.lints.rust]` and `d2b-resource-api` takes `[lints] workspace =
true`, and `forbid` cannot be relaxed by an inner `allow`, so the one
legitimate call site would not compile. Opting that crate out of the workspace
lint table to enable this would trade a repository-wide guarantee for a local
one. It is also an abuse of `unsafe`, which states a memory-safety contract,
not an authorization one.

**Branded lifetimes binding issuer to acceptor at the type level.** An
invariant-lifetime brand would turn cross-store presentation from a runtime
`Err` into a compile error, which is strictly stronger for that one case. It
requires the branded values to live inside the closure that generates the
brand. `RedbResourceStore` is held `'static` inside an `Arc` in a service that
outlives any closure, so adopting a brand means threading a lifetime parameter
through `NativeAuthorizer`, `ResourceService`, `CheckedResourceStore`, the
backend trait, and every future async boundary between them. The cross-store
case is already denied, never degraded; buying a compile error for it at that
price is the larger design that anticipates, over the smaller one that can be
extended.

## Invariants this decision creates

1. `SealedMutation` is constructed in exactly one place in the dependency
   graph: `MutationSealIssuer::seal`. Adding a second constructor, a `Clone`,
   a `Copy`, a `Default`, a `Debug`, a `From`, or a public field is a
   trust-boundary change requiring a stated reason.
2. Both mint symbols have exactly one non-test call site each, in
   `d2b-resource-api`: `mutation_seal_pair` and `MutationSealIssuer::seal`. A
   second `mutation_seal_pair` call site means a store exists whose writes the
   evaluator does not gate. A second `seal` call site means a commit exists
   that no evaluation produced, which the census cannot see because `seal` is
   already public.
3. `d2b-resource-store` and `d2b-resource-store-redb` never depend on
   `d2b-resource-api`. Unchanged from D106, now also required for the seal to
   remain coherent.
4. `d2b-resource-store-redb` exposes exactly one method that mutates, and it
   takes `SealedMutation` by value. No generic write path, no view trait, no
   second write entry point.
5. `mutation_seal.rs` contains no `cfg(test)` or `cfg(not(test))`.
6. The external-seals rustc shim forces `--cfg test` on all three crates. A
   future crate joining this boundary joins the shim.
7. No type declared in `mutation_seal.rs` implements `Debug`, `Display`,
   `Serialize`, `Deserialize`, `JsonSchema`, `PartialEq`, or `Eq`, and the file
   renders no identity to text. Adding a formatting or serialization impl is a
   disclosure-boundary change; adding a comparison impl is a correlator-boundary
   change, because a derived one on `StoreSealIdentity` would fold the store
   slot into the identity comparison. Either requires a stated reason, not a
   convenience.
8. The store UUID, the `SealAuthority` pointer value, and the host database
   path never appear in an error message, a log line, a metric label, a span
   attribute, or an audit record. What a caller may learn about a mismatch is
   the stable reason code, the `SealIdentityMismatch` variant, and the bounded
   store slot, and nothing else.
9. A store is identified by `ResourceUid`, not by `String`. Equality is byte
   equality over the single canonical form and is deliberately not
   constant-time, because it is the diagnosable check and never the
   authorization check.
10. A store is located, as opposed to identified, by `StoreSlot`: a zero-based
    index into the composition root's declared store list, bounded by
    `MAX_STORE_SLOTS`, assigned by a deterministic total order over the
    declared configuration and derived from no UUID, path, address, hash, or
    completion order. It is the only operator-facing identifier any seal
    diagnostic carries. It is never persisted, never serialized, never on the
    wire, and never compared for identity. Every startup diagnostic a
    composition root emits for a store carries it, on the success path as well
    as the failing ones.
11. `StoreSealIdentity` and `StoreIdentity` are constructed with a slot or not
    at all, and `open_owned` refuses an acceptor whose slot disagrees with the
    store's. Without that refusal the correlator can name a configuration entry
    the operator did not misconfigure, which is worse than naming none.

## Consequences

The backend is still part of the trusted computing base and this decision does
not change that. `OpenedMutation` hands the writer everything it needs, and a
buggy or hostile backend can still ignore it, skip the revision recheck, or
write through another path inside its own crate. D106 already states this; the
seal narrows who can reach the backend, not what the backend does once reached.

`d2b-resource-store` grows from a pure contract crate into a crate that owns
one capability. That is a real cost: it is now a security-relevant crate, and
its section in `docs/contributing/critical-subsystems.md` says so. The
alternative placements were worse.

**What this seal does not do, stated plainly.** It binds evidence to a store
instance. It does not bind a store instance to the production evaluator, and it
cannot, because `mutation_seal_pair` and `open_owned` are both public and must
be. A crate outside this workspace can pair its own issuer and acceptor, open
its own store, mint its own evidence, and commit records that no authorization
evaluation produced. That was already true under the `type_name` guard; the
only change is that it no longer requires squatting the `d2b_resource_api`
library name, so the seal makes that particular path easier, not harder. What
the seal takes away is the part that was actually load-bearing: the write path
of a correctly wired store is now bound by move semantics and `Arc` identity
instead of a string comparison, and no crate holding a real store handle can
present evidence for it. Anyone who can already construct the store owns the
file, the identity, and the durable marker, so this residual grants nothing
they did not have; it is stated because a future reader will otherwise read
"unforgeable" and believe more than is true.

The concrete failure this design makes possible inside the workspace, and the
guard that catches it: a later wave adds a second backend, most plausibly an
in-memory test store, which calls `mutation_seal_pair` itself to obtain an
acceptor. If that wiring ever reaches production registration, a store exists
whose acceptor is paired with an issuer the evaluator does not hold, and every
write to it is unauthorized but accepted. The near neighbour is cheaper still:
a second `MutationSealIssuer::seal` call site anywhere in `d2b-resource-api`
mints committable evidence over a hand-built `MutationSealBody` with no
evaluation at all, because `seal` is public, the body's fields are public, and
the authorizer retains the issuer for its whole life. Neither is visible to the
API-surface census, because neither changes a public surface. Invariant 2 and
the two counts in
`packages/d2b-contract-tests/tests/policy_resource_mutation_seal.rs` exist for
exactly these two cases and nothing else.

A third, quieter failure: `NativeAuthorizer::take_store_seal` is once-only, so
a caller that constructs two stores against one authorizer gets an error rather
than two stores sharing a mint. That error surfaces at composition time, in
code that runs once at daemon start, so it must not be swallowed into a
degraded start path. It is also the failure section 2c's slot is for: without a
correlator, a daemon with six configured stores reports that one of them
refused and does not say which, so the operator's only move is to bisect their
own configuration.

Downstream crates that today implement `VerifiedMutationView` do not exist;
the trait shipped on an unmerged branch. There is no compatibility window to
manage.

## Implementation handoff

Two waves. Wave A is not sliced: the signature change spans three crates and
any parallel split would need a prep commit carrying most of the work.

### Wave A: the seal

One scope, owning `packages/d2b-resource-store/src/**`,
`packages/d2b-resource-store-redb/src/**`, `packages/d2b-resource-api/src/**`,
`packages/d2b-resource-api/tests/external_seals.rs`,
`packages/d2b-resource-api/tests/ui/external-seals/tests/forge_issuer.rs`,
the three `Cargo.toml` files, the four census snapshots under
`tests/golden/api-surface/`, the four `packages/d2b-bus/tests/approved-*.txt`
snapshots, and the two documents named below.

The census snapshots are Wave A's, not Wave B's, and the reason is mechanical
rather than tidy. `tests/golden/api-surface/capability-api.txt` carries
fourteen lines naming `d2b_resource_api::VerifiedMutation` and
`d2b_resource_api::PreparedStoreMutation`, and
`packages/d2b-bus/tests/approved-capability-api.txt` carries four more.
Wave A deletes the first type and re-homes the second, so both files change the
moment Wave A compiles, and `run_api_surface_gate` plus
`public_mint_surface.rs` both run inside `make test-rust`. If Wave B owned
those files, Wave A's own stopping condition could not be met. Wave A therefore
re-pins the snapshots for the surface it deletes and re-homes, and nothing
else; Wave B adds the new capability roots and re-pins again.

Deliverable: sections 1 through 5 above, including 2a, 2b, and 2c, and section
7, plus the eleven runtime negatives in section 6.

The critical-subsystem documentation lands in this wave, not earlier. A row
describing `mutation_seal.rs` before that file exists would put
`AGENTS.md` and `docs/contributing/critical-subsystems.md` in drift against
committed code, which is the failure the canon rule forbids. Wave A therefore
adds one row to the `AGENTS.md` critical-subsystem table and one matching
section to `docs/contributing/critical-subsystems.md`, headed
"Resource mutation seal", pointing at
`packages/d2b-resource-store/src/mutation_seal.rs` and carrying invariants 1
through 11 above.

That row is the reason `make test-rust` is not a sufficient stopping condition
for this wave, and the gap is measured, not hypothetical. `test-rust` excludes
`d2b-contract-tests`, which is where `policy_docs.rs` enforces a 40,000-byte
ceiling on `AGENTS.md`; the file is 38,812 bytes at this branch tip, leaving
1,188 bytes of headroom that one critical-subsystem row can plausibly consume.
The same excluded crate holds `policy_dash_gate.rs` and `policy_lints.rs`,
which read `AGENTS.md` and `docs/adr/README.md`, and `policy_units.rs` and
`policy_docs.rs`, which the daemon-only rule already requires for any doc row
describing a control-plane surface. Wave A changes both documents and eleven
lines of `tests/golden/api-surface/`, so it must run the lane that covers them.

`make test-fixture-contracts` refuses to run unless `D2B_ENABLE_FIXTURE_BUILD`
is `1`: `tests/test-rust.sh` fails with "fixture-contracts mode requires
D2B_ENABLE_FIXTURE_BUILD=1; refusing to report a skipped gate as passing". The
Layer-1 orchestrator sets it from `tests/layer1-jobs.json`; a hand-run
stopping condition has to set it too, or it is not a stopping condition.

Done when all of the following hold:

```bash
cargo test -p d2b-resource-store -p d2b-resource-api -p d2b-resource-store-redb   # exit 0
rg -n 'VerifiedMutationView|VerifiedPreparedMutationView|MutationPort|type_name::<' \
   packages/d2b-resource-store packages/d2b-resource-store-redb \
   packages/d2b-resource-api                                                      # exit 1
rg -n 'd2b-resource-api' packages/d2b-resource-store/Cargo.toml \
   packages/d2b-resource-store-redb/Cargo.toml                                    # exit 1
rg -n 'cfg\(test\)|cfg\(not\(test\)\)' packages/d2b-resource-store/src/mutation_seal.rs  # exit 1
rg -n 'Debug|Display|Serialize|Deserialize|JsonSchema|PartialEq|Eq|as_str|to_canonical_string' \
   packages/d2b-resource-store/src/mutation_seal.rs                               # exit 1
rg -n 'VerifiedMutation' tests/golden/api-surface packages/d2b-bus/tests           # exit 1
rg -n 'StoreSlot' packages/d2b-contracts \
   packages/d2b-resource-store-redb/src/keys.rs \
   packages/d2b-resource-store-redb/src/values.rs \
   packages/d2b-resource-store-redb/src/schema.rs                               # exit 1
make api-surface-pin && git diff --exit-code tests/golden/api-surface/             # exit 0
make test-rust                                                                     # exit 0
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts                             # exit 0
```

The `StoreSlot` grep is invariant 10's "never serialized, never on the wire" as
a stopping condition. `d2b-contracts` is the crate that has a wire form, and
`keys.rs`, `values.rs`, and `schema.rs` are the whole durable key, value, and
table encoding, so naming the correlator in any of the four is the first step
of persisting a value that describes a composition and not a store.

### Wave B: the seals and the census

Two file-disjoint scopes, both opening against merged Wave A.

- **B1 compile-fail seals.** Owns `packages/d2b-resource-api/tests/**`.
  This ownership begins from merged Wave A, which has already updated
  `external_seals.rs` and `forge_issuer.rs` to remove their references to
  `VerifiedMutation`. B1 then extends the fixture crate manifest and lock,
  extends the rustc shim to the two additional crates, and adds the ten
  fixtures in section 6 with their exact asserted diagnostics.
  Done when `cargo test -p d2b-resource-api --test external_seals` exits 0 and
  the test asserts all ten.
- **B2 census roots and mint policy.** Owns
  `tests/golden/api-surface/roots.json`, the four snapshots under
  `tests/golden/api-surface/`,
  `packages/d2b-bus/tests/approved-capability-trait-impls.txt`,
  `packages/d2b-contract-tests/tests/policy_resource_mutation_seal.rs`,
  `tests/test-policy.sh`, and the "Capability mint surface allowlist" rows in
  `AGENTS.md` and
  `docs/contributing/critical-subsystems.md`, whose crate lists gain
  `packages/d2b-resource-store/`.
  Adds the four capability roots, regenerates the snapshots, adds the
  in-crate trait-solver ambiguity assertions including the `Debug` arm, and
  adds the three counts and the three `mutation_seal.rs` scans to
  `policy_resource_mutation_seal.rs`. It also moves Wave A's `StoreSlot` wire
  and durable serialization grep into that policy lint as a fail-closed
  workspace scan, and wires the lint into `test-policy`, so invariant 10
  remains enforced after the implementation branch closes. The approved
  trait-impl file gains only holder rows, per the closure analysis in section
  6.
  Done when `make api-surface-pin` followed by
  `git diff --exit-code tests/golden/api-surface/` exits 0,
  `make test-rust-api-surface` exits 0,
  `make test-policy` exits 0,

  ```bash
  rg -n 'SealedMutation|MutationSealIssuer|MutationSealAcceptor|OpenedMutation' \
     packages/d2b-bus/tests/approved-capability-trait-impls.txt   # exit 1
  D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts          # exit 0
  ```

  The first condition is section 2a as a grep: no seal type may have acquired a
  trait impl. The last is not decoration: B2 edits two `AGENTS.md` rows and
  re-pins the golden census, and the crate that gates both is excluded from
  `test-rust` for the same reason Wave A carries the same condition.

B1 and B2 are disjoint by path. They both open against Wave A, which has
already landed every shared contract they read, so no separate integrator prep
commit is needed.

Both waves land on the `adr046-w5` lineage. The parked uncommitted work in that
worktree is superseded by this decision and is not a starting point.
