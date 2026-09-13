### Removed

- The persistent resource store is deleted, not deprecated: the
  `d2b-resource-store` and `d2b-resource-store-redb` crates (roughly 28k
  lines: actor, transaction, revision log, backup, ownership, value/key
  codecs, schema, audit, metrics, tracing), the redb-backed
  `RedbRegisteredControllerApi` (`packages/d2b-resource-api/src/registered.rs`)
  and the legacy watch half of `d2b-resource-api/src/watch.rs`, and the #507
  wrong-plane fence together with the plane it fenced (`LegacyPlaneFence` and
  every refusal helper in `d2b-resource-api/src/store.rs`). The manager is now
  the only execution model, so a wrong-plane read is no longer possible and the
  fence has nothing left to refuse. The store DTOs that outlived the crates -
  `StoreSlot`, `AdmittedAuthorization`, `PolicySnapshot`, `StoreOperationContext`,
  the mutation-seal and store-error types - moved to
  `packages/d2b-contracts-resource/src/v3/operations/{mod,error,seal}.rs` and are
  still re-exported at `d2b_contracts_resource::v3::*`, so a consumer of the
  types needs no change.
- The control-model machinery that existed only to route controllers through
  that store is deleted: `d2b-controller-toolkit`'s `Runner`,
  `ControllerSource`, `PendingQueue` and its queue machinery, the
  `ReconcileResult`/`MutationIntent` protocol, the reconcile-pass context half,
  and the crate's benches and integration tests (deleted rather than ported -
  nothing surviving drove them); and `d2b-core-controller`'s store-routing
  module class (`runtime.rs` with `CoreControllerSource`, the
  `configuration/{mod,bundle_apply,generation_transition}` bundle/cleanup/generation
  code, `cleanup.rs`, `audit.rs`, `authz*.rs`, `watches.rs`, `store.rs`,
  `resource_store.rs`, `export_import*.rs`, `metrics.rs`, `tracing.rs`,
  `provider_effects.rs`, `dependencies.rs`, `hints.rs`, `optional_state_admission.rs`,
  `ownership.rs`, `budgets.rs`, `quota.rs`, `emergency_policy.rs`,
  `user_session_authority.rs`) with their tests. `d2b-core-controller` keeps the
  domain modules the converted drivers and the daemon import (authority,
  controller assignment, controllers, coordinator, owner reconciliation,
  providers, rbac, zone links/status), and `d2b-provider-test-controller` - the
  signed acceptance fixture the activation lane drives - is kept: it needs only
  the two assignment constants, not the store routing.
- `d2b-bus` loses its store-era fixtures: `src/production_rss.rs`, the
  `production-rss-fixture` feature and its `[[test]]` entry, and
  `tests/production_watch_rss.rs` with the store-backed watch test and helpers
  in `src/router.rs`. The ResourceV3 watch contracts are exercised through the
  manager-served paths that survive.
- The daemon's durable status, authority and controller-checkpoint helpers are
  gone: `d2bd-runtime`'s store reads/provisioning (zone row readers, bundle
  materialization and validation, status persistence, controller-session
  evidence persistence, store identity resolution), the `Durable` arm of the
  guest store backend, and the whole `RedbAuthorityPersistence` adapter. The
  broker's zone-store file handover is not built anymore, so the daemon no
  longer opens a database file at startup.
- Every workspace manifest, `BUILD.bazel`, `Cargo.lock`, `Cargo.guest.lock` and
  the generated package-policy inputs drop the deleted crates and their
  dependency edges; the register of packages under `packages/policy-inputs`
  shrank from 357 to 354 packages (the two store crates and `redb`).

### Changed

- Zone authority operations - including the generation-publication barrier that
  proves one complete local generation set before the resource plane serves
  reads - are now owned by a process-local ledger
  (`d2bd_runtime::authority_persistence::ZoneAuthorityLedger`) instead of a
  durable store. This is a deliberate reduction of durability, not a
  re-homing: the admission barrier still serializes concurrent operations
  inside one daemon lifetime and still fences a conflicting generation, but a
  restart begins with an empty ledger. What re-establishes the state across a
  restart is re-derivation, not replay: the daemon's composition path recomputes
  the complete generation set from the Zone bundle and the authority identity
  and re-prepares/re-commits the publication marker on every boot, while the
  resources themselves are recovered by the drivers' probe/adopt path. The
  activation host-integration fixture proves exactly that for the controller
  half - its restart stages assert the external provider controller keeps its
  PID and its Process row keeps uuid and generation across
  `systemctl restart d2bd.service`, and that the row is still `Ready` with
  `observedGeneration == generation` after the resync window.
- Host-global claims (GPU and external NIC) recorded in
  `packages/d2bd-runtime/src/authority_persistence.rs` become process-local:
  in-flight claims are no longer crash-recovered from a durable row, so a daemon
  crash releases a claim that the durable adapter used to keep reserved until the
  owner released it. Owners are re-derived on the next boot by the same
  probe/adopt recovery path, and a claim that cannot be re-proven is refused
  rather than assumed. This is the deliberate controller-checkpoint reduction
  U14 was planned to make; it is called out here so a reviewer can weigh it
  instead of discovering it as a regression.
- The interaction runtime's audio admission no longer re-checks a dependency
  against a persisted status fence: `validate_audio_assignment` is gone with the
  persisted status it read, and `fresh_audio_dependency`
  (`packages/d2bd/src/resource_runtime/interaction_effects.rs`) now validates the
  authoritative row itself - identity, ownership and Zone - read from the
  manager. An assignment is admitted from the manager row and the committed
  provider configuration, the same authorities the rest of the interaction
  family reads; the consequence is that a stale persisted status can no longer
  veto an assignment the manager considers current - and equally, no persisted
  status can *authorize* one any more. The guest-side effect for these rows
  remains intentionally unimplemented (U13 note).
- Guest-local serving is memory-backed and target-local only. `GuestResourceStore`
  (`packages/d2bd-runtime/src/guest_resource_runtime.rs`) keeps
  controller-created Process-family rows in process for the lifetime of the
  admitted parent-Zone ComponentSession, with no database file: the guest runtime
  constructors no longer take a state directory, and there is no per-target
  database to reopen. Zone-authority types are refused at the boundary
  (`AuthorizationDenied` for their schema reads and watches) rather than
  pretended to be served, a watch answers with a receipt bound to the in-memory
  store, and a target-local restart rebuilds those rows from the host - the only
  binding domain of record. Guest-side effects for those rows remain
  intentionally unimplemented (U13 note), unchanged by this cut.
