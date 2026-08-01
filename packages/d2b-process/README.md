# `d2b-process`

Neutral process launch and supervision primitives shared by Process Providers
and the core-owned ProviderSupervisor effect adapter.

## Provider identity

This is a core contract crate, not a Provider. It declares no Provider identity.

## Config schema

`ProcessRequest` wraps the validated `LaunchTicket`; `BackendObservation` and
`BackendLaunch` carry only opaque identity, closed verified bindings, wait/reap
ownership, and core-local authority. No separate configuration schema is
exported.

## Exported resource types

None. `Process` and `EphemeralProcess` remain owned by their selected Process
Provider.

## Controllers / services / workers / binaries

No controller, service, worker, or binary. The crate exports the synchronous
`ProcessEffectBackend` contract consumed behind the supervisor's bounded async
adapter.

## Placement and dependencies

Core-owned local effect adapters use this crate on a Host or Guest. Provider
controllers depend on the neutral `d2b-process-conformance` surface and never
receive the backend handle.

## RBAC requirements

No RBAC grant is accepted here. Authorization, current-resource checks, and
ticket issuance precede construction of `ProcessRequest`.

## Security posture

A Provider performs no privileged mutation and reaches host state only through
the injected typed effect port. Backend errors are closed value-free codes;
request, observation, launch, identity, and local descriptor diagnostics are
redacted. Local process authority is neither cloneable nor serializable.

## State and telemetry

The crate owns no persistent state and emits no telemetry. The consuming
supervisor retains local handles in memory and reopens them only after stable
identity verification.

## Build and test

```bash
cd packages && cargo test -p d2b-process
cd packages && cargo clippy -p d2b-process --all-targets
```

Real broker, systemd, cgroup, and process-lifecycle scenarios belong to the
declared fixtures under `integration/` and require their named integration tier.
