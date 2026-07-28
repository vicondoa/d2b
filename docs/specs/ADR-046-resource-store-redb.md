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

The owner passes one already-open regular database descriptor to the Zone
runtime as an owned `File` or `OwnedFd`; borrowed descriptors and retained raw
fd integers are forbidden. Before publishing the descriptor to any thread or
task, the receiver verifies with `F_GETFD` that `FD_CLOEXEC` is set. An
ordinary handoff without that bit fails closed. An `SCM_RIGHTS` receiver must
use `recvmsg(..., MSG_CMSG_CLOEXEC)` so close-on-exec is set atomically as the
descriptor enters the process, then perform the same verification. A reviewed
equivalent must provide the same atomic receipt guarantee; repairing a
received descriptor later with `F_SETFD` is forbidden because it races
fork+exec.

The pinned redb API must use `FileBackend::new(File)` or an equivalently
reviewed owned-fd API. The Zone runtime does not resolve a caller-controlled
store path. Tests cover direct and `SCM_RIGHTS` receipt, the defined fork
inheritance behavior, and absence of the database descriptor after exec,
including a receipt racing a fork+exec probe.

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
migration in `ADR046-store-005`.

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

`d2b-resource-store` is runtime- and storage-neutral: it depends on neither
Tokio nor redb, and its native async trait methods return `impl Future + Send`.
The API holds one concrete store implementation and test fakes are generic
parameters, so no trait object or `async-trait` dependency is used.
The main workspace keeps the ten-table names and codec discriminants
engine-neutral and does not resolve redb. The exact redb 4.1.0 pin remains
inside the disposable proof workspace until the unchanged feasibility gate
passes. A future production backend adds redb and enables only the minimum
Tokio features (`rt`, `sync`, and `time`); the resource API additionally
enables `macros`, and only the Zone runtime binary enables `rt-multi-thread`.

- one bounded fair async write queue of 256 requests feeds one dedicated
  blocking store actor;
- a group commit contains at most 16 mutations;
- read requests execute as short-lived blocking MVCC read transactions through
  a bounded pool of 4 threads with at most 16 concurrent transactions;
- each read transaction has a 250 ms lifetime ceiling;
- one global watch-admission budget holds at most 1024 queued delivery entries
  across all registrations; there is no per-watch 1024-entry queue;
- each watch retains only bounded cursor/filter/accounting state;
- exhausted admission returns typed backpressure before registration;
- a watcher that cannot release its budget deterministically is evicted and
  resumes from its last acknowledged revision;
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
2. Service range-seeks the big-endian revision key at `afterRevision` and
   streams only later rows. It never scans or decodes older complete
   ResourceEnvelopes.
3. Under the watch coordinator it registers live delivery and rechecks the
   high-water revision, preventing a replay/live gap.
4. Live committed changes fan out one shared immutable decoded ChangeBatch;
   registrations do not receive cloned envelopes or dedicated deep queues.
5. One global bounded watch-admission budget accounts for all registrations
   and queued delivery work. Admission exhaustion returns typed backpressure.
   A deterministic slow-watcher policy evicts a watcher that cannot release
   budget and records the last acknowledged cursor for resume.
6. Caller acknowledges fully processed revisions.
7. Disconnect or slow-watcher eviction resumes from the last acknowledged
   revision.

There is no fixed polling, debounce, or compaction-tick delivery delay.

The log is bounded by bytes, count, and age. Slow clients cannot pin it forever.
Compaction advances a durable floor and deletes old batches in bounded write
transactions. A cursor below the floor receives `revision-expired` plus current
revision and must list/re-watch.

The watch-budget implementation exports a bounded saturation snapshot and
metrics with current registrations, budget used and capacity, admission
rejections, slow-watcher evictions, and replay work. These signals use closed
labels and expose no selectors, resource names, subjects, cursors, or payloads.
Both `ADR046-store-002` and `ADR046-store-004` must demonstrate these signals
in their acceptance tests; a budget that saturates silently is not acceptable.

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

