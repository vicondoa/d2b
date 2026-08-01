# ADR 0049: Store-owned mutation seal for verified-only resource writes

- Status: Accepted
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
uncommitted. This decision is authored against committed code only.

## Decision

Move the mutation authorization capability **down** into
`packages/d2b-resource-store`, the crate both sides already depend on, and make
it a private concrete type rather than a trait. The verifier mints; the backend
consumes; nothing else can do either.

### 1. Which crate owns the capability

`d2b-resource-store` owns it, in a new module
`packages/d2b-resource-store/src/mutation_seal.rs`.

This is the only crate that both `d2b-resource-api` and
`d2b-resource-store-redb` already depend on. Placing the capability there needs
no new dependency edge, creates no cycle, and leaves D106's
"never depend on `d2b-resource-api`" invariant untouched. `d2b-resource-store`
already owns every payload type the evidence carries
(`StoreMutation`, `AdmittedAuthorization`, `PolicySnapshot`,
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
#[derive(Clone, PartialEq, Eq)]
pub struct StoreSealIdentity {
    pub zone: ZoneId,
    pub store_uid: ResourceUid,
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

`mutation_seal_pair` is public and that is deliberate. A downstream crate can
call it, obtain an issuer, and mint a `SealedMutation`. That value is inert:
`open` accepts only evidence carrying the same `Arc<SealAuthority>` the
acceptor holds, and no downstream can obtain the production acceptor's
authority. This is the same posture `d2b-session` takes with `SessionAcceptor`:
the security comes from instance identity and consumption, not from the absence
of a constructor. What makes it governed rather than merely tolerated is
section 6.

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

`ResourceService::new` fails closed with `StoreBindingError` when the
authorizer it is handed never issued a seal, so a service cannot exist holding
an issuer that no store will ever accept, and a store cannot exist holding an
acceptor no service will ever mint against.

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

No type in that chain is `Clone` or `Copy`.

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

### 5. `MutationPort` is not exposed

There is no `MutationPort`, and no other named trait over the write path. The
write surface is one inherent method on the concrete backend:

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
check. An unverified commit is unrepresentable because the sole write method
takes a type whose sole constructor is `MutationSealIssuer::seal`, and issuers
reachable to a caller are never the store's own.

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

Each fixture and the diagnostic it must assert, all measured:

| Fixture | Asserted diagnostic |
| --- | --- |
| `implement_mutation_view` | `error[E0432]` and `` no `VerifiedMutationView` in the root `` |
| `forge_sealed_mutation` | `error[E0451]` and `` fields ... of struct `SealedMutation` are private `` |
| `open_sealed_without_acceptor` | `error[E0616]` and `` field `body` of struct `SealedMutation` is private `` |
| `clone_sealed_mutation` | `error[E0599]` and `` no method named `clone` found for struct `SealedMutation` `` |
| `reuse_sealed_mutation` | `error[E0382]` and `` use of moved value: `sealed` `` |
| `clone_seal_acceptor` | `error[E0599]` and `` no method named `clone` found for struct `MutationSealAcceptor` `` |
| `name_seal_authority` | `error[E0603]` and `` module `authority` is private `` |

`clone_sealed_mutation` must take its argument by value. By reference,
autoref resolves `.clone()` to `<&T as Clone>::clone` and the test asserts a
misleading `error[E0308]` instead of the absent-method error it means to prove.

API-surface census: `SealedMutation`, `MutationSealIssuer`, and
`MutationSealAcceptor` are added to `capability_roots` in
`tests/golden/api-surface/roots.json`, the four snapshots are regenerated with
`make api-surface-pin`, and the approved list at
`packages/d2b-bus/tests/approved-capability-trait-impls.txt` gains their `Debug`
implementations and nothing else. Trait-solver ambiguity assertions in
`d2b-resource-store` mirror `packages/d2b-bus/tests/public_mint_surface.rs`, so
`Clone`, `Copy`, `Default`, and `From` for the three types are rejected in
every compiled configuration. Any later public constructor, public field, or
extra trait implementation reachable from the capability closure then appears
as a snapshot diff and fails `make test-rust-api-surface`.

One source-level policy test, `packages/d2b-resource-store/tests/mint_call_sites.rs`,
enforces two properties a census cannot see: `mutation_seal_pair(` occurs in
exactly one non-test call site in the workspace, inside
`packages/d2b-resource-api/src/authz.rs`; and `mutation_seal.rs` contains no
`cfg(test)` or `cfg(not(test))` token at all.

Runtime negatives, all fail-closed on an exact reason string:

- `open_rejects_seal_from_a_foreign_pair`
- `open_rejects_seal_bound_to_another_store_identity`
- `open_owned_rejects_acceptor_bound_to_another_store`
- `commit_rejects_seal_from_another_store`, end to end across two live redb
  databases, replacing the parked `mutation_port_rejects_submission_to_another_store`
- `service_new_rejects_an_authorizer_that_issued_no_seal`

### 7. Migration

**`d2b-resource-store`** gains `mutation_seal.rs` and receives
`PreparedStoreMutation`, moved down from `d2b-resource-api` because it is now
part of the sealed payload. It gains no dependency. Its public surface grows by
one module.

**`d2b-resource-store-redb`** loses `VerifiedMutationView`,
`VerifiedPreparedMutationView`, the `V` parameter on `RedbResourceStore`, the
`PhantomData<fn(V)>` field, and the `type_name` guard.
`provision_owned` and `open_owned` gain a `MutationSealAcceptor` parameter and
refuse a mismatched one. `commit_verified` becomes non-generic.
`transaction::from_view` takes `&OpenedMutation` instead of a generic view.
This is a breaking change to a crate with exactly one consumer.

**`d2b-resource-api`** deletes `VerifiedMutation` and its public re-export,
re-exports `PreparedStoreMutation` from `d2b-resource-store` for source
compatibility, changes `ResourceStoreBackend::commit_verified` to take
`SealedMutation`, and adds `NativeAuthorizer::take_store_seal`.
`StoreAdmissionBinding::verify` keeps both existing pointer-identity checks and
now returns a `MutationSealBody` that `CheckedResourceStore::commit` seals
immediately before the backend call. `AdmissionIssuer`, `AdmissionPermit`, and
`AdmittedMutation` are unchanged.

No wire contract, no persisted format, no schema, and no Nix surface changes.
D106's register text remains accurate as written: it already reserves that
"private implementation type names may change without changing this contract",
and the properties it asserts are strengthened rather than altered. No
amendment to `docs/specs/ADR-046-decision-register.md` is required.

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

## Invariants this decision creates

1. `SealedMutation` is constructed in exactly one place in the dependency
   graph: `MutationSealIssuer::seal`. Adding a second constructor, a `Clone`,
   a `Copy`, a `Default`, a `From`, or a public field is a trust-boundary
   change requiring a stated reason.
2. `mutation_seal_pair` has exactly one non-test call site, in
   `d2b-resource-api`. A second production call site means a store exists whose
   writes the evaluator does not gate.
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

The concrete failure this design still makes possible, and the guard that
catches it: a later wave adds a second backend, most plausibly an in-memory
test store, which calls `mutation_seal_pair` itself to obtain an acceptor. If
that wiring ever reaches production registration, a store exists whose acceptor
is paired with an issuer the evaluator does not hold, and every write to it is
unauthorized but accepted. The API-surface census cannot see this, because no
public surface changes. Invariant 2 and
`packages/d2b-resource-store/tests/mint_call_sites.rs` exist for exactly this
case and nothing else.

A second, quieter failure: `NativeAuthorizer::take_store_seal` is once-only, so
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
the three `Cargo.toml` files, and the two documents named below.

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
make test-rust                                                                    # exit 0
```

### Wave B: the seals and the census

Two file-disjoint scopes, both opening against merged Wave A.

- **B1 compile-fail seals.** Owns `packages/d2b-resource-api/tests/**`.
  Extends the fixture crate manifest and lock, extends the rustc shim to the
  two additional crates, and adds the seven fixtures in section 6 with their
  exact asserted diagnostics.
  Done when `cargo test -p d2b-resource-api --test external_seals` exits 0 and
  the test asserts all seven.
- **B2 census and mint policy.** Owns `tests/golden/api-surface/**`,
  `packages/d2b-bus/tests/approved-capability-trait-impls.txt`,
  `packages/d2b-resource-store/tests/**`, and the "Capability mint surface
  allowlist" rows in `AGENTS.md` and
  `docs/contributing/critical-subsystems.md`, whose crate lists gain
  `packages/d2b-resource-store/`.
  Adds the three capability roots, regenerates the snapshots, adds the approved
  `Debug` rows, and adds `mint_call_sites.rs`.
  Done when `make api-surface-pin` followed by
  `git diff --exit-code tests/golden/api-surface/` exits 0, and
  `make test-rust-api-surface` exits 0.

Both waves land on the `adr046-w5` lineage. The parked uncommitted work in that
worktree is superseded by this decision and is not a starting point.
