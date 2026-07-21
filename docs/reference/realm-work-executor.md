# Realm routing / work-executor / transport fabric

**Diataxis category:** reference.

This page documents the W8 `realm-routing-work-executor-fabric` component: one
coherent typed dispatch surface spanning `d2b-realm-router`,
`d2b-realm-transport`, and `d2b-exec-runner`. It composes only already-owned,
already-tested state in those crates (`RealmEntrypointTable`,
`DurableExecTable`, `OperationRouter`, `RemoteFullHostAdapter`,
`RemoteNodeRegistry`, `SessionLifecycle`, `LoopbackTransport`,
`LocalTcpTransport`) and introduces no new realm relay, session, or provider
credential; no remote node registry outside the existing `RemoteNodeRegistry`;
and no free-form path/argv construction. It preserves
[ADR 0032](./../adr/0032-d2b-v2-constellation-control-plane.md) and
[ADR 0045](./../adr/0045-provider-and-transport-framework.md): relay identity
is never mapped to local admin authority, the host-resident `WorkExecutor`
holds no realm relay credential and no remote-node registry of its own, and
the allocator surface (`allocator.rs`/`allocator_engine.rs`) is consumed as-is,
never edited.

None of this is wired into a running control plane by this change. Every new
module is written so that adding it costs an integrator only the `mod`/
`pub use` lines documented in [Integrator wiring](#integrator-wiring) below —
no other file needs to change, no `Cargo.toml` needs a new dependency, and no
existing type's shape changes.

## Component map

| File | Role |
| --- | --- |
| `packages/d2b-realm-router/src/work_executor.rs` (new) | `WorkExecutor`: the single typed dispatch entry point tying realm resolution, host-resident authorization/idempotency + durable execution, and an *injected* gateway-backed dispatch port together. |
| `packages/d2b-realm-router/src/execution.rs` (touched) | Adds `state_code(ExecState) -> &'static str` (stable observability vocabulary). Also carries the test-only conditional module declaration that brings `work_executor.rs` into this crate's own `cargo test` — see [Test-only `#[cfg(test)]` module declarations](#test-only-cfgtest-module-declarations--not-production-wiring). |
| `packages/d2b-realm-router/src/remote_node.rs` (touched) | Adds the `GatewayPort` trait, its reference implementer `SingleGatewayPort`, and an `original_operation_id` field/helper on `RemoteDispatchOutcome` so a retry's own id is never confused with the router's first-attempt id. This file is unconditionally compiled (already `pub mod remote_node;` in `lib.rs`), so it is where `WorkExecutor`'s gateway-crossing contract has to live. |
| `packages/d2b-realm-router/src/target_resolver.rs`, `session_lifecycle.rs` | Consumed as-is by `WorkExecutor`; not modified in this pass. Owned only because they define the composed types. |
| `packages/d2b-realm-transport/src/fabric.rs` (new) | `TransportFabric`: a scheme-keyed composition of `TransportProvider` impls (e.g. `LoopbackTransport`, `LocalTcpTransport`) behind one `TransportProvider` facade, with a persistent bounded fan-in `FabricListener`. |
| `packages/d2b-realm-transport/src/local_tcp.rs` (touched) | Adds `LOCAL_TCP_SCHEME_NAME`, fixes `parse_target()` to strip its scheme prefix case-insensitively, classifies recoverable `accept`-stage io errors and attaches a bounded retry hint (`is_recoverable_accept_error`), and carries the test-only conditional module declaration that brings `fabric.rs` into this crate's own `cargo test`. |
| `packages/d2b-exec-runner/src/service_mode.rs` (touched) | Adds `ExecutionOutcomeCode` + `outcome_code_for_phase(StatusPhase)`. Also fixes a pre-existing parallel-test scratch-dir race in this file's own `#[cfg(test)]` helper (see [Regression: unique scratch allocation](#regression-unique-scratch-allocation-in-service_moders-tests)). |
| `packages/d2b-exec-runner/src/spec.rs` (touched) | Makes `validate_workload_unit_name` `pub`: a reusable shape-validator for the slot-derived workload unit name `d2b-guestd` writes, without duplicating its derivation. |

## `WorkExecutor` (router)

`WorkExecutor<C: Clock = SystemClock>` composes:

- `RealmEntrypointTable` — resolves an `OperationRequest`'s `RealmTarget` to
  `DispatchTarget::HostResident { target }` or
  `DispatchTarget::GatewayBacked { gateway, target }`.
- `DurableExecTable` — host-resident metadata for the exec family
  (`ExecStart`/`ExecAttach`/`ExecLogs`/`ExecCancel`).
- Its **own** `OperationRouter<C>` — the host-resident scope's authorization
  and idempotency owner. `WorkExecutor` never touches the durable table
  before this router has accepted the request.
- `local_node: NodeId` and `session_principal`/`capabilities` — the identity
  and grants this executor authorizes *host-resident* requests against.
- A bounded `HashMap<OperationId, SessionLifecycle>` for gateway-backed
  `DisplaySessionOpen` operations only, capped at
  `DEFAULT_MAX_GATEWAY_SESSIONS` (4096, override with
  `WorkExecutor::with_max_gateway_sessions`).

`WorkExecutor` holds **no** `RemoteFullHostAdapter` and **no**
`RemoteNodeRegistry` field. `WorkExecutor::dispatch(&mut self, req,
generation, client, gateway_port: &mut dyn GatewayPort)` takes the gateway
port as an explicit per-call argument instead:

1. Resolve `req`'s `RealmTarget` via the entrypoint table
   (`WorkExecutorError::Resolve` on failure).
2. **`HostResident { target }`**:
   - Validate `req.node == self.local_node` *before* touching any router or
     table state (`WorkExecutorError::WrongNode`, fail-closed).
   - Decode the operation body to the exec-family request shape its
     `OperationKind` requires; a malformed body is rejected
     (`WorkExecutorError::MalformedBody`) before the router is consulted.
   - For `ExecAttach`/`ExecLogs`/`ExecCancel`, check any *existing*
     `DurableExecTable` record's workload against `req.workload`
     (`WorkExecutorError::WorkloadMismatch` on a mismatch; no existing record
     is not itself a mismatch). For `ExecStart`, check the body's own
     `workload` field against `req.workload` the same way.
   - Route through `self.router.route_with_capabilities(req, session_principal,
     capabilities)`. `Accept` runs the durable-table action and (for mutating
     kinds) `mark_completed`/`mark_failed`s the router record, keyed by
     `req.operation_id` (the first attempt *is* the original); `Replay`
     decodes the previously-recorded `HostResidentOutcome` from the router's
     `OpaquePayload` cache without re-running the action, keyed by the
     router-supplied `original_operation_id` (the *first* attempt's id, which
     may differ from this call's own `req.operation_id` — see
     [Original-operation-id propagation](#original-operation-id-propagation)
     below); `InProgress` returns
     `WorkExecutorError::HostOperationInProgress { original_operation_id }`
     carrying that same original id; every other refusal (capability denied,
     missing/conflicting idempotency key, principal mismatch, unsupported
     kind, dedup capacity exceeded, …) becomes a typed
     `WorkExecutorError::Router(ConstellationError)` via `route_decision_error`.
     **Empty capabilities and a missing idempotency key on a mutating
     operation are therefore rejected by construction**, before the durable
     table is ever touched.
   - Returns `WorkDispatchOutcome::HostResident { original_operation_id,
     outcome: HostResidentOutcome }`. Callers that need to correlate a
     status query, a replay, or a session-lifecycle key with *this* logical
     operation must use `original_operation_id`, never `req.operation_id` —
     the two are only guaranteed equal on the very first (accepted) attempt.
3. **`GatewayBacked { gateway, target }`**: hand the *unmodified* canonical
   `gateway`/`req` to the injected `gateway_port.dispatch_via_gateway(gateway,
   req, generation, client)`. `WorkExecutor` performs no authorization of its
   own here — that lives entirely inside whatever already-authorized
   `GatewayPort` implementation the caller injected (see
   [`GatewayPort` / `SingleGatewayPort`](#gatewayport--singlegatewayport-remote_noders)
   below). Session-lifecycle tracking (bounded by `max_gateway_sessions`) is
   **reservation-before-dispatch**, not allocate-after-success:
   - If `req.operation_id` is not yet tracked and `DisplaySessionOpen` tracks
     sessions for this kind, capacity is checked and a *provisional*
     `SessionLifecycle::new()` entry is inserted **before**
     `gateway_port.dispatch_via_gateway` is ever called. At capacity, this
     returns `WorkExecutorError::GatewaySessionCapacityExceeded` immediately —
     proving the peer is never contacted for a request that cannot be
     tracked. If `req.operation_id` is *already* tracked (a retry reusing the
     same id), no new reservation is made and no capacity check runs.
   - After the call, the outcome's `original_operation_id` (see below) is
     resolved. On `Ok(Sent | Replayed)`: if the original id differs from
     `req.operation_id` and this call made the provisional reservation, that
     now-superfluous reservation is removed and the lifecycle is instead
     finalized/advanced under the *original* id (best-effort — if capacity is
     exhausted at that point the remote dispatch has already succeeded, so
     tracking is skipped rather than erroring). `Ok(QueryRemoteState { .. })`
     releases any provisional reservation this call made and only *reports*
     an existing tracked phase for the original id, never allocating one. On
     `Err(_)`, **only** a reservation this exact call created is removed —
     a pre-existing entry under the same id (for example a legitimately
     running session from an earlier successful attempt) is never touched by
     a failed/conflicting retry. `stop_gateway_session` drives orderly
     teardown (`stop()` + `finish_stop()`), evicting the session once it
     reaches `Stopped`.
   - Returns `WorkDispatchOutcome::GatewayBacked { original_operation_id,
     session_phase, remote }`.

### Original-operation-id propagation

`OperationRouter`'s dedup key (`DedupKey::for_request`) is
`(realm, principal, node, kind, idempotency_key)` — it deliberately excludes
`operation_id`, matching `OperationRequest::dedup_fingerprint_input()`'s own
exclusion of `operation_id`/`correlation_id`/`idempotency_key`/`trace`. Two
requests sharing the same idempotency key but carrying *different*
`operation_id`s are the same logical operation to the router: the *first*
attempt's `operation_id` is what the router remembers and replays under
(`RouteDecision::Replay`/`InProgress`'s own `original_operation_id` field), and
a retry's own freshly-generated `operation_id` is never the right key for
status queries, replay lookups, or session-lifecycle tracking.

`RemoteDispatchOutcome::original_operation_id(&self, current: &OperationId) ->
&OperationId` (added to `remote_node.rs`) resolves this uniformly: `Sent`
returns `current` (the caller's own id, since that request is what established
the lease), while `Replayed`/`QueryRemoteState` return their carried field.
`work_executor.rs`'s `dispatch_host_resident`/`dispatch_gateway_backed` both
use this to compute the id they key session/status state by, and both expose
it on their respective `WorkDispatchOutcome` variants so external callers
(status endpoints, audit correlation) key their own state the same way —
**never under a retry's own `operation_id`**.

Dependency direction is preserved: `work_executor.rs` imports only
`crate::{...}` (router's own re-exports, including `crate::remote_node`'s
`GatewayPort`/error/outcome types) plus `d2b_realm_core`/`serde`/`serde_json`
(already direct dependencies). It adds no transport or codec dependency to
production code — a `RemotePeerClient` trait object is a caller-supplied byte
transport, not a concrete `d2b-realm-transport` type.

## `GatewayPort` / `SingleGatewayPort` (`remote_node.rs`)

The old design embedded a `RemoteFullHostAdapter<C>` (which itself owns a
`RemoteNodeRegistry`) directly inside `WorkExecutor` — a host-resident type
holding remote-node registry state, which is exactly what ADR 0032 says must
stay gateway-side, never in `d2bd`/host code. This pass removes that field
entirely and introduces:

- **`pub trait GatewayPort: Send`** — `fn dispatch_via_gateway(&mut self,
  gateway: &RealmTarget, req: &OperationRequest, generation: &ProtocolToken,
  client: &mut dyn RemotePeerClient) -> Result<RemoteDispatchOutcome,
  RemoteNodeError>`. Implementations MUST refuse (fail closed) a `gateway`
  that does not match whatever boundary they were constructed to front. This
  trait lives in `remote_node.rs` (unconditionally compiled) rather than
  `work_executor.rs` (test-gated today), so it is reachable regardless of
  which crate declares which `mod`.
- **`RemoteDispatchOutcome::Replayed`/`::QueryRemoteState` now carry an
  `original_operation_id: OperationId` field**, and the enum gained
  `original_operation_id(&self, current: &OperationId) -> &OperationId` (see
  [Original-operation-id propagation](#original-operation-id-propagation)).
  `RemoteFullHostAdapter::dispatch()` populates this field from the router's
  own `RouteDecision::Replay`/`InProgress` — it is not independently derived,
  so it is exactly the router's notion of "the first attempt", never this
  attempt's own id.
- **`SingleGatewayPort<C = SystemClock>`** — the reference implementer:
  wraps exactly one gateway-side `RemoteFullHostAdapter<C>` plus a
  `boundary: RealmTarget`. `dispatch_via_gateway` refuses
  (`RemoteNodeErrorKind::UnauthorizedGateway`, fail-closed) any `gateway` that
  does not equal `boundary`, and otherwise delegates unchanged to
  `adapter.dispatch(...)`. This type is documented, and meant, to run
  **inside the gateway guest process** it fronts — never embedded in a
  host-resident `WorkExecutor` — so `RemoteFullHostAdapter` (and the
  `RemoteNodeRegistry` it wraps) stays exactly where ADR 0032 requires it.

The host process passes only the realm entrypoint table's already-resolved
canonical `gateway`/`target` `RealmTarget`s across this boundary; it never
constructs, owns, or reaches into gateway-side registry/router state
directly. See [Integrator wiring](#integrator-wiring) for how a real
constellation daemon is expected to place the two halves in separate
processes.

## `TransportFabric` (transport)

`TransportFabric` is itself just another `TransportProvider` impl: a
scheme-keyed composition of already-existing transports
(`crate::LoopbackTransport`, `crate::LocalTcpTransport`, or any future
`TransportProvider`), keyed by a bounded, validated scheme parsed from
`TransportTarget::endpoint`.

- **Scheme grammar** (`FabricScheme::parse`): `ALPHA *( ALPHA / DIGIT / "+" /
  "-" / "." )`, bounded to `MAX_FABRIC_SCHEME_LEN` (32) chars, case-insensitive
  (stored lowercased) — close to RFC 3986 §3.1's URI scheme production, chosen
  so it accepts the crate's own `"loopback"` and `"tcp+local"` literals.
- **`register(scheme, transport)`**: bounded to `MAX_FABRIC_TRANSPORTS` (16)
  entries; rejects a duplicate scheme (`FabricError::DuplicateScheme`) or an
  invalid scheme literal (`FabricError::InvalidScheme`) fail-closed.
- **`connect()`**: parses the scheme prefix (substring before the first
  `"://"`, or the whole endpoint when there is none — the shape
  `LoopbackTransport`'s bare `"loopback"` target uses) and dispatches to the
  registered transport. An unregistered scheme fails closed with
  `d2b_realm_core::ErrorKind::InvalidTarget` — there is no default transport.
- **`listen()`**: fans out to every registered transport's own `listen()`
  call. A single transport's `listen()` call failing (for example, a
  single-use provider whose listener side was already consumed by an earlier
  `listen()` call) does **not** fail the whole fan-out: the healthy subset is
  kept and only surfaced as an error once *no* registered transport was able
  to start listening.
- **`FabricListener::accept()`**: fans **in** every healthy sub-listener via
  a persistent per-listener background task (one `tokio::spawn` per
  sub-listener) that loops calling that sub-listener's own `accept()`.
  Each `Err` outcome is classified via
  `d2b_realm_provider::error::ProviderError::retry_hint()`: when a hint is
  present, the error is a known-recoverable accept-stage condition (see
  [Recoverable accept-error classification](#recoverable-accept-error-classification-local_tcprs)
  below) and the task sleeps for the hint's bounded `applied_backoff()`
  (floored at `FABRIC_ACCEPT_MIN_BACKOFF`) and retries in place — the
  transient error is *never* forwarded into the channel — up to
  `FABRIC_ACCEPT_MAX_CONSECUTIVE_RECOVERABLE_ERRORS` (32) consecutive
  recoverable errors, after which the run falls back to terminal handling so
  a sub-listener that is recoverable-but-never-actually-recovers cannot spin
  its background task forever. A terminal error (no hint, or the bound
  exhausted) and every `Ok` outcome are forwarded into one shared, bounded
  `tokio::sync::mpsc` channel (`FABRIC_ACCEPT_QUEUE_CAPACITY`, 64).
  `accept()` pulls from that channel, skipping over (but remembering)
  terminal sub-listener errors so it keeps waiting on whichever sub-listeners
  are still healthy, and only returns an error once every sub-listener has
  gone terminal and the channel has drained and closed. A background task
  stops looping (dropping its sender) once its own sub-listener goes
  terminal, so a permanently dead transport cannot spin forever flooding the
  queue. Dropping the `FabricListener` `abort()`s every spawned task —
  bounded, explicit cancellation, no leaked accept loops, including one
  currently asleep in a retry backoff.

This replaces the previous one-shot `tokio::task::JoinSet` race, which called
`abort_all()` on the first accepted session — silently discarding any *other*
session simultaneously accepted on a sibling listener. The bounded channel
instead queues (with backpressure via `send().await`, never `try_send`) every
simultaneously accepted session so a later `accept()` call still delivers it.

`TransportFabric` holds no realm relay/session/provider credential and no
remote node registry: it is strictly a byte-transport composition. It carries
no free-form path/argv construction.

### Recoverable accept-error classification (`local_tcp.rs`)

`LocalTcpListener::accept()`'s io errors are mapped to a typed
`ProviderError` by `local_tcp_io_error(stage, kind)`. For the `"accept"`
stage specifically, `is_recoverable_accept_error(kind)` classifies
`Interrupted`/`ConnectionAborted`/`WouldBlock`/`TimedOut` as recoverable —
well-known transient accept conditions that do not indicate the listening
socket itself has failed — and attaches a bounded
`RetryHint::bounded(ACCEPT_RETRY_BASE, Duration::ZERO, ACCEPT_RETRY_MAX)`
(5ms base, 50ms cap). Every other io error kind, and every non-`"accept"`
stage (e.g. `"bind"`), carries no retry hint and is therefore always
terminal. This keeps the recoverable/terminal decision entirely inside the
structural, transport-agnostic `retry_hint()` signal `fabric.rs` already
consumes, rather than leaking `std::io::ErrorKind` matching into the
fan-in's own classification logic.

### Scheme case normalization

`FabricScheme::parse`/`from_endpoint` have always lowercased for scheme
matching, but `local_tcp.rs`'s own `parse_target()` stripped its
`"tcp+local://"` prefix with a case-sensitive `strip_prefix`. A mixed-case
endpoint (e.g. `"TcP+LoCaL://127.0.0.1:5000"`) would therefore be routed
correctly by the fabric to `LocalTcpTransport` and then fail inside
`parse_target` because it could not strip the differently-cased prefix.
`parse_target` now checks/strips its scheme prefix case-insensitively
(`eq_ignore_ascii_case`), matching the fabric's own normalization, with a
`mixed_case_scheme_endpoint_round_trips_through_fabric_and_local_tcp`
regression test exercising the full fabric→local-tcp path end to end.

## `execution::state_code` / `service_mode::ExecutionOutcomeCode`

Router and guest-runner cannot depend on each other (guest-runner is
dependency-pure; router does not, and must not, depend on
`d2b-exec-runner`). To let an external observer (audit, metrics, CLI status)
correlate a router-side `ExecState` with a guest-runner `StatusPhase` without
either crate importing the other's types, both sides expose an identical
small lowercase-ASCII string vocabulary:

- `d2b_realm_router::execution::state_code(ExecState) -> &'static str`:
  `"pending"`, `"running"`, `"exited"`, `"cancelled"`, `"failed"`.
- `d2b_exec_runner::service_mode::outcome_code_for_phase(StatusPhase) ->
  ExecutionOutcomeCode`, whose `.code()` yields the terminal-relevant subset:
  `"running"`, `"exited"`, `"cancelled"`, `"failed"` (a runner has no
  `"pending"` phase of its own; `Signaled` collapses into `"exited"`;
  `SpawnFailed`/`InfraFailed` both collapse into `"failed"`, matching the
  router's single terminal failure state).

Both functions are pure string mappings with unit tests asserting: full state
coverage, pairwise distinctness (router side), and — on the runner side — that
every emitted code is a member of the router's vocabulary (`spec.rs`/
`service_mode.rs` pin the router's literal code set inline rather than
importing it, keeping the crate dependency-pure). Keep the two vocabularies in
lockstep if either changes.

## `spec::validate_workload_unit_name`

`d2b-guestd`'s `workload_unit_name(slot)` (in `d2b-guestd/src/detached.rs`,
not touched by this change) is the single writer/deriver of the canonical
`d2b-exec-<NN>-w.service` unit name. `d2b-exec-runner/src/spec.rs` already
owned the reader-side shape validator; this change only makes it `pub` (no
behavior change) so other in-repo callers can reuse the identical bounded
shape check (`d2b-exec-` prefix, `-w.service` suffix, ASCII
alphanumeric/`-`/`.` only, no path separators, bounded length) instead of
hand-rolling a second, possibly-diverging one. It intentionally does **not**
add a second name-deriving function: the derivation stays single-owned in
`d2b-guestd`, only the validation contract is shared.

## Test-only `#[cfg(test)]` module declarations — not production wiring

`lib.rs` is a shared integration sink outside this component's owned files,
so this change cannot add `pub mod work_executor;` to
`packages/d2b-realm-router/src/lib.rs`, nor `pub mod fabric;` to
`packages/d2b-realm-transport/src/lib.rs`. Left with no `mod` declaration at
all, `work_executor.rs`/`fabric.rs` would be dead files: never compiled by
`cargo build`/`cargo test`, and any claim that "the tests pass" would
silently rest on a temporarily-patched `lib.rs` instead of the committed
tree.

**This is a test-harness convenience, not a substitute for production
integration, and it must not be read as such.** To make the committed tree
self-verifying under `cargo test` without touching `lib.rs`, each new file is
nested, **conditionally, and only when compiling this crate's own tests**,
inside a sibling file that `lib.rs` already declares unconditionally:

- `packages/d2b-realm-router/src/execution.rs` (already `pub mod execution;`
  in `lib.rs`) adds:

  ```rust
  #[cfg(test)]
  #[path = "work_executor.rs"]
  mod work_executor;
  ```

- `packages/d2b-realm-transport/src/local_tcp.rs` (already `mod local_tcp;`
  in `lib.rs`) adds:

  ```rust
  #[cfg(test)]
  #[path = "fabric.rs"]
  mod fabric;
  ```

Both declarations disappear entirely from a non-test build (`#[cfg(test)]`),
so the production compiled surface of either crate is byte-for-byte
unaffected by this change; `cargo build` for either crate does not compile
`work_executor.rs`/`fabric.rs` at all today, and **neither type is reachable
from any production code path in this tree** — there is no running control
plane, daemon, or CLI surface that exercises `WorkExecutor`/`TransportFabric`
until an integrator performs the wiring below. `cargo test`, however, does
compile and run their own `#[cfg(test)] mod tests` — this is what makes every
test result reported in [Validation performed](#validation-performed) a real
exercise of the committed tree, not of a temporarily-patched one. Both files
exclusively use absolute `crate::`-qualified paths (never `super::`) for
every cross-reference, so nesting them under a different parent module in
test builds changes nothing about their own logic.

This is a conditional, test-only reference from an already-declared module to
an otherwise-undeclared one — it satisfies "avoid unconditional references
from declared modules to undeclared modules" while still letting the owned
files compile and their tests run today. It is deliberately narrow scaffolding
scoped to `#[cfg(test)]` alone, expected to be short-lived: **the integration
commit that adds the real, unconditional `pub mod work_executor;`/`pub mod
fabric;` lines to each crate's `lib.rs` (documented next, in
[Integrator wiring](#integrator-wiring)) MUST also delete these two
`#[cfg(test)]` blocks from `execution.rs`/`local_tcp.rs` in the same commit.**
Once the production `pub mod` line exists, the conditional test-only path
declaration is redundant (the file is already reachable unconditionally) and
leaving it in place would needlessly compile `work_executor.rs`/`fabric.rs`
twice under two different module paths for `cargo test`, which is exactly
the kind of stale test-only scaffolding this note exists to prevent.

## Regression: unique scratch allocation in `service_mode.rs` tests

Exact-head W9 CI exposed a pre-existing parallel-test race in this owned
file's `#[cfg(test)]` helper `scratch_slot()`, unrelated to the new
`ExecutionOutcomeCode`/`outcome_code_for_phase` additions above but fixed as
part of this component's ownership of `service_mode.rs`.

**Symptom:** `cancel_sentinel_terminates_and_records_cancelled` intermittently
failed under parallel CI test execution with a missing/unexpected status
file.

**Root cause:** `scratch_slot()` named each test's scratch dir
`runner-svc-<pid>-<nanos>` (process id + `SystemTime::now()` nanoseconds) and
created it with `create_dir_all`, which succeeds silently when the directory
already exists. Two test threads running in the *same test binary process*
(same pid) can observe the same nanosecond tick on a coarse clock, especially
under parallel scheduling pressure — so both calls could resolve to the same
physical directory, race to write their own `status`/log files into it, and
stomp on each other. The failure was purely a test-harness-scratch-allocation
bug: no production `RunnerPaths`/`service_mode` behavior was at fault.

**Fix:** `scratch_slot()` now combines a per-process, monotonically
incrementing `AtomicU64` sequence number with the timestamp, and allocates
the top-level scratch dir with `create_dir` (not `create_dir_all`), so a
collision is observable (`ErrorKind::AlreadyExists`) instead of silently
tolerated. On collision the loop draws a fresh sequence number and retries;
because the counter only ever increases, every retry is guaranteed to produce
a name no earlier attempt (in this process) could already hold, so the loop
always makes forward progress. A bounded attempt ceiling
(`SCRATCH_SLOT_MAX_ATTEMPTS`) keeps the fallback fail-closed — a hard panic
naming the runaway condition — instead of spinning forever if the temp dir
is unwritable or otherwise adversarial.

A regression test, `scratch_slot_is_unique_under_concurrent_same_process_allocation`,
spawns 64 threads behind a `std::sync::Barrier` so they all call
`scratch_slot()` as close to simultaneously as possible (reproducing the
same-process, same-tick contention that caused the original race) and
asserts every returned directory is both created and pairwise distinct.

This fix is test-only (confined to the `#[cfg(test)]` module of an owned
file); it changes no production type, trait, or public API surface of
`d2b-exec-runner`. W9's integration wave is expected to carry an identical
narrow fix to the same helper independently before its own panel; when
reconciling the two branches, the fix is idempotent to re-apply (a
byte-identical `scratch_slot()` body from either branch satisfies both), so
whichever version lands first should be kept as-is rather than merged
field-by-field.

## Integrator wiring

None of the new modules are declared in their crate's `lib.rs` by this
change (`lib.rs` is a shared integration sink outside this component's owned
files). To bring them into the compiled production surface, an integrator
adds exactly the following, **and, in the same commit, deletes the two
`#[cfg(test)]` test-only path-module declarations described in
[Test-only `#[cfg(test)]` module declarations](#test-only-cfgtest-module-declarations--not-production-wiring)
from `execution.rs`/`local_tcp.rs`** — once the unconditional `pub mod` lines
below exist, those conditional test-only declarations are redundant
duplicate-compilation scaffolding, not additional production wiring:

`packages/d2b-realm-transport/src/lib.rs`:

```rust
pub mod fabric;
pub use fabric::{
    FABRIC_ACCEPT_QUEUE_CAPACITY, FabricError, FabricScheme, MAX_FABRIC_SCHEME_LEN,
    MAX_FABRIC_TRANSPORTS, TransportFabric,
};
```

`packages/d2b-realm-router/src/lib.rs`:

```rust
pub mod work_executor;
pub use work_executor::{
    DEFAULT_MAX_GATEWAY_SESSIONS, HostResidentOutcome, WorkDispatchOutcome, WorkExecutor,
    WorkExecutorError,
};
```

`packages/d2b-realm-router/src/remote_node.rs` already declares `GatewayPort`
and `SingleGatewayPort` as `pub` items of an already-`pub mod remote_node;`
file, so no additional `lib.rs` line is needed to reach them — they are
already part of the crate's compiled public surface today, independent of
the `work_executor` wiring above.

**Constructing a `GatewayPort` for a real deployment**: a `WorkExecutor`
running host-side never constructs a `SingleGatewayPort` (or any
`RemoteFullHostAdapter`) itself. Instead:

1. The gateway guest process owns exactly one `SingleGatewayPort` (built from
   its own `RemoteFullHostAdapter`, its `RemoteNodeRegistry`, and the
   `RealmTarget` boundary it fronts).
2. Whatever session/transport plumbing already carries operations from the
   host-resident `d2bd`/constellation daemon to that gateway guest (a
   realm-scoped session per ADR 0032/0045, not a new credential this
   component introduces) is responsible for presenting a `&mut dyn
   GatewayPort` handle to the host-side `WorkExecutor::dispatch` call —
   either by running the call itself inside the gateway process against its
   local `SingleGatewayPort`, or by wrapping that session boundary in a small
   adapter type (not part of this component) that forwards
   `dispatch_via_gateway` calls across it and implements `GatewayPort` on the
   host side. This component defines the trait and one reference
   implementer; it deliberately does not prescribe or implement that
   session-crossing adapter, since doing so would require touching the
   realm session/transport wiring outside this component's owned files.

No `Cargo.toml`/`Cargo.lock`/workspace manifest change is required for either
crate: both new modules use only dependencies already declared
(`tokio` "rt"/"sync", `serde`/`serde_json`, `async-trait`, `d2b-realm-core`,
`d2b-realm-provider` — all pre-existing direct dependencies of the crate that
gained the new module). `service_mode.rs`'s and `spec.rs`'s additions in
`d2b-exec-runner` need no wiring at all: `spec.rs` is already `pub mod spec;`
in `d2b-exec-runner/src/lib.rs`, and `service_mode.rs` is already `mod
service_mode;` in `d2b-exec-runner/src/main.rs` (both pre-existing
declarations, unmodified by this change).

## Validation performed

All three crates build and `cargo test` / `cargo clippy --all-targets -- -D
warnings` / `cargo fmt --check` are clean **against the committed tree as
it stands** — no `lib.rs` patch was needed or used to produce these results,
because `work_executor.rs`/`fabric.rs` are reached through the test-only
`#[cfg(test)]` module declarations described above. Test counts observed:
`d2b-realm-router` 102 lib tests (including 23 in
`execution::work_executor::tests` covering wrong-node fail-closed, empty
capabilities/missing idempotency rejection, workload-mismatch rejection,
replay-vs-restart, gateway-port boundary-mismatch fail-closed,
original-operation-id propagation through replay/in-progress, reserve-before-
dispatch capacity enforcement (proving the peer is never called when at
capacity), and non-destructive eviction of a legitimately running session
under a failed/conflicting retry); `d2b-realm-transport` 38 lib tests
(including persistent-fan-in, partial-listen-failure, all-listen-failure,
mixed-case-scheme, recoverable-accept-error classification/retry-hint
attachment, scripted-transient-error retry-without-terminating, bounded
recoverable-error-exhaustion-goes-terminal, and drop-aborts-a-task-asleep-in-
backoff regressions); `d2b-exec-runner` 34 lib + 22 bin + 2 + 4 integration
tests. `d2b-exec-runner`'s test suite (including
`scratch_slot_is_unique_under_concurrent_same_process_allocation` and
`cancel_sentinel_terminates_and_records_cancelled`) was additionally run
five consecutive times with `--test-threads=16` with no failures.
