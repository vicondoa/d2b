# ADR 0046 redb Zone resource store

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resource-store-redb` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | Zone runtime, `d2b-resource-store-redb` |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model` |
| Supersedes | None |

## Ownership and process boundary

Every Zone runtime embeds exactly one redb database and one resource service.
The database belongs to that Zone only.

The generated storage contract gives one broker/Host/Guest-local storage owner
authority to create/validate:

- database inode;
- identity marker;
- owner/group/mode/link count;
- filesystem/locking support;
- parent-directory fsync;
- replacement/missing-state detection.

The owner passes one already-open regular `File` to the Zone runtime. The
pinned redb API must use `FileBackend::new(File)` or an equivalently reviewed
fd-backed API. The Zone runtime does not resolve a caller-controlled store
path.

Providers/controllers never receive a redb handle, database file/dir fd, path,
table access, or direct store client. Only the resource service/store actor
touches redb.

## Store identity

One closed `store_meta` table binds:

| Key | Value |
| --- | --- |
| `store_uuid` | Random immutable store identity |
| `zone_name` | Must match `Zone/<name>` self resource |
| `zone_uid` | Immutable UID of self resource |
| `created_at` | RFC 3339 UTC |
| `schema_version` | Internal physical schema version |
| `current_revision` | Latest committed Zone revision |
| `compaction_floor` | Earliest replayable revision |
| `active_configuration_revision` | Active Nix/root configuration |
| `policy_revision` | Current authorization policy revision |
| `api_catalog_revision` | Current bound ResourceType catalog |
| `clean_shutdown` | Clean/crash-open marker |
| `backup_generation` | Latest validated logical backup |

A previously provisioned database that is missing, replaced, bound to another
Zone/UID, newer than the binary schema, or internally inconsistent fails closed.
The runtime never silently creates an empty replacement.

## Physical tables

The physical schema contains exactly ten redb tables. The key-space byte and
value-kind number are part of the on-disk D099 contract:

| Table | `d2bkey/v1` key-space | Components after the discriminant | `d2bval/v1` value kind | Value |
| --- | --- | --- | --- | --- |
| `store_meta` | `0x01` | `(metadata_key:text)` | `0x0001` | Closed versioned scalar |
| `api_schemas` | `0x02` | `(schema_digest:text)` | `0x0002` | Signed ResourceTypeSchema/ResourceApiExport, validator fingerprint, compatibility/provenance |
| `resources` | `0x03` | `(bound_resource_type:text, resource_name:text)` | `0x0003` | Complete strict ResourceEnvelope plus internal owner UID |
| `type_index` | `0x04` | `(bound_resource_type:text, resource_name:text)` | `0x0004` | Immutable resource UID |
| `owner_index` | `0x05` | `(owner_uid:text, child_uid:text)` | `0x0005` | Child ResourceType/name and latest revision |
| `producer_index` | `0x06` | `(producer_uid:text, endpoint_uid:text)` | `0x0006` | Endpoint producerRef reverse index (D092): resolves the `Endpoint` resources a `Process`/`Device`/`Guest`/`Host`/qualified producer realizes |
| `controller_index` | `0x07` | `(controller_binding_id:text, resource_type:text, resource_name:text)` | `0x0007` | Resource UID |
| `revision_log` | `0x08` | `(revision:u64)` | `0x0008` | Ordered bounded ChangeBatch |
| `operations` | `0x09` | `(operation_id:text)` | `0x0009` | Idempotency/request digest, resources, phase/outcome, accepted/finished revisions, retention |
| `zone_link_cursors` | `0x0a` | `(peer_zone_uid:text)` | `0x000a` | Link epoch and last sent/acked/received/applied revisions |

Keys use versioned length-prefixed binary tuples, never delimiter-joined
caller strings. Dynamic spec/status is canonical JSON validated against the
exact signed schema before storage. Envelope/index/operation/change values use
one versioned deterministic encoding owned by d2b-contracts.

Key-space and value-kind assignments are contiguous, permanent, and
table-specific. They are never reused or renumbered. Removing a table reserves
both numbers forever; adding a table allocates the next unused numbers.
Changing an assignment requires a physical `schema_version` bump and the staged
migration in `ADR046-store-003`.

