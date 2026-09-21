# `d2b-provider-host`

This is the crate root for the `Host` resource type. It owns the type's
driver, its spec decoder, and the driver declaration the v3 resource plane
registers the type by.

`Host` is the physical/local host execution, policy, and budget parent. The
driver observes it and nothing else: it realizes no target-local state, owns
no child row, and spawns nothing.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `system-core` |
| ResourceType | `Host` |
| Package | `packages/d2b-provider-host/` |
| Driver declaration | `host_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: `Host` rows carry the closed
`HostSpec` base contract from `d2b-contracts-resource` (`providerRef`,
execution policy, budget, attachments, isolation posture) and nothing outside
it decodes at validate. The `spec.providerRef` fence is part of that
contract: `Provider/system-core` is the only Provider the Host contract
admits, and any other value is a terminal refusal.

## Exported resource types

`Host` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a host is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `Host` rows through the `ResourceDriver` verbs: `validate` decodes the
stored spec and enforces the Provider fence, `recover` adopts what the
contract already establishes, `reconcile` observes the host once per desired
generation and publishes the typed in-memory status, `finalize` drains owned
children, and `delete` converges without effects.

`HostDriverFactory` is the registration surface; `host_descriptor` carries it
with the decoder, the type's verbs, execution domain, reads, and the
`BUILTIN | STARTUP` allowed-source mask.

## Placement and dependencies

`Host` names no placement anchor: the row *is* the host target, so the plane
reconciles it on its own Host domain. The observation is this crate's own
bounded probe (`src/probe.rs`), served through the family's declared effects
service (`host.d2bus.org/effects`, U5): the probe reads `/proc`,
`/etc/os-release`, `/dev`, `/sys/fs/cgroup`, and `/run/user` directly, and
the one daemon-owned read - the minijail platform gate - arrives as the
declared `MinijailPlatformGateSource` facet the composition root supplies.

The crate depends on `d2b-contracts-resource`, `d2b-provider-system-core`
(the Host reconciler and probe-port types the implementation drives),
`d2b-resource-runtime`, `d2b-resource-types`, and `d2b-provider-toolkit`
(the effects-service vocabulary it serves). The probe reads the pipewire
runtime socket name and the usbip kernel module names as host-state paths
spelled in the crate itself, with no sibling-provider vocabulary. It depends
on no daemon runtime; the daemon hosts the family's declared effects service
per zone from the crate's own factory.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. It serves no broker operations.

## Security posture

The driver never invents a capability, kernel release, or process count from
spec text: the stored spec is decoded strictly, the Provider fence refuses
anything but `Provider/system-core`, and every observation is a bounded value
the crate's own probe already reduced (bounded reads, bounded lengths, a
degraded fallback when the probe cannot complete). A spec that fails to
decode is a terminal refusal, and the family owns no spawn surface at all.

## State and telemetry

The type publishes no durable status: the in-memory `HostDriverStatus` (the
generation observed, plus the typed `HostObservationReport`) is the only
status projection, matching the plane's in-memory status rule. Failures
travel as registered failure kinds (`system-core-spec-invalid`,
`system-core-host-observation-failed`, `system-core-drain-pending`) on the
structured failure surface, which is what the daemon logs and what tests
assert.

## Build and test

```bash
cargo test -p d2b-provider-host
```

The unit tests drive validate, recover, reconcile, finalize, and delete over
a scripted effects double, prove a Host row reaches its driver through the
registry alone, and prove the effects service's happy and degraded
observation paths over a scripted probe; the `registration` suite proves the
declaration registers the type with its decoder and factory, that a
duplicate registration is refused, and that the declared mask cannot arrive
after the plane opens.
