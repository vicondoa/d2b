# ADR 0046 resource reconciliation

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resource-reconciliation` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | Core hint dispatcher, Provider controller toolkit |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-resource-store-redb`, `ADR-046-resource-api-and-authorization` |
| Supersedes | None |

## Ownership

Provider controller processes own:

- watch subscription;
- local pending/running map;
- per-resource serialization;
- cross-resource async concurrency;
- reconcile/observe/finalize handlers;
- retry/requeue decision;
- status/mutation batches.

Core owns:

- watch-plan validation;
- ResourceType/provider/controller ownership;
- type/owner/dependency reverse indexes;
- change filtering;
- converged/self-status suppression;
- owner triggers;
- coalesced high-water hints;
- controller lease/generation;
- fair API/watch budgets;
- durable revision/watch delivery.

Core does not execute Provider domain logic. Providers do not poll the store or
open broad watches.

## Controller descriptor

Each signed controller descriptor declares:

- Provider/component/controller ID and generation;
- ResourceTypes and versions;
- Host/Guest Provider capabilities supported;
- Process domains supported;
- spec/status/finalizer verbs;
- exact watch selectors;
- explicit dependency selectors;
- owner-child triggers consumed;
- reconcile/observe concurrency;
- maximum pending resources;
- observe/resync policy;
- finalizers owned;
- service/schema fingerprints;
- ResourceType-specific deadlines/retry classes.

Runtime registration must match the installed Provider descriptor and
authenticated Process/Host/Guest identity.

## Async interface

Language-neutral semantic interface:

```text
async describe() -> ControllerDescriptor
async validateSpec(context, resource) -> ValidationResult
async plan(context, resource, dependencies) -> ReconcilePlan
async reconcile(context, resource, dependencies) -> ReconcileResult
async observe(context, resource) -> ObservationResult
async finalize(context, deletingResource) -> FinalizeResult
async health() -> ControllerHealth
async drain(deadline) -> DrainResult
```

The official Rust toolkit exposes async traits and an async ResourceClient.
Non-Rust SDKs implement the same vectors/state machine.

No handler holds a redb transaction or blocking kernel/systemd/filesystem call
across an await. Blocking effects use explicit bounded adapters.

## Reconcile context

Context contains:

- controller/Provider/Process/Host/Guest/Zone identity and generations;
- target ResourceRef/UID/revision/generation;
- trigger reason set and high-water revision;
- operation/idempotency/correlation/trace IDs;
- attempt;
- monotonic deadline/cancellation;
- policy/API/config/controller revisions;
- capability-limited async ResourceClient.

It contains no database handle, direct broker socket, reusable credential, raw
route table, or authority supplied by the resource payload.

## Reconcile results

Result contains:

- processed revision/generation;
- zero or one ResourceMutationBatch;
- latest status/conditions/outcome;
- one disposition:
  - `converged`;
  - `pending`;
  - `degraded`;
  - `failed-retryable`;
  - `failed-terminal`;
  - `requeue-at`;
  - `finalized`;
- next observe/requeue time where applicable.

A stale mutation conflict discards the result; the toolkit re-reads and lets the
controller retry under policy. Core never merges stale Provider output.

## Async loop

1. Register descriptor/watch plan.
2. List resources requiring initial work and receive snapshot revision.
3. Open watch after snapshot revision.
4. Dedicated async receiver continuously consumes hints.
5. If a resource is idle, dispatch immediately.
6. If already queued/running, replace pending high-water revision and union
   non-droppable reasons.
7. Each resource has one running handler; independent resources run in parallel
   under semaphore/budget.
8. Handler reads one fresh resource/dependency snapshot.
9. Handler may asynchronously write Pending/starting status.
10. Handler starts effects in its own task/blocking adapter.
11. Receiver continues reading and dispatching other ready resources.
12. Handler commits mutation/status with expected revisions.
13. Toolkit acknowledges/checkpoints after commit or terminal no-mutation
    outcome.