`ResourceExport` and `ResourceImport` rows are stored and indexed like any other
bound ResourceType through `resources`, `type_index`, `owner_index`, and
`controller_index`. The local typed projection that core creates for a
`ResourceImport` is a normal local resource with
`metadata.ownerRef: ResourceImport/<name>`, so it appears in the same indexes as
any Provider-created resource. No cross-Zone rows are stored in a Zone database;
imports carry only their local `zoneLinkRef` plus bounded `exportKey`, and
ZoneLink cursor state remains the only cross-Zone store state.

Spec has the frozen three-layer shape (D089): the universal Resource
envelope/metadata, the ResourceType base spec at top-level `spec.*` (including
`spec.providerRef`), and an optional canonical `spec.provider =
{ schemaId, schemaVersion, settings }` extension. A spec write is validated
against the ResourceType base schema and, when `spec.provider` is present,
against the installed Provider's registered, signed, digested extension schema
(`schemaId`/`schemaVersion`) with strict unknown-field denial and spec bounds;
a mismatch fails closed with `spec-provider-schema-invalid`, a `settings` that
restates a base field with `spec-provider-shadow`, before any redb mutation.

Status is the default durable observation surface for bounded non-secret
operational state (D087) and is stored inside the resource envelope, not in a
side stream. Status has the frozen three-layer shape (D088): the universal
`ResourceStatus` base, the ResourceType-common `status.resource` object, and an
optional Provider-specific `status.provider` extension. A status write is
validated per layer before storage - total canonical serialized status ≤ 64 KiB,
`status.resource` typed detail ≤ 32 KiB, `status.provider.details` ≤ 32 KiB, and
bounded condition/list/map cardinality - and a `status.provider` is validated
against the installed Provider's registered, signed extension schema
(`schemaId`/`schemaVersion`) with strict unknown-field denial. An over-limit
write fails closed with `status-oversize`, an unregistered/unknown-field
extension with `status-provider-schema-invalid`, and a duplicated
universal/`status.resource` field with `status-provider-overlap`, all before any
redb mutation (see `ADR-046-resource-object-model` § Status). All present layers
are committed atomically in one status mutation. Controllers write status only
on a material change, so status churn cannot outpace revision compaction; there
are no high-frequency byte streams, logs, metrics, or ring buffers in the
store.

Unknown table/encoding/schema versions fail closed.

## Store errors

`ResourceErrorKind` and `ResourceError` live in
`packages/d2b-contracts/src/v3/error.rs`. `StoreErrorKind` and `StoreError`
live in `packages/d2b-resource-store/src/error.rs`. The store set is closed at
exactly 34 serialized lower-kebab strings: the exact 31
`ResourceErrorKind` strings enumerated by
`ADR-046-resource-api-and-authorization` section "Errors", plus exactly:

- `store-integrity-failure`;
- `store-backpressure`;
- `store-quarantined`.

The API boundary in `packages/d2b-resource-api/src/error.rs` owns the only
mapping:

| `StoreErrorKind` string | `ResourceErrorKind` string |
| --- | --- |
| Any of the 31 shared strings | The identical string |
| `store-integrity-failure` | `internal-integrity-failure` |
| `store-backpressure` | `backpressure` |
| `store-quarantined` | `resource-plane-unavailable` |

The mapping is total and one-way. There is no
`ResourceErrorKind`-to-`StoreErrorKind` conversion, and neither layer may add a
fallback or unknown variant. Integrity means an inconsistent database,
encoding, or index; backpressure means bounded store queue/pool admission was
refused before a transaction opened; quarantine means the Zone resource plane
has already failed closed and will accept no store operation.

## Async storage adapter

redb is synchronous. The Zone runtime exposes only async resource APIs.

- one bounded fair async write queue feeds one dedicated blocking store actor;
- read requests execute as short-lived blocking MVCC read transactions through
  a bounded adapter pool;
- async executor threads never call blocking redb/filesystem APIs;
- read transactions cannot survive an await or watch lifetime;
- per-principal/controller fair admission prevents one caller monopolizing the
  writer;
- queue saturation returns typed backpressure before opening a transaction.

The writer may perform bounded group commit. It takes a small immediately
available batch from the fair queue without sleeping/debouncing:

- preserves per-principal fairness and per-resource order;
- validates each mutation/result independently in one write transaction;
- includes only non-conflicting/dependency-compatible mutations;
- assigns one Zone revision and ordered per-mutation ordinals;
- performs one crash-safe commit/fsync;
- returns each caller its own success/conflict/error;
- emits ordered ChangeBatch entries and controller hints after commit.

A mutation that depends on/conflicts with another queued mutation is ordered
explicitly or committed separately. Group commit never silently changes atomic
batch semantics or shares authorization outcomes.

## Write transaction

For every mutation/bounded group:

1. authenticate/admit request before queueing;
2. begin one redb write transaction;
3. recheck policy/API/controller generations;
4. resolve target refs/UIDs and expected revisions;
5. validate ResourceType schema, owner graph, finalizers, quotas, and controller
   ownership;
6. reject conflicts with no mutation;
7. update resources plus every affected index;
8. update durable operation/idempotency state;
9. allocate exactly `current_revision + 1`;
10. append one ordered ChangeBatch;
11. update store metadata;
12. commit with the selected crash-safe redb durability;
13. only after commit, swap in-memory indexes and push matching watch/reconcile
    events directly to d2b-bus.

No success, status, watch event, reconcile hint, or effect starts before durable
commit returns.

Two callers using the same expected resource revision cannot both succeed. The
first valid durable commit wins; the other gets `resource-conflict` with the
current revision and may re-read/retry.

### Expedited commit proof (D090)

For an expedited (`waitForReconcile`) `Create`/`UpdateSpec`/`Delete`, the writer
reserves the target revision under one mutation ticket (`operationId`) while the
owning controller runs preflight/plan in parallel. The controller performs no
external effect, finalizer release, or status mutation until the writer emits,
only after step 12 (durable commit), a typed
`CommittedRevisionProof { resourceUid, generation, revision, operationId }`. If
the transaction fails or is rejected at any step, the writer emits `Abort` for
that `operationId` and no effect occurs. A durable commit is authoritative and
is never rolled back because the subsequent expedited reconcile pass fails or
times out. The commit still emits the ordinary ChangeBatch/hint (step 13); the
expedited request additionally enters the priority reconcile lane. Status
written by the expedited pass is a normal later asynchronous status mutation
with its own revision, never part of the spec/create commit.

## Revision model

Zone revision:

- is a monotonically increasing u64 inside one Zone;
- increments once per successful write transaction;
- is not wall time or cross-Zone causal order;
- orders changes within a ChangeBatch by bounded ordinal.

Resource metadata.revision is the Zone revision of its latest change. List
returns a consistent MVCC snapshot plus snapshot revision.

## ChangeBatch

Each event contains only:

- revision and ordinal;
- ResourceType/name/UID;
- event `Created|SpecUpdated|StatusUpdated|MetadataUpdated|DeleteRequested|Deleted`;
- old/new generation where applicable;
- current ownerRef/owner UID;
- payload digest;
- complete bounded ResourceEnvelope when watch authorization permits;
- operation/correlation IDs.

No secret, credential byte, terminal data, raw Provider state, pidfd, host path,
or process argv/environment enters revision_log.

## Watches

Watch is application-owned because redb has no native changefeed.

1. Caller supplies exact authorized ResourceTypes/filters and `afterRevision`.
2. Service replays matching revision_log entries.
3. Under the watch coordinator it registers live delivery and rechecks the
   high-water revision, preventing a replay/live gap.
4. Live committed changes are pushed immediately through d2b-bus named streams.
5. Caller acknowledges fully processed revisions.
6. Disconnect resumes from the last acknowledged revision.

There is no fixed polling, debounce, or compaction-tick delivery delay.

The log is bounded by bytes, count, and age. Slow clients cannot pin it forever.
Compaction advances a durable floor and deletes old batches in bounded write
transactions. A cursor below the floor receives `revision-expired` plus current
revision and must list/re-watch.

## Owner triggers

Every child mutation updates owner_index/ChangeBatch in the same transaction.
After commit, the dispatcher emits `owned-resource-changed` for the singular
owner with child ref/UID/revision/event.