Evidence for the aggregate RSS row is staged. SPIKE-01 measured a
whole-process maximum of 25,216 KiB (24.625 MiB), 640 KiB or about 2.6% above
24,576 KiB at the 10,000-resource/100-watch fixture. This metric is the
complete process maximum with no empty-process, runtime, allocator, or other
baseline subtraction. After the range-seek,
streaming-decode, shared-fan-out, and global-budget corrections,
`ADR046-store-004` records the same whole-process median against the unchanged
<=24 MiB gate. That rerun gates backend and watch-dispatcher acceptance;
contract-only store work has no RSS exit criterion and must not report the
aggregate row as passing.
The work items that land each fixed controller separately record
`Provider/system-core <=22 MiB` and `Provider/system-minijail <=12 MiB`.
Provider integration records the first valid all-three-live aggregate result,
which must remain <=64 MiB. The sub-budgets total 58 MiB; the remaining 6 MiB
is variance headroom, not an independently spendable budget.

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
| Current anchor | The engine-neutral generic store contract and service-to-store API wiring exist in `d2b-resource-store`, `d2b-resource-store-redb` contract modules, and `d2b-resource-api`. Native RBAC evaluates requests and the authenticated ttrpc adapter reaches the generic store interface in tests, but no production d2b-bus or Zone path dispatches the adapter. Reuse `d2b-core/src/storage.rs`, `sync.rs`; daemon snapshot/operation records; `d2b-realm-router` idempotency; broker fd/path safety |
| Evidence class | Generic store contract, native RBAC, service-to-store wiring, and the ttrpc adapter are `implemented-but-unwired`; production bus/Zone dispatch and the production redb backend are absent, and backend adoption remains spike-gated. The adapter's existence is not production reachability. Current storage/locks/ledgers are mixed reachable/generated |
| Behavior retained | Single repair owner, no-follow/fd-relative safety, atomic rename/fsync, OFD locks, bounded records, pidfd non-persistence, idempotency/quarantine |
| Required delta | Dispatch the existing resource adapter from d2b-bus and wire a Zone runtime to the production backend; after the feasibility gate, implement the redb database, store actor, revisions, indexes, watches, conflicts, backup, and upgrade |
| Reuse path | Extract exact storage/atomic/idempotency validators named below; redb only supplies ACID B-trees |
| Replacement/deletion | No existing state file/ledger is removed until its owning resource/operation migration lands |
| Feasibility proof | SPIKE-01 and SPIKE-02 executed. Functional, crash, watch, conflict, and commit-to-handler thresholds passed, but SPIKE-01 whole-process RSS was 25,216 KiB (24.625 MiB), 640 KiB or about 2.6% above 24,576 KiB. The redb pin remains proof-only and the production backend remains provisional until the corrected design passes the unchanged whole-process gate. |
| Future owner | Work items below |

## Feasibility gate and implementable scope

The exact redb 4.1.0 dependency is present so the disposable spike and
spike-independent contract code compile against one reviewed API. Its presence
is not evidence that redb meets this store's workload. SPIKE-02 passed, while
SPIKE-01 failed the whole-process RSS gate. Production backend,
post-commit-dispatcher, and watch adoption remain blocked until the corrected
design passes an unchanged SPIKE-01 whole-process RSS rerun.

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

The remainder is genuinely gated. `ADR046-store-004` may implement the redb
backend and the measured-failure corrections, but cannot be accepted until the
unchanged whole-process RSS gate passes. It owns big-endian revision-key
range-seek replay, streaming decode without decoding older complete envelopes,
and shared immutable ChangeBatch fan-out. `ADR046-store-002` owns the one
global bounded watch-admission budget, small per-watch cursor/filter state,
typed backpressure, and deterministic slow-watcher eviction/resume. It also
cannot be accepted until that RSS rerun passes. Both items must export current
registrations, budget used/capacity, admission rejections, slow-watcher
evictions, and replay work. `ADR046-store-003` may author the storage-row
source contract, but `ADR046-store-005` redb logical backup, restore, staged
migration, and crash-publication acceptance waits for the corrected SPIKE-01
rerun. SPIKE-02 does not gate codecs, physical table definitions, error types,
or the storage-row source contract.

## Implementation work items

### ADR046-store-001

