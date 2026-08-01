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
    #[derive(Debug)]
    pub struct SealAuthority;
}

/// Declared identity of exactly one provisioned store.
///
/// Mirrors what `StoreIdentity` actually holds. `store_uuid` is the
/// discriminator: two stores in one Zone share `zone` and `zone_uid` and
/// differ only here, so keying on `zone_uid` would make the declared check a
/// no-op for the most plausible multi-store case.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreSealIdentity {
    pub zone: ZoneId,
    pub store_uuid: String,
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
    pub fn binds(&self, store: &StoreSealIdentity) -> bool;
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
    ) -> Result<MutationSealAcceptor, StoreBindingError>;
}
```

Composition order becomes:

```rust
let authorizer = Arc::new(NativeAuthorizer::new(/* ... */));
let acceptor = authorizer.take_store_seal(identity.seal_identity())?;
let backend = RedbResourceStore::open_owned(file, identity, acceptor).await?;
let service = ResourceService::new(Arc::new(RedbBackend::new(backend)), authorizer)?;
```

`take_store_seal` mirrors the committed `take_store_binding`, which
`ResourceService::new` already calls behind a `Mutex<Option<_>>` and which
already returns `StoreBindingError`. The new handoff is a second entry in the
same pattern, not a new one.

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
it then compares the declared `StoreSealIdentity` for equality, which is the
diagnosable one. `open_owned` additionally refuses an acceptor whose declared
identity does not match the store it is being installed into. Three
fail-closed points, each with a stable reason string:

| Condition | Reason | Kind | Retry |
| --- | --- | --- | --- |
| Evidence from a foreign issuer pair | `mutation-seal-authority-mismatch` | `InternalIntegrityFailure` | `Never` |
| Evidence declaring another store | `mutation-seal-store-identity-mismatch` | `InternalIntegrityFailure` | `Never` |
| Acceptor installed into the wrong store | `mutation-seal-acceptor-store-mismatch` | `InternalIntegrityFailure` | `Never` |

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

API-surface census: `SealedMutation`, `MutationSealIssuer`,
`MutationSealAcceptor`, and `OpenedMutation` are added to `capability_roots` in
`tests/golden/api-surface/roots.json`, the four snapshots under
`tests/golden/api-surface/` are regenerated with `make api-surface-pin`, and
the approved list at
`packages/d2b-bus/tests/approved-capability-trait-impls.txt` gains their `Debug`
implementations and nothing else. Trait-solver ambiguity assertions go in
`d2b-resource-store` itself, mirroring the
`CapabilityMustNotImplementCloneCopyDefaultOrFrom` construction that
`packages/d2b-session/src/admission.rs` already carries for `SessionAcceptor`,
so `Clone`, `Copy`, `Default`, and `From` for the four types are rejected in
every compiled configuration. `packages/d2b-bus/tests/public_mint_surface.rs`
is the census that reads those results, not the place the assertions live. Any
later public constructor, public field, or extra trait implementation reachable
from the capability closure then appears as a snapshot diff and fails
`make test-rust-api-surface`.

Two source-level properties a census cannot see are enforced by extending
`packages/d2b-resource-store/tests/d106_policy.rs`, which already owns exactly
this kind of assertion for the layer above (`admission.record_allow(` occurs
once; `pub fn admission_pair` never appears):

- **Both mint symbols are counted, not just the pair.** `mutation_seal_pair(`
  occurs in exactly one non-test call site in the workspace, and
  `.seal(` on a `MutationSealIssuer` occurs in exactly one, both in
  `d2b-resource-api`. Counting only the pair would leave the mint unbounded:
  `MutationSealIssuer::seal` is `pub`, takes a public-field body, and is
  retained for the authorizer's lifetime, so a second `seal` call site is a
  write that no evaluation gated. This mirrors the committed
  `admission.record_allow(` count and exists for the same reason.
- **`mutation_seal.rs` contains no `cfg(test)` or `cfg(not(test))` token**, and
  it joins the existing `Role`/`RoleBinding` scan, whose `include_str!` set
  covers only `src/lib.rs` today and would otherwise leave the new file
  unscanned.

Both counts require walking the workspace, not an `include_str!` list: a closed
list cannot see a call site added in a file nobody remembered to include. The
walk anchors on `env!("CARGO_MANIFEST_DIR")`, skips `target/` and `.scratch/`,
and fails closed with a distinct message if the workspace root is not found,
because a scan that silently examines nothing reports success.

Runtime negatives, all fail-closed on an exact reason string:

- `open_rejects_seal_from_a_foreign_pair`
- `open_rejects_seal_bound_to_another_store_identity`
- `open_owned_rejects_acceptor_bound_to_another_store`
- `commit_rejects_seal_from_another_store`, end to end across two live redb
  databases
- `service_new_rejects_an_authorizer_that_issued_no_seal`

### 7. Migration

**`d2b-resource-store`** gains `mutation_seal.rs` and receives
`PreparedStoreMutation`, moved down from `d2b-resource-api` because it is now
part of the sealed payload. It gains no dependency. Its public surface grows by
one module.

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
`StoreIdentity` gains `seal_identity()`, returning a `StoreSealIdentity` built
from its `zone` and `store_uuid`. This is a breaking change to a crate with
exactly one consumer.

**`d2b-resource-api`** deletes `VerifiedMutation` and its public re-export,
re-exports `PreparedStoreMutation` from `d2b-resource-store` for source
compatibility, changes `ResourceStoreBackend::commit_verified` to take
`SealedMutation`, and adds `NativeAuthorizer::take_store_seal`.
`StoreAdmissionBinding::verify` keeps both existing pointer-identity checks and
now returns a `MutationSealBody` that `CheckedResourceStore::commit` seals
immediately before the backend call. `AdmissionIssuer`, `AdmissionPermit`, and
`AdmittedMutation` are unchanged.

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
   a `Copy`, a `Default`, a `From`, or a public field is a trust-boundary
   change requiring a stated reason.
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
the two counts in `packages/d2b-resource-store/tests/d106_policy.rs` exist for
exactly these two cases and nothing else.

A third, quieter failure: `NativeAuthorizer::take_store_seal` is once-only, so
a caller that constructs two stores against one authorizer gets an error rather
than two stores sharing a mint. That error surfaces at composition time, in
code that runs once at daemon start, so it must not be swallowed into a
degraded start path.

Downstream crates that today implement `VerifiedMutationView` do not exist;
the trait shipped on an unmerged branch. There is no compatibility window to
manage.

## Implementation handoff

Two waves. Wave A is not sliced: the signature change spans three crates and
any parallel split would need a prep commit carrying most of the work.

### Wave A: the seal

One scope, owning `packages/d2b-resource-store/src/**`,
`packages/d2b-resource-store-redb/src/**`, `packages/d2b-resource-api/src/**`,
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

Deliverable: sections 1 through 5 and 7 above, plus the five runtime negatives
in section 6.

The critical-subsystem documentation lands in this wave, not earlier. A row
describing `mutation_seal.rs` before that file exists would put
`AGENTS.md` and `docs/contributing/critical-subsystems.md` in drift against
committed code, which is the failure the canon rule forbids. Wave A therefore
adds one row to the `AGENTS.md` critical-subsystem table and one matching
section to `docs/contributing/critical-subsystems.md`, headed
"Resource mutation seal", pointing at
`packages/d2b-resource-store/src/mutation_seal.rs` and carrying invariants 1
through 6 above.

Done when all of the following hold:

```bash
cargo test -p d2b-resource-store -p d2b-resource-api -p d2b-resource-store-redb   # exit 0
rg -n 'VerifiedMutationView|VerifiedPreparedMutationView|MutationPort|type_name::<' \
   packages/d2b-resource-store packages/d2b-resource-store-redb \
   packages/d2b-resource-api                                                      # exit 1
rg -n 'd2b-resource-api' packages/d2b-resource-store/Cargo.toml \
   packages/d2b-resource-store-redb/Cargo.toml                                    # exit 1
rg -n 'cfg\(test\)|cfg\(not\(test\)\)' packages/d2b-resource-store/src/mutation_seal.rs  # exit 1
rg -n 'VerifiedMutation' tests/golden/api-surface packages/d2b-bus/tests           # exit 1
make api-surface-pin && git diff --exit-code tests/golden/api-surface/             # exit 0
make test-rust                                                                     # exit 0
```

### Wave B: the seals and the census

Two file-disjoint scopes, both opening against merged Wave A.

- **B1 compile-fail seals.** Owns `packages/d2b-resource-api/tests/**`.
  Extends the fixture crate manifest and lock, extends the rustc shim to the
  two additional crates, and adds the seven fixtures in section 6 with their
  exact asserted diagnostics. Measured safe to own: no file under that path
  names `VerifiedMutation` or `PreparedStoreMutation` at `dc145025`, so Wave A
  does not touch it.
  Done when `cargo test -p d2b-resource-api --test external_seals` exits 0 and
  the test asserts all seven.
- **B2 census roots and mint policy.** Owns
  `tests/golden/api-surface/roots.json`, the four snapshots under
  `tests/golden/api-surface/`,
  `packages/d2b-bus/tests/approved-capability-trait-impls.txt`,
  `packages/d2b-resource-store/tests/d106_policy.rs`, and the "Capability mint
  surface allowlist" rows in `AGENTS.md` and
  `docs/contributing/critical-subsystems.md`, whose crate lists gain
  `packages/d2b-resource-store/`.
  Adds the four capability roots, regenerates the snapshots, adds the approved
  `Debug` rows, adds the in-crate trait-solver ambiguity assertions, and adds
  the two mint counts and the `mutation_seal.rs` scans to `d106_policy.rs`.
  Done when `make api-surface-pin` followed by
  `git diff --exit-code tests/golden/api-surface/` exits 0,
  `make test-rust-api-surface` exits 0, and
  `cargo test -p d2b-resource-store --test d106_policy` exits 0.

B1 and B2 are disjoint by path. They both open against Wave A, which has
already landed every shared contract they read, so no separate integrator prep
commit is needed.

Both waves land on the `adr046-w5` lineage. The parked uncommitted work in that
worktree is superseded by this decision and is not a starting point.