- repeated child changes coalesce only while owner is queued/running;
- child status-only updates still trigger the owner;
- propagation follows the validated acyclic owner chain;
- strict depth/work budgets prevent amplification;
- parent relists owner_index and reasserts its complete child set.

## Deletion

Delete flow:

1. set metadata.deletionRequestedAt and trigger controller/finalizers;
2. reconcile children/finalizers child-first;
3. final transaction creates a `Deleted` change event, removes resource/type/
   owner/controller indexes, and commits;
4. GET returns not found immediately after commit.

No resource tombstone is retained. revision_log is the deletion history.

## EphemeralProcess cleanup

Terminal EphemeralProcess status includes completedAt and cleanupEligibleAt.

- Succeeded defaults successfulTtl to `1h`.
- Failed defaults failedTtl to `24h`.
- Pending/Ready/Unknown/Degraded never age out.
- explicit bounds are enforced by schema/policy;
- finalizers and incident holds block cleanup;
- cleanup uses expected revision and the normal delete transaction;
- owner deletion/reconcile notification is preserved.

## Backup, restore, and physical schema upgrade

Backup uses a bounded consistent logical read snapshot containing:

- store identity/schema and revision boundary;
- API schemas/catalog;
- resources;
- operations;
- Zone-link cursors;
- index checksums/rebuild inputs.

An open database file is never copied unless the pinned redb version explicitly
documents that operation as crash-safe.

Restore/upgrade:

1. stop Zone mutation admission;
2. validate source identity/digest/schema;
3. build a staged new redb database;
4. validate resources, refs, owners, indexes, revisions, operations;
5. fsync staged file and parent;
6. atomically publish while retaining prior file for the bounded rollback
   window;
7. reopen/rebuild indexes before readiness.

Corruption or ambiguous publication quarantines the Zone resource plane. It
does not create a fresh store or claim partial success.

## Performance contract

On the pinned reference host/release profile:

| Metric | Hard target |
| --- | --- |
| Normal readiness | <=500 ms |
| Aggregate Zone resource service/store + fixed system-core and system-minijail controllers idle RSS | <=64 MiB |
| p95 local Get/bounded List | <=2 ms |
| p95 crash-safe single-resource mutation | <=10 ms |
| p95 durable commit → matching controller handler start | <=5 ms |
| p95 ready Process commit → launch-attempt start | <=20 ms |

Benchmark fixtures include:

- empty store;
- 10,000 resources;
- 100 live watches;
- 1/10/100 concurrently ready Process resources;
- expected-revision conflict storm;
- owner-trigger fan-in/chain;
- revision compaction;
- forced crash at every commit boundary;
- backup/restore/internal schema upgrade;
- repeated open/close and long-reader rejection.

Failure to meet a hard target changes the Proposed design. Durability,
authorization, or audit cannot be weakened to pass.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | No generic store. Reuse `d2b-core/src/storage.rs`, `sync.rs`; daemon snapshot/operation records; `d2b-realm-router` idempotency; broker fd/path safety |
| Evidence class | Current storage/locks/ledgers are mixed reachable/generated; redb store is ADR-only |
| Behavior retained | Single repair owner, no-follow/fd-relative safety, atomic rename/fsync, OFD locks, bounded records, pidfd non-persistence, idempotency/quarantine |
| Required delta | Entire redb schema, store actor, revisions, indexes, watches, conflicts, backup/upgrade |
| Reuse path | Extract exact storage/atomic/idempotency validators named below; redb only supplies ACID B-trees |
| Replacement/deletion | No existing state file/ledger is removed until its owning resource/operation migration lands |
| Feasibility proof | SPIKE-01 and SPIKE-02 are specified but unexecuted; the redb pin and production backend remain provisional |
| Future owner | Work items below |

## Feasibility gate and implementable scope

The exact redb 4.1.0 dependency is present so the disposable spike and
spike-independent contract code compile against one reviewed API. Its presence
is not evidence that redb meets this store's workload. Production backend
adoption remains conditional on successful SPIKE-01; post-commit dispatcher
and watch adoption remains conditional on successful SPIKE-02.

The following work is engine-neutral and may proceed before either spike:

- the exact ten-table schema, key component shapes, D099 key-space and value
  discriminants, codecs, decode rejection rules, and golden vectors;