| Field | Value |
| --- | --- |
| Dependency/owner | Resource-object contracts; store contract integrator |
| Current source | `packages/d2b-core/src/storage.rs`, `sync.rs`; `packages/d2bd/src/supervisor/state.rs`, `daemon_audit.rs`; `d2b-realm-router/src/lib.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/error.rs`; `packages/d2b-resource-store/src/lib.rs`, `error.rs`; `packages/d2b-resource-store-redb/src/schema.rs`, `keys.rs`, `values.rs` |
| Detailed design | Keep `d2b-resource-store` free of redb and Tokio; expose native async trait methods returning `impl Future + Send`; use generic test fakes without trait objects or `async-trait`; freeze the closed error set, store-neutral request/response/trait/transaction types, engine-neutral ten-table names, `d2bkey/v1` and `d2bval/v1` codecs and discriminants, decode rejection rules, and literal golden vectors. The exact redb dependency remains isolated to the disposable proof workspace until the backend feasibility gate passes. `ResourceService` consumes the evaluator's verifier and store-identity binding exactly once into a private checked store. A mutating call can reach `commit_verified` only after that store matches both authorities, consumes the admitted mutation, and prepares its final identity and digest. This prevents a caller from forging admission or replaying it against another store, but it does not constrain the backend after verification. The backend is trusted to mutate only from the supplied `VerifiedMutation`, recheck captured policy/API-catalog/active-configuration/controller revisions inside the same transaction, preserve structural and atomicity checks, expose no independent mutation path, never evaluate RBAC, and never auto-retry a failed recheck. Production registration therefore requires security review and backend conformance tests. This contract item makes no scale, latency, RSS, crash-recovery, or production-suitability claim and has no feasibility dependency. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Resource API consumes the typed contract; production backend wiring belongs to ADR046-store-004 |
| Data migration | None - contract only |
| Validation | Literal codec golden vectors and decode-rejection cases; hermetic small-scale transaction semantics that make no scale claim; compile tests for `Send` futures and generic fake injection; proof-origin, single-owner binding, cross-store rejection, and in-transaction revision-recheck tests for sealed admission evidence; policy test forbidding resource-store dependency on API/RBAC symbols; engine-neutral table-descriptor assertion |
| Removal proof | Existing ledgers removed only by owning future work items |
| Implementation state | Merged |
| Evidence | Every destination is present: `packages/d2b-contracts/src/v3/error.rs`, `packages/d2b-resource-store/src/{lib,error}.rs`, and `packages/d2b-resource-store-redb/src/{schema,keys,values}.rs`, with literal vectors, rejection tests, generic fake tests, sealed admission tests, and engine-neutral table descriptors. The main workspace has no redb dependency. |

### ADR046-store-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-store-001; ADR046-feasibility-001; store backend integrator |
| Current source | `packages/d2b-core/src/storage.rs`, `sync.rs`; `packages/d2bd/src/supervisor/state.rs`, `daemon_audit.rs`; `d2b-realm-router/src/lib.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-store-redb/src/lib.rs`, `actor.rs`, `transaction.rs` |
| Detailed design | Adopt the already-pinned redb API only behind the failed-spike correction and rerun gate. Implement the redb engine, owned-fd database open, store identity, fair actor, MVCC reads, atomic indexes/revisions/operations/conflicts, crash recovery, and the contract constants: write queue 256, group-commit batch 16, read pool 4, concurrent reads 16, and read lifetime 250 ms. Revision replay range-seeks the big-endian revision key, streams only rows after `afterRevision`, and never decodes older complete envelopes. Live delivery shares one immutable decoded ChangeBatch across matching watchers instead of cloning each envelope. The global admission budget and slow-watcher policy belong to `ADR046-store-002`, not this backend. Use full crash-safe durability with one fsync per write transaction; no reduced-durability mode. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Zone runtime owns the concrete backend; resource API is the sole caller through ADR046-store-001 |
| Data migration | Full reset; logical backup belongs to ADR046-store-005 |
| Validation | Corrected SPIKE-01 evidence passing the unchanged whole-process <=24 MiB gate with no baseline subtraction; range-seek tests proving older revisions and envelopes are neither scanned nor decoded; shared immutable ChangeBatch fan-out tests; integration tests for the exported watch-budget saturation snapshot (current registrations, budget used/capacity, admission rejections, slow-watcher evictions, replay work); conformance tests proving verified-only mutation, no independent write path, and the required structural and atomic checks; security review of each registered backend; unit/property/fault tests and the hard 10,000-resource/100-watch benchmark; owned-fd and `FD_CLOEXEC` checks for direct and `SCM_RIGHTS` receipt plus fork/exec inheritance probes; exact dependency/feature-policy lint; queue/pool constant assertions; paused-clock read-expiry tests; no reduced-durability call-site lint |
| Removal proof | Existing ledgers remain until their owning migration work items land |
| Implementation state | Planned |
| Evidence | `packages/d2b-resource-store-redb/src/actor.rs` and `transaction.rs` are absent. SPIKE-01 executed and failed the whole-process RSS threshold, so the existing `lib.rs` remains contract-only and no production backend is accepted. |