14. Disconnect/revision-expired relists and rebuilds the queue.

There is no fixed polling interval, debounce window, or sleep between ready
resources.

## Expedited reconcile pass (D090)

An expedited (`waitForReconcile`) mutation enters a **bounded priority lane**
into the SAME per-resource single-flight reconciler; it does not create a second
executor and never bypasses per-resource serialization:

1. The controller may preflight/plan in parallel with Core admission/commit but
   starts no external effect, finalizer release, or status mutation until it
   receives the typed `CommittedRevisionProof { resourceUid, generation,
   revision, operationId }` from Core; an `Abort` for that `operationId` means
   no effect.
2. On proof, the expedited request is dispatched ahead of normally-queued work
   for that resource; any already-queued ordinary reconcile stays queued and
   runs after the expedited pass completes.
3. All effect IDs/idempotency keys derive from `(UID, generation, revision,
   operationId)`. The later ordinary re-entry observes the converged/progressing
   state and no-ops or rejoins the in-flight operation — it never duplicates an
   effect.
4. The pass returns a bounded `ReconcileProjection` and one **disposition**:
   `Converged`, `Progressing`, `Blocked`, `UpgradeRequired`, or `Failed`
   (these map onto the ordinary result dispositions; expedited completion is one
   pass reaching a disposition, not long-running external Ready).
5. Status persistence is asynchronous: the controller returns the projected
   layered status candidate; the actual `UpdateStatus` write is a normal later
   mutation. A timeout/cancel after commit yields a committed-but-reconcile-
   pending outcome and the ordinary queue continues.

Priority quotas and fairness bound the expedited lane so it cannot starve
ordinary reconciles or be used for DoS. Only authorized UX mutations/core (and
the admin `resource reconcile` action) use this lane.

## Currency and disruptive upgrade (D091)

Every controller additionally implements `assess_update`, `plan_upgrade`, and
`execute_upgrade` alongside ordinary reconcile, serialized through the same
per-resource single-flight so a reconcile and an upgrade never run concurrently
for one resource:

- **assess_update** runs on core/Provider-generation, artifact/image/NixOS-
  generation, immutable-spec, dependency, or security-policy change and writes
  the bounded `status.update` currency object (state/reasons/observed+target
  IDs/disruption/preserveState/owned+dependency aggregates). A controller MUST
  report `UpgradeRequired` for a disruptive change rather than apply it in
  place; non-disruptive changes reconcile normally.
- **plan_upgrade** produces a bounded plan (disruption class, preserveState,
  affected owned/dependent set); **execute_upgrade** applies it. The core
  Operation ledger persists the upgrade operation/idempotency/progress and
  resumes after crash/restart; `status.update` carries only the latest bounded
  plan/result, never a second ledger.
- Upgrade **preserves** the Resource UID and spec identity where possible and
  recycles only the realization and owned ephemeral Processes/endpoints; durable
  and state/secret Volumes and TPM identity are preserved (`preserveState:
  true`). `Replace` of the resource-row identity is used only when explicitly
  required and planned with ownership/state transfer; full factory reset is a
  separate destructive path (`ADR-046-reset-and-cutover`).
- A **dependency-aware planner** topologically drains, recycles, and restarts
  affected owned/dependent resources. Example: a GPU Device marks itself
  `UpgradeRequired`/`Blocked` while applications depend on it; the planner
  drains dependent Processes/Guests, recycles the GPU realization, then restarts
  the dependents — no surprise disruption. Core invokes dependency/owner
  triggers and aggregates self/owned/dependency currency for list/get.

## Trigger reasons

Closed common reasons:

- spec-generation-changed;
- owned-resource-changed;
- dependency-changed;
- dependency-ready;
- deletion-requested;
- finalizer-required;
- controller-generation-changed;
- Provider-generation-changed;
- policy-changed;
- security-policy-changed;
- artifact-or-image-changed;
- execution-status-changed;
- scheduled-observe;
- assess-update-due;
- upgrade-requested;
- expedited-mutation;
- retry-due;
- manual-reconcile;
- startup-relist.

