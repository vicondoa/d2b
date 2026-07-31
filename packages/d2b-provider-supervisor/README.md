# `d2b-provider-supervisor`

The fixed core-owned Process effect adapter. It is not a configurable Provider
resource and cannot be replaced by a third-party artifact.

## Provider identity

Fixed `provider-supervisor` bootstrap component. It has no `providerRef` because
it is the trusted adapter between Process Provider decisions and local effect
owners.

## Config schema

No public config schema. Construction accepts a core-owned backend, a bounded
blocking concurrency limit, and a fallback timeout. Broker and service-manager
details remain private to their effect owners.

## Exported resource types

None. It implements `ProcessLaunchEffectPort` for the Process Providers but
does not own `Process` or `EphemeralProcess` resources.

## Controllers / services / workers / binaries

`ProviderSupervisor` is an in-process fixed adapter, not a controller, service,
worker, or binary. `BrokerProcessBackend` dispatches existing broker runner
roles; `SystemdProcessBackend` wraps a core-owned system or user manager.

## Placement and dependencies

Runs beside the local Host or Guest process supervisor. It depends on
`d2b-process`, `d2b-process-conformance`, the broker wire contract, and an
injected trusted launch resolver or service-manager effect owner. The crate does
not depend on `d2bd` or the privileged broker implementation.

## RBAC requirements

Only a Process-controller-authenticated `LaunchTicket` enters the adapter.
Authorization, controller lease, resource revision, endpoint policy, and trusted
bundle installation are validated before this boundary.

## Security posture

The broker backend sends only opaque `SpawnRunner`, `OpenPidfd`, and
`SignalRunner` requests and retains descriptors received through `SCM_RIGHTS`.
The broker remains the sole privileged executor and audit owner, including
user-namespace setup and final cgroup placement. The systemd backend accepts
only an atomic invocation, cgroup, main-process, start-time, and
Provider/template/generation identity and rechecks the entire tuple after
opening a fresh descriptor. All diagnostics are value-free and redacted.

## State and telemetry

Local descriptors and verified observations exist only in memory and are not
serialized or exposed through status. Errors map to a closed code set; metric
labels must use only those codes and closed operation names.

## Build and test

```bash
cd packages && cargo test -p d2b-provider-supervisor
cd packages && cargo clippy -p d2b-provider-supervisor --all-targets
```

The hermetic suite drives both existing Process Providers through the production
adapter over deterministic core-owned backends. The declared `integration/`
scenarios separately name the container and booted-host evidence still required
for real broker, sandbox, cgroup, systemd, pidfd, and wait/reap behavior.
