# `d2b-provider-system-systemd`

The `system-systemd` Process Provider controller.

See [Create a Provider](../../docs/how-to/create-provider.md) for the
uniform crate layout, schema links, configuration, and test lanes.

## Provider identity

`system-systemd`, referenced as `Provider/system-systemd`. It is an ordinary
Provider, not a bootstrap one.

## Config schema

The Provider config is bounded: `launchTimeoutSec`,
`terminationGraceSec`, `userManagerCheckTimeout`, and
`maxConcurrentLaunches`. Unit names and systemd properties are effect-port
implementation details.

## Exported resource types

`Process` and `EphemeralProcess`, for systemd-capable Hosts and Guests. The
ResourceType and its status projection do not change with the execution
parent.

## Controllers / services / workers / binaries

One controller, shipped as a library type: `SystemdProcessProvider` and its
`SystemdProcessController`, generic over the injected
`SystemdProcessEffectPort`. Lifecycle, drain, audit, and metric helpers remain
typed and path-free.

## Placement and dependencies

Runs under a Host or a Guest whose service manager is systemd. It depends on
the v3 primitive contracts and on the Provider-neutral Process conformance
crate, and on nothing else.

## RBAC requirements

Process and EphemeralProcess reconciliation for the resources it is
authorized to own. It claims no wildcard permission.

## Security posture

A process is a non-forking transient system unit or scope, or, for the user
domain, a verified transient user scope created through the fixed user
supervisor. Identity is the unit InvocationID bound together with the
cgroup, the unit main process, that process's start time, and the Provider,
template, and generation triple. A unit name alone is never identity, and is
never public status. systemd owns `wait` and reap; this Provider holds only
a locally verified pidfd.

Neither this controller nor the process it launches calls systemd's D-Bus or
socket API, and neither calls `pidfd_open`. The controller validates the
ticket and calls the injected effect port, which the fixed core effect adapter
implements and which is the sole caller of the systemd effect owner.

Adoption revalidates every required identity binding before a pidfd is
opened. Ambiguity quarantines; it never signals, kills, or reuses.

## State and telemetry

No state of its own and no persistent framework unit. Public status is the shared `ProcessStatusReport`,
which carries an opaque identity digest, typed resource references, and
closed enumerations only.

## Build and test

```bash
cargo test -p d2b-provider-system-systemd
cargo clippy -p d2b-provider-system-systemd --all-targets
```