Reasons are coalesced without dropping owner/deletion/finalizer/policy/
generation causes.

## Core suppression

Core may suppress:

- unbound ResourceTypes/scopes;
- irrelevant metadata/status fields;
- controller's own status-only event when no owner/dependency consumer needs
  it;
- object whose generation equals observedGeneration, controller generation is
  current, no dependency/owner/delete/observe/retry cause exists, and conditions
  do not require work.

Core may not suppress:

- any child mutation's ownerRef trigger;
- deletion/finalizer;
- policy/Provider/controller generation;
- explicit dependency;
- due retry/observe;
- Unknown state requiring observation.

## Owner reconciliation

On any child mutation:

1. store emits owned-resource-changed after durable commit;
2. owner hint includes child ref/UID/revision/event;
3. owner controller relists owner_index;
4. it compares complete desired children with observed children;
5. it creates missing children;
6. it repairs drift through expected-revision writes;
7. it requests deletion for children no longer desired;
8. child finalizers/status remain owned by their controllers.

Propagation to ancestors is acyclic, depth/budget bounded, and coalesced.

### ResourceImport projection ownership (D096)

A `ResourceImport` owns exactly one local projection **Service** through
`metadata.ownerRef: ResourceImport/<name>`. Its ResourceType is the same
qualified semantic/provider-neutral `*Service` type as the remote owner Service,
as bound by the signed projection factory. The consumer's local `providerRef`
selects the conformant implementation; the projection does not copy the
owner's `spec.provider`. Core rejects a missing/mismatched factory or any
semantic-type rewrite and never projects a Device, Endpoint, or `*Binding`.

Operator/Nix-authored matching same-Zone `*Binding` resources reference the
projection's `serviceRef` and an allowed consuming Guest/User/Zone. They are not
owned by the import. Binding spec contains desired consumer intent only; all
observations belong in status. Their Provider controller creates and reconciles
owned Process/Endpoint children. The import controller never creates, exports,
or deletes Binding; per-session leases/streams remain internal records.

Status and D091 update currency propagate owner Service → export → import →
projection Service → Binding → owned children. `ResourceExport` removal or
ZoneLink loss revokes the lease and marks the projection Service degraded/
revoked; Binding controllers then stop children in topological order. Import
finalization marks the projection draining, rejects new sessions, and waits for
all referencing Bindings to be deleted or retargeted. It then releases the remote
lease, deletes the projection Service and remaining provider-owned children, and
clears its own finalizer. `BindingReferencesRemain` is visible pending cleanup;
there is no implicit Binding cascade.

Service and Binding base reconcile inputs and status are
implementation-independent, and every selected Provider accepts the canonical
minimal base. PipeWire, CTAPHID, OTEL, and USBIP observations may appear only in
their registered bounded `status.provider` extension; they never affect the
semantic dependency keys or base conditions/errors.

### Authority adoption, quarantine, and drain-recycle (D097)

An authority owner (Resource or owner service Process for a scarce/singleton
backing) is adopted across a restart by its signed `ownerProof` (process/resource
identity), never by re-opening the backing speculatively. The reconciler
revalidates the authority index entry `(Zone/scope, authorityClass,
opaqueKeyDigest)` against the recovered owner:

- exact identity match → adopt in place, no re-open, no second effect;
- ambiguity (two candidates, or an index entry with no verifiable owner) →
  **quarantine** the authority (no effect, `Degraded` naming the incumbent owner
  digest) until an operator or a deterministic tiebreak resolves it;
- a duplicate config/API claimant → deterministic `duplicateConflict`, no
  second open.