### ADR046-store-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-store-004; ADR046-feasibility-001; watch/reconciliation integrator |
| Current source | `packages/d2b-realm-core/src/mux.rs`, `d2b-realm-router/src/mux_session.rs`, `route_engine.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-store-redb/src/revision_log.rs`, `packages/d2b-resource-api/src/watch.rs` |
| Detailed design | Implement replay/live no-gap watch, cursors, owner hints, compaction floor, and expired relist around one global bounded watch-admission budget. Registrations retain only small cursor/filter/accounting state. Budget exhaustion returns typed backpressure before registration; deterministic slow-watcher eviction releases its budget and resumes from the last acknowledged cursor. Export current registrations, budget used/capacity, admission rejections, slow-watcher evictions, and replay work with closed non-sensitive labels. Range-seek replay, streaming decode, and shared immutable ChangeBatch fan-out are supplied by `ADR046-store-004`. |
| Integration | d2b-bus named streams; controller toolkit |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | SPIKE-02 latency evidence and a corrected SPIKE-01 rerun passing the unchanged whole-process <=24 MiB gate with no baseline subtraction; deterministic watch/compaction/disconnect/fan-in tests; global-budget exhaustion and typed-admission-backpressure tests; deterministic slow-watcher eviction/resume tests; saturation-signal tests for current registrations, budget used/capacity, admission rejections, slow-watcher evictions, and replay work |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | Both destinations are absent: `packages/d2b-resource-store-redb/src/revision_log.rs` and `packages/d2b-resource-api/src/watch.rs`. SPIKE-02 passed, but SPIKE-01 failed the whole-process RSS threshold; production watch integration remains blocked pending the corrected rerun. |

### ADR046-store-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-store-001; storage-row contract integrator |
| Current source | `nixos-modules/storage-json.nix`, `packages/d2b-priv-broker/src/ops/storage_contract.rs`, existing marker/ownership tests |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/storage.rs`; `nixos-modules/zone-storage-json.nix`; `docs/reference/schemas/v3/zone-storage.json`; `packages/d2b-contract-tests/tests/zone_storage_contract.rs` |
| Detailed design | Freeze the closed `ZoneStoreStorageRow`: opaque zone-store and parent-directory ids plus required ownership, filesystem, locking, marker, replacement-detection, fsync, and publication invariants, never a host path |
| Integration | Generated storage-row contract is consumed by the broker storage owner and ADR046-store-005 |
| Data migration | None - contract only |
| Validation | Storage-row source validation, generated-schema drift, and rendered-contract parity |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | All destinations are absent: `packages/d2b-contracts/src/v3/storage.rs`, `nixos-modules/zone-storage-json.nix`, `docs/reference/schemas/v3/zone-storage.json`, and `packages/d2b-contract-tests/tests/zone_storage_contract.rs`. |

### ADR046-store-005

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-store-003; ADR046-store-004; ADR046-feasibility-001; storage/broker backend integrator |
| Current source | `nixos-modules/storage-json.nix`, `packages/d2b-priv-broker/src/ops/storage_contract.rs`, existing marker/ownership tests |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-store-redb/src/backup.rs`, `migration.rs`; `packages/d2b-contracts/src/broker_wire.rs` (`OpenZoneStore` request/response); `packages/d2b-priv-broker/src/ops/zone_store.rs`, `live_handlers.rs`, `fd_passing.rs`; `packages/d2b-priv-broker/tests/zone_store.rs` |
| Detailed design | Consume the D115 storage-row contract for fd-backed provision/open and marker identity; implement logical backup, staged restore/upgrade, crash-safe publication, replacement detection, and corruption quarantine. Add a typed broker `OpenZoneStore` operation that accepts only opaque storage-row ids, resolves and validates the signed row, provisions or opens the database without a caller path, and returns exactly one owned descriptor with atomic close-on-exec receipt. |
| Integration | Broker storage owner passes the owned database File to the Zone runtime backend from ADR046-store-004 |
| Data migration | Destructive v3 bootstrap; v3-to-v3 logical restore |
| Validation | Corrected SPIKE-01 evidence passing the unchanged whole-process RSS gate; marker replacement, crash publication, backup/restore/upgrade, and store-identity mismatch tests; broker wire codec/unknown-op tests; opaque-id/path-injection and signed-row mismatch rejection; provision/open idempotency; exactly-one-fd `SCM_RIGHTS` transfer with `MSG_CMSG_CLOEXEC`, `F_GETFD` verification, and fork/exec non-inheritance; audit record contains the operation/result but no host path |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | Backup/migration, broker wire operation, broker handler, fd-handoff integration, and broker tests are absent; SPIKE-01 executed but failed the whole-process RSS threshold. |