- the closed D111 resource/store error types and one-way mapping;
- storage-neutral request, response, trait, store-identity, transaction-state,
  and expected-revision types;
- hermetic small-scale codec, table-open, atomic index/revision, and transaction
  semantic tests that make no scale, latency, RSS, crash-recovery, or production
  suitability claim;
- the D115 generated-storage-row source contract.

The remainder is genuinely gated. `ADR046-store-001` cannot adopt or complete
the production redb backend, fair actor, MVCC pool, group commit, crash
recovery, or benchmark validation until SPIKE-01 passes.
`ADR046-store-002` cannot start its production replay/live watch, compaction,
owner-hint dispatcher, or latency integration until both SPIKE-01 and SPIKE-02
pass. `ADR046-store-003` may author the storage-row source contract, but its
redb logical backup, restore, staged migration, and crash-publication code wait
for SPIKE-01. SPIKE-02 does not gate codecs, physical table definitions, error
types, or the storage-row source contract.

## Implementation work items

### ADR046-store-001

| Field | Value |
| --- | --- |
| Dependency/owner | Resource-object contracts; store integrator. Contract-only scope may proceed, but production redb backend completion depends on successful SPIKE-01 evidence |
| Current source | `packages/d2b-core/src/storage.rs`, `sync.rs`; `packages/d2bd/src/supervisor/state.rs`, `daemon_audit.rs`; `d2b-realm-router/src/lib.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/error.rs`; `packages/d2b-resource-store/src/lib.rs`, `error.rs`; `packages/d2b-resource-store-redb/src/lib.rs`, `schema.rs`, `keys.rs`, `transaction.rs` |
| Detailed design | Before SPIKE-01: closed errors, store-neutral types, exact table/discriminant codecs and golden vectors, and hermetic small-scale transaction semantics. After SPIKE-01 passes: fd-backed redb backend, store identity, fair actor, MVCC reads, atomic indexes/revisions/operations/conflicts, crash recovery, and production benchmarks. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Zone runtime owns store; resource API is sole caller |
| Data migration | Full reset; logical backup only for v3 stores |
| Validation | Pre-spike codec/golden-vector and small-scale semantic tests make no scale claim; completion requires SPIKE-01 evidence, unit/property/fault tests, and the hard benchmark |
| Removal proof | Existing ledgers removed only by owning future work items |

### ADR046-store-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-store-001 contract scope plus successful SPIKE-01 and SPIKE-02 evidence; watch/reconciliation integrator |
| Current source | `packages/d2b-realm-core/src/mux.rs`, `d2b-realm-router/src/mux_session.rs`, `route_engine.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-store-redb/src/revision_log.rs`, `packages/d2b-resource-api/src/watch.rs` |
| Detailed design | replay/live no-gap watch, cursors, owner hints, compaction floor, expired relist |
| Integration | d2b-bus named streams; controller toolkit |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | SPIKE-01 correctness and SPIKE-02 latency evidence before implementation; deterministic watch/compaction/disconnect/fan-in tests afterward |
| Removal proof | Not applicable |

### ADR046-store-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-store-001 contract scope; storage-row source contract may proceed, but redb backup/migration completion depends on successful SPIKE-01 evidence; storage/broker integrator |
| Current source | `nixos-modules/storage-json.nix`, `packages/d2b-priv-broker/src/ops/storage_contract.rs`, existing marker/ownership tests |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-store-redb/src/backup.rs`, `migration.rs`; `packages/d2b-contracts/src/v3/storage.rs`; `nixos-modules/zone-storage-json.nix`. The generated schema and rendered-contract parity test are integrator-owned outputs named by D115, not implementation-slice destinations |
| Detailed design | Before SPIKE-01: generated-storage-row source contract. After SPIKE-01 passes: fd-backed provision/open, marker identity, logical backup, staged restore/upgrade, and corruption quarantine. The integrator generates the schema and owns the contract-test addition after the source slice merges |
| Integration | Broker/Host/Guest storage owner passes File to Zone runtime |
| Data migration | Destructive v3 bootstrap; v3-to-v3 logical restore |
| Validation | Storage-row source validation may proceed; marker replacement, crash publication, backup/restore/upgrade tests require SPIKE-01 evidence first |
| Removal proof | Not applicable |
