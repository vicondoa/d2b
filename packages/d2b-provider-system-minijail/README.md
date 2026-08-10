# `d2b-provider-system-minijail`

The `system-minijail` Process Provider controller.

See [Create a Provider](../../docs/how-to/create-provider.md) for the
uniform crate layout, schema links, configuration, and test lanes.

## Provider identity

`system-minijail`, referenced as `Provider/system-minijail`. It is the
second of the two fixed, non-configurable bootstrap Providers, and after the
first Host exists it launches every other Provider, controller, service, and
worker as a Process under a Host or a Guest.

## Config schema

None. Like the other bootstrap Provider, it takes no operator configuration.

## Exported resource types

`Process` and `EphemeralProcess`, the same pair `system-systemd` exports and
under the same conformance, so a future Process Provider passes the suite
without a schema change.

## Controllers / services / workers / binaries

One controller, shipped as a library type: `MinijailProcessProvider`,
generic over the injected `ProcessLaunchEffectPort`. It ships no binary.

## Placement and dependencies

Runs in the fixed core-controller process boundary at bootstrap, under a
distinct authenticated subject from `system-core`. The system domain is
always supported; the user domain is admitted only where the Provider
descriptor says so.

## RBAC requirements

Process and EphemeralProcess reconciliation under the compiled,
non-extensible bootstrap policy that binds the exact `system-minijail`
subject. After bootstrap it is an ordinary RBAC subject.

## Security posture

The sandbox is compiled inline, the spawn prefers `clone3(CLONE_PIDFD)` so
the process is born directly in its final cgroup, and d2b owns `wait` and
reap.

This controller never imports or calls the broker. It validates the
ExecutionSpec and SandboxSpec and calls the injected
`ProcessLaunchEffectPort` with the resource UID and the compiled digests;
the effect adapter is the sole caller of the broker's spawn effect, and the
broker remains the sole privileged executor and audit owner.

Adoption verifies pid, process start time, cgroup, executable, template, and
generation before any `pidfd_open`. The pid and start-time pair is the
pid-reuse guard: a matching pid whose start time disagrees is a different
process, so it is ambiguity. Ambiguity quarantines and reports Unknown; it
never signals, kills, or reuses.

## State and telemetry

No state of its own. Public status is the shared `ProcessStatusReport`,
which carries an opaque identity digest, typed resource references, and
closed enumerations only.

## Build and test

```bash
cd packages && cargo test -p d2b-provider-system-minijail
cd packages && cargo clippy -p d2b-provider-system-minijail --all-targets
```
