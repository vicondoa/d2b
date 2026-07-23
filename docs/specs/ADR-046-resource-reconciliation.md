# ADR 0046 resource reconciliation

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resource-reconciliation` |
| Parent | ADR 0046 |
| Status | Proposed |
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
- execution-status-changed;
- scheduled-observe;
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
| Reuse action | extract and adapt |
| Destination | `packages/d2b-controller-toolkit/src/lib.rs`, `runner.rs`, `queue.rs`, `context.rs`, `result.rs` |
| Detailed design | Async ResourceReconciler, watch receiver, coalescing, per-resource serialization, parallel tasks, retry/checkpoint/finalize |
| Integration | Provider controller binaries wrap handlers with toolkit |
| Data migration | None |
| Validation | Golden state-machine vectors, deterministic clocks, conflict/restart/queue tests |
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
| Data migration | None |
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
| Data migration | None |
| Validation | Hard <=5 ms/<=20 ms p95 gates and 1/10/100 Process concurrency |
| Removal proof | Not applicable |