A D091 upgrade of an authority **drains its consumers first** (leases/projections
released or migrated in topological order) and then recycles the authority owner;
it never recycles a backing while consumers still hold live leases. Reset
preserves or destroys the underlying backing state per the authority's explicit
per-authority disposition.

## Process fast path

When a Process or EphemeralProcess durable commit completes and dependencies
are ready:

- post-commit dispatcher pushes a matching hint immediately;
- p95 handler start is <=5 ms;
- Process Provider validates/starts launch attempt in a background task;
- p95 commit-to-launch-attempt is <=20 ms;
- status is written asynchronously;
- watch receiver continues reading;
- next independent Process may start before prior readiness/completion;
- only per-resource ordering, true dependencies, configured concurrency, or
  backpressure may delay dispatch.

The benchmark proves 1/10/100 ready Process resources and records queue/
event-loop responsiveness.

## Process status

Typical transitions:

```text
Pending (Queued/Starting condition)
  -> Ready
  -> Degraded | Failed | Unknown
```

Starting/retrying/draining are conditions/reasons, not common phases.

EphemeralProcess:

```text
Pending -> Succeeded | Failed | Unknown
```

It includes startedAt/completedAt/outcome/exitCode and:

- successfulTtl default 1h;
- failedTtl default 24h;
- cleanupEligibleAt;
- finalizer/incident-hold-safe deletion.

## Finalization

The exact finalizer owner receives deletion-requested. It returns:

- complete;
- pending with requeue-at;
- blocked with typed condition;
- ambiguous with no false success.

A controller clears only its finalizer. Core removes the resource only after all
finalizers and owned-child deletion complete.

## Resync and external drift

Controllers do not poll by default. A ResourceType whose external state can
drift declares a bounded observe interval. Core schedules exactly that
reconcile reason. Missed watch events are recovered by revision replay/relist.

## Status-first recovery after restart

Resource `status` is the default durable observation and recovery surface for
bounded non-secret operational state (D087): reconcile stage, opaque
non-authorizing external handles/IDs/digests, adoption observations, bounded
counters, closed-enum detail, dependency readiness, and last successful
checkpoints. It is written only on a material change and never carries secrets,
authority-conferring handles, private path/argv/environment/PID/unit data, or
high-frequency streams (see `ADR-046-resource-object-model` § Status bounds).

Status has the frozen three-layer shape (D088): the universal `ResourceStatus`
base, the ResourceType-common `status.resource` object, and an optional
Provider-specific `status.provider` extension. A controller writes all present
layers in one status mutation with a single expected revision; the layers never
diverge. Cross-resource and cross-provider reconcilers depend only on the
universal base plus `status.resource` (a base-only projection) and never read a
peer's `status.provider.details`; any field a second implementation needs is
promoted to `status.resource`.

Symmetrically, desired `spec` has the frozen three-layer shape (D089): the
universal envelope, the ResourceType base spec at `spec.*` (including
`spec.providerRef`), and an optional canonical `spec.provider =
{ schemaId, schemaVersion, settings }`. Generic controllers reconcile from the
base spec and base status only; a Provider controller additionally reads its own
`spec.provider.settings` and writes its own `status.provider`. A Provider that
cannot honor an optional base capability reports the provider-neutral
`unsupported-capability` outcome rather than ignoring or reinterpreting the base
field.

On a Zone or controller restart a controller re-reads its owned resources'
status and treats every field as **observation, not authority**. Before
relying on any recovered observation it reverifies against external reality —
re-discovering running processes from declared cgroup leaves, opening fresh
pidfds, and revalidating opaque external handles and markers against the live
external system — and quarantines or degrades any ambiguity. Status never
substitutes for that reverification and never carries or stands in for a
privileged effect. This makes a durable payload Volume unnecessary for a
component whose operational state is fully derivable from spec, status, the
core Operation ledger, and independent external observation.

## Backpressure and fairness

