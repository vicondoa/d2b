# OtelHostBridge readiness gate

Status: **P3 `ph3-p3-otelbridge-readiness`** — implemented.

## What this gate is

The P1 work folded the legacy `nixling-otel-host-bridge.service`
host singleton into a broker-`SpawnRunner` lifecycle under
`RunnerRole::OtelHostBridge`. The broker can now spawn the
forwarder per the trusted bundle's intent, and `pidfd_table`
tracks its liveness — but until this gate landed there was no
*formal readiness* signal the daemon could block on before
declaring an observability VM "ready".

Without a readiness gate the daemon would report `overall_ok=true`
for the obs VM the moment the per-VM process DAG settled, even if
the host-side OTLP forwarder was still mid-handshake (or had
silently failed to bind its vsock host socket). Operators then
saw mysterious gaps in Grafana / Tempo / Loki for the first few
seconds of every obs VM boot.

This page documents the typed gate that closes that window.

## When the gate fires

The gate is evaluated by `dispatch_broker_vm_start` *after* the
per-VM process DAG returns `overall_ok=true`, but only when both
of the following are true for the VM being started:

1. `manifest._observability.enabled == true` — the operator has
   opted into observability for this site.
2. `request.vm == manifest._observability.vmName` — the VM being
   started IS the observability VM that the OtelHostBridge relays
   into.

Workload VMs short-circuit (the OtelHostBridge isn't on their
critical path). Observability-disabled sites also short-circuit.

## Readiness predicate

The gate is satisfied when **both** of these are true at the same
time:

- The `RunnerRole::OtelHostBridge` runner is registered in the
  daemon's `pidfd_table` for the obs VM (proves the broker
  successfully spawned it and pidfd-tracked it).
- The obs vsock host socket file
  (`_observability.obsVsockHostSocket`) exists on disk (proves
  the runner has called `bind(2)` + `listen(2)` and is ready to
  `accept(2)` OTLP from workload-VM clients).

The two signals together are the side-effect-free proxy for
"socket accept succeeded + first OTLP forward acknowledged". A
formal `sd_notify READY=1` channel from broker-spawned runners
to the daemon is a later-phase deliverable; when it lands, this
gate will additionally require that READY=1 has been received
before declaring `Ready`. The current predicate is conservative
in the failure direction (it can transiently report `Pending`
while the runner's socket is mid-bind) and never falsely
positive.

## Timeout + degraded-mode contract

Default timeout: **30 000 ms** (configurable via the
`NIXLING_OTEL_BRIDGE_READINESS_TIMEOUT_MS` env var, parsed as
unsigned milliseconds).

Polling cadence: **100 ms** between samples.

On timeout the daemon falls back to **degraded mode**:

- The VM is left running. cloud-hypervisor + virtiofsd + swtpm
  have already accepted the boot at this point; tearing them
  down again would just hide a transient observability issue
  behind a much larger failure surface.
- The successful `vm start` response is returned to the client
  with a structured `tracing::warn!` annotation in the daemon
  logs (`elapsed_ms`, `reason`, `vm`).
- The typed error
  [`TypedError::OtelHostBridgeReadinessTimeout { vm, elapsed_ms }`](./error-codes.md#otel-host-bridge-readiness-timeout)
  (exit code **65**) is the canonical kind operators can
  reference from metrics, audit, and `nixling host doctor`.

If the runner exit marker indicates the broker-spawned runner
died before readiness, the gate short-circuits the deadline and
returns the same degraded-mode outcome with `reason = "runner
exited before readiness signal"`.

### Strict mode

Operators who want a hard refusal instead of degrading can set:

```
NIXLING_OTEL_BRIDGE_READINESS_STRICT=1
```

In strict mode the timeout (or runner-exit) outcome is returned
to the client as the typed
`otel-host-bridge-readiness-timeout` envelope (exit code 65)
instead of a successful-with-warning response. Strict mode is
appropriate for ops sites where observability is a hard
prerequisite and a broken forwarder MUST surface as a failed VM
start.

## Remediation

- Inspect `nixling host doctor` (P3
  `ph3-p3-host-doctor-extended`) for the OtelHostBridge runner's
  pidfd liveness + last-relay-flush timestamp.
- If the runner is missing entirely, the broker `SpawnRunner` for
  `RunnerRole::OtelHostBridge` failed — inspect the broker audit
  log.
- If the vsock host socket does not exist, the obs VM is not
  accepting OTLP from workload VMs; restarting the obs VM
  usually clears the condition.
- To raise the deadline, set
  `NIXLING_OTEL_BRIDGE_READINESS_TIMEOUT_MS=<ms>`.
- To fail-closed instead of degrading, set
  `NIXLING_OTEL_BRIDGE_READINESS_STRICT=1`.

## Module layout

The gate ships in `packages/nixlingd/src/otel_host_bridge_readiness.rs`:

- `enum OtelHostBridgeReadiness` — pure verdict
  (`Ready` / `Pending { elapsed_ms }` / `Failed { reason }`).
- `struct ReadinessProbe` — pure inputs (booleans + elapsed +
  deadline).
- `fn evaluate_readiness(&ReadinessProbe) -> OtelHostBridgeReadiness`
  — pure evaluator; no I/O.
- `trait OtelHostBridgeProbeSource` — read-only injection point
  for the side-effecting wrapper.
- `struct PidfdAndSocketProbeSource` — production implementation
  backed by `PidfdTable` + filesystem `stat(2)`.
- `fn await_otel_host_bridge_readiness(...) -> ReadinessWaitOutcome`
  — side-effecting wrapper. Loops `evaluate_readiness` until
  `Ready` or `Failed`; takes an injected `sleep` callback for
  deterministic testing.
- `struct ReadinessWaitConfig { timeout, poll_interval, strict }`
  — populated by `ReadinessWaitConfig::from_env()`.

Twelve unit tests cover the pure evaluator (truth-table over all
four inputs), the env-var parser (default fallback on garbage
inputs, strict-mode flag), and the side-effecting wrapper
(eventual-ready, timeout-to-degraded, runner-exit-to-degraded).
