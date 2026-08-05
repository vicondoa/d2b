# `d2b-provider-system-core`

The fixed `system-core` bootstrap Provider: the one core-controller process
per Zone, which is also `Provider/system-core`.

See [Create a Provider](../../docs/how-to/create-provider.md) for the
uniform crate layout, schema links, configuration, and test lanes.

## Provider identity

`system-core`, referenced as `Provider/system-core`. It is one of the two
fixed, non-configurable bootstrap Providers, so it is not represented by a
Process resource and cannot be hand-authored as an operator-declared
Provider.

## Config schema

None. The bootstrap Provider takes no operator configuration; there is
nothing to project, and admitting a config surface here would be a way to
influence the most privileged component in the Zone.

## Exported resource types

`Host` and `User`, and nothing else. The list is enforced as an allowlist in
`ownership.rs`, so `Process`, `EphemeralProcess`, `Volume`, `Network`,
`Device`, `Credential`, and every semantic runtime, desktop, or cloud
ResourceType are refused rather than merely undocumented. Process and
EphemeralProcess belong to `system-systemd` and `system-minijail`.

## Controllers / services / workers / binaries

One controller, embedded in the Zone runtime binary rather than launched as
a Process. This crate ships the reconciliation logic as a library: the
`HostReconciler` computes Host status, and the `UserReconciler` performs
local User discovery over an injected effect port. It ships no binary.

## Placement and dependencies

Runs as the fixed core-controller process, before any other resource exists.
It depends only on the v3 primitive contracts. It does not depend on the
Process conformance crate, because it owns no Process ResourceType.

## RBAC requirements

Host and User reconciliation only, under the compiled, non-extensible
bootstrap policy that binds the exact `system-core` subject. Bootstrap
authority is not widenable by operator config, and after bootstrap this is
an ordinary RBAC subject.

## Security posture

The standing Provider rules apply unchanged: no privileged mutation, host
state reached only through an injected typed effect port, and the broker
remains the sole privileged executor and audit owner. In particular this
crate calls no NSS interface and reads no local account database; User
discovery is an effect, and the fixed core effect adapter is the sole
implementor of `UserDiscoveryEffectPort`.

The user-only Host posture is non-negotiable. `isolationPosture` and
`isolationPostureMessage` are derived from the spec alone, and a submitted
status naming either field is rejected outright, including an explicit null
intended to suppress the posture.

## State and telemetry

No state of its own. Public status is `HostStatusReport` and
`UserStatusReport`, both of which carry only typed resource references,
closed enumerations, and an opaque identity digest. No uid, gid, home
directory, shell, resolved OS username, unit name, cgroup, or path is
representable in either, and both render redacted through `Debug`.

## Build and test

```bash
cd packages && cargo test -p d2b-provider-system-core
cd packages && cargo clippy -p d2b-provider-system-core --all-targets
```