- bounded watch stream credit;
- bounded pending map;
- per-controller/Provider/Host/Guest concurrency;
- fair ResourceClient queues;
- reserved health/cancel/status capacity;
- typed backpressure, never silent drop of an admitted mutation;
- latest high-water coalescing for the same resource;
- no cross-resource eviction.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-realm-router` shared OperationRouter/mux/session lifecycle; d2bd DAG/topological executor/readiness/pidfd; role-specific state machines |
| Evidence class | Current route/DAG logic is tested; generic controller loop/hints are ADR-only |
| Behavior retained | Deterministic ordering, capability denial, idempotency, bounded queues, cancellation, fail-fast typed errors, pidfd adoption |
| Required delta | Async controller SDK/loop, store watches/hints, owner triggers, cross-resource concurrency, status batches |
| Reuse path | Extract pure state machines/limits and deterministic test clocks; replace role branches with ResourceType controllers |
| Replacement/deletion | DAG/role path remains until each successor controller/Process graph is integrated |
| Feasibility proof | Real multi-process controller over d2b-bus; latency/load/conflict/owner/finalizer tests |
| Future owner | Work items below |

## Implementation work items

### ADR046-reconcile-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0/W1a; controller toolkit owner |
| Current source | `packages/d2b-realm-router/src/lib.rs`, `mux_session.rs`, `session_lifecycle.rs`; `packages/d2bd/src/supervisor/dag.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-controller-toolkit/src/lib.rs`, `runner.rs`, `queue.rs`, `context.rs`, `result.rs` |
| Detailed design | Async ResourceReconciler, watch receiver, coalescing, per-resource serialization, parallel tasks, retry/checkpoint/finalize; expedited priority lane and `CommittedRevisionProof`-gated effects (D090); `assess_update`/`plan_upgrade`/`execute_upgrade` methods serialized in the same single-flight (D091) Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Provider controller binaries wrap handlers with toolkit |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Golden state-machine vectors, deterministic clocks, conflict/restart/queue tests; D090: commit-fails/Abort → no effect, controller finishes-before-commit gated on proof, effects-gate, status-write-delayed (`statusPersistence: pending`), normal-queued no-op/rejoin, concurrent mutation, delete event-only projection, expedited timeout committed-but-pending, restart re-entry no duplicate; D091: current/non-disruptive/each-trigger assess, UpgradeRequired-not-in-place, dependency propagation/topological drain-recycle-restart, GPU blocking, state/TPM preservation, crash/re-entry resume, single-flight reconcile-vs-upgrade serialization |
| Removal proof | Current per-role orchestration removed only after ResourceType successors |

### ADR046-reconcile-002

| Field | Value |
| --- | --- |
| Dependency/owner | Store/API + ADR046-reconcile-001; core controller |
| Current source | `d2b-realm-core/src/route_engine.rs`, `allocator_engine.rs`; `d2b-realm-router/tests/transport_topology_harness.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/hints.rs`, `dependencies.rs`, `owner_reconcile.rs` |
| Detailed design | Watch-plan validation, indexes, suppression, owner/dependency hints, leases, startup relist, fair admission |
| Integration | Store post-commit dispatcher → d2b-bus controller streams |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Owner/dependency chains, suppression/no-loss, restart/relist, lease withdrawal |
| Removal proof | Not applicable |

### ADR046-reconcile-003

| Field | Value |
| --- | --- |
| Dependency/owner | Process Providers + benchmark owner |
| Current source | `d2bd/src/supervisor/dag.rs`, `pidfd.rs`, unsafe-local blocked supervisor, guest exec runner |
| Reuse action | adapt |
| Destination | `packages/d2b-controller-toolkit/benches/reaction.rs`, Process Provider integration tests |
| Detailed design | Commit-to-handler/launch fast path, nonblocking watch, parallel ready resources |
| Integration | Resource store → bus/session → controller → Process effect/status |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Hard <=5 ms/<=20 ms p95 gates and 1/10/100 Process concurrency |
| Removal proof | Not applicable |
