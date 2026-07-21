# Realm routing / work-executor / transport fabric

**Diataxis category:** reference.

This page documents the W8 `realm-routing-work-executor-fabric` component: one
coherent typed dispatch surface spanning `d2b-realm-router`,
`d2b-realm-transport`, and `d2b-exec-runner`. It composes only already-owned,
already-tested state in those crates (`RealmEntrypointTable`,
`DurableExecTable`, `RemoteFullHostAdapter`, `SessionLifecycle`,
`LoopbackTransport`, `LocalTcpTransport`) and introduces no new realm relay,
session, or provider credential; no remote node registry outside the existing
`RemoteNodeRegistry`; and no free-form path/argv construction. It preserves
[ADR 0032](./../adr/0032-d2b-v2-constellation-control-plane.md) and
[ADR 0045](./../adr/0045-provider-and-transport-framework.md): relay identity
is never mapped to local admin authority, host code holds no realm relay
credential, and the allocator surface (`allocator.rs`/`allocator_engine.rs`)
is consumed as-is, never edited.

None of this is wired into a running control plane by this change. Every new
module is written so that adding it costs an integrator only the `mod`/
`pub use` lines documented in [Integrator wiring](#integrator-wiring) below —
no other file needs to change, no `Cargo.toml` needs a new dependency, and no
existing type's shape changes.

## Component map

| File | Role |
| --- | --- |
| `packages/d2b-realm-router/src/work_executor.rs` (new) | `WorkExecutor`: the single typed dispatch entry point tying realm resolution, host-resident durable execution, gateway-backed remote dispatch, and gateway session-lifecycle tracking together. |
| `packages/d2b-realm-router/src/execution.rs` (touched) | Adds `state_code(ExecState) -> &'static str`: a stable, router-side observability vocabulary for `ExecState`. |
| `packages/d2b-realm-router/src/target_resolver.rs`, `remote_node.rs`, `session_lifecycle.rs` | Consumed as-is by `WorkExecutor`; not modified. Owned only because they define the composed types. |
| `packages/d2b-realm-transport/src/fabric.rs` (new) | `TransportFabric`: a scheme-keyed composition of `TransportProvider` impls (e.g. `LoopbackTransport`, `LocalTcpTransport`) behind one `TransportProvider` facade. |
| `packages/d2b-realm-transport/src/local_tcp.rs` (touched) | Adds `LOCAL_TCP_SCHEME_NAME`: the public scheme literal a fabric registers `LocalTcpTransport` under, avoiding a duplicated string literal at the call site. |
| `packages/d2b-exec-runner/src/service_mode.rs` (touched) | Adds `ExecutionOutcomeCode` + `outcome_code_for_phase(StatusPhase)`: the guest-runner-side half of the same stable-vocabulary contract as `execution::state_code`. |
| `packages/d2b-exec-runner/src/spec.rs` (touched) | Makes `validate_workload_unit_name` `pub`: a reusable shape-validator for the slot-derived workload unit name `d2b-guestd` writes, without duplicating its derivation. |

## `WorkExecutor` (router)

`WorkExecutor<C: Clock = SystemClock>` composes:

- `RealmEntrypointTable` — resolves an `OperationRequest`'s `RealmTarget` to
  `DispatchTarget::HostResident` or `DispatchTarget::GatewayBacked`.
- `DurableExecTable` — host-resident metadata for the exec family
  (`ExecStart`/`ExecAttach`/`ExecLogs`/`ExecCancel`).
- `RemoteFullHostAdapter<C>` — the existing gateway-side remote dispatch path
  (codec/transport-neutral: callers supply a `RemotePeerClient` object).
- A bounded `HashMap<OperationId, SessionLifecycle>` for gateway-backed
  `DisplaySessionOpen` operations only.

`WorkExecutor::dispatch(&mut self, req, generation, client)` is the one entry
point:

1. Resolve `req`'s `RealmTarget` via the entrypoint table.
2. `HostResident` → decode the operation body (`serde_json`, already a direct
   `d2b-realm-router` dependency) to the exec-family request shape its
   `OperationKind` requires, then call the matching `DurableExecTable` method.
   Returns `WorkDispatchOutcome::HostResident(HostResidentOutcome)`.
3. `GatewayBacked` → delegate to `RemoteFullHostAdapter::dispatch()`
   unchanged. For `DisplaySessionOpen` only, additionally advance a tracked
   `SessionLifecycle` for the operation id (bounded by
   `DEFAULT_MAX_GATEWAY_SESSIONS`, override with
   `WorkExecutor::with_max_gateway_sessions`). Every other gateway-backed
   operation kind carries `session_phase: None` — the lifecycle models
   workload/display session establishment, not generic exec.

`WorkExecutorError` distinguishes resolution failure, malformed body,
unsupported host-resident operation kind, durable-table rejection, remote
adapter rejection, and gateway session-table capacity. `stop_gateway_session`
drives orderly teardown (`stop()` + `finish_stop()`), evicting the session
once it reaches `Stopped`.

Dependency direction is preserved: `work_executor.rs` imports only
`crate::{...}` (router's own re-exports) plus `d2b_realm_core`/`serde_json`
(already direct dependencies). It adds no transport or codec dependency to
production code — a `RemotePeerClient` trait object is a caller-supplied byte
transport, not a concrete `d2b-realm-transport` type.

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
- **`listen()`**: fans out to every registered transport and returns one
  `FabricListener` whose `accept()` races every sub-listener's `accept()` via
  a bounded `tokio::task::JoinSet` (the "rt" tokio feature, already enabled by
  this crate — no new dependency) and resolves to the first session accepted
  on any of them. A sub-listener error does not fail the whole fan-out — the
  race keeps waiting on the rest and only surfaces an error once every
  registered transport has failed. On success, every other in-flight accept
  task is aborted (`JoinSet::abort_all`) — bounded, explicit cancellation, no
  leaked background accept loops.

`TransportFabric` holds no realm relay/session/provider credential and no
remote node registry: it is strictly a byte-transport composition. It carries
no free-form path/argv construction.

## Integrator wiring

None of the new modules are declared in their crate's `lib.rs` by this
change (`lib.rs` is a shared integration sink outside this component's owned
files). To bring them into the compiled surface, an integrator adds exactly
the following, and nothing else:

`packages/d2b-realm-transport/src/lib.rs`:

```rust
pub mod fabric;
pub use fabric::{FabricError, FabricScheme, MAX_FABRIC_SCHEME_LEN, MAX_FABRIC_TRANSPORTS, TransportFabric};
```

`packages/d2b-realm-router/src/lib.rs`:

```rust
pub mod work_executor;
pub use work_executor::{
    DEFAULT_MAX_GATEWAY_SESSIONS, HostResidentOutcome, WorkDispatchOutcome, WorkExecutor,
    WorkExecutorError,
};
```

No `Cargo.toml`/`Cargo.lock`/workspace manifest change is required for either
crate: both new modules use only dependencies already declared
(`tokio` "rt", `serde_json`, `async-trait`, `d2b-realm-core`,
`d2b-realm-provider` — all pre-existing direct dependencies of the crate that
gained the new module). `service_mode.rs`'s and `spec.rs`'s additions in
`d2b-exec-runner` need no wiring at all: `spec.rs` is already `pub mod spec;`
in `d2b-exec-runner/src/lib.rs`, and `service_mode.rs` is already `mod
service_mode;` in `d2b-exec-runner/src/main.rs` (both pre-existing
declarations, unmodified by this change).

## Validation performed

All three crates build, and `cargo test` / `cargo clippy --all-targets` /
`cargo fmt --check` are clean, with the two `lib.rs` wiring stanzas above
applied only transiently in a local working copy to prove the modules compile
and their tests pass end to end; the committed tree does not include that
`lib.rs` change (per the owned-files boundary for this component).
