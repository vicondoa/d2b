# `d2b-provider-runtime-qemu-media`

Canonical implementation of `Provider/runtime-qemu-media`. The Provider
reconciles one manual-only QEMU Guest runtime per declared Guest resource.
`d2bd` owns lifecycle orchestration, Core owns attachment authorization, and
the broker/ProviderSupervisor owns privileged spawn and pidfd evidence.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[runtime-qemu-media dossier](../../docs/specs/providers/ADR-046-provider-runtime-qemu-media.md)
for the full contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `runtime-qemu-media` |
| Provider reference | `Provider/runtime-qemu-media` |
| Package | `packages/d2b-provider-runtime-qemu-media/` |

## Config schema

`ProviderConfig` requires a Host controller placement, bounded QMP deadlines,
opaque Network and Volume Provider references, and bounded runtime tmpfs
quotas. Configuration is projected to the controller only; worker Processes
receive no root Provider config, ResourceAPI authority, or d2b-bus authority.

## Exported resource types

The crate validates the locator-free Guest settings and status extension,
including Volume media references, bounded provider phases, and the
manual-only `paused-at-boot` state. Runtime Volumes, Process specs, Endpoint
attachments, and Device/host-kvm observations remain opaque Core resources.

## Controllers / services / workers / binaries

`QemuMediaController` gates launch on media, Network, display, and
Host-global KVM readiness; the QEMU worker is the signed
`qemu-media-runner` Process template. QMP capability negotiation, health,
media hotplug, restart adoption, and finalization are typed seams over
Core-owned effects.

## Placement and dependencies

The controller runs on the configured Host. A Guest may depend on
Volume/virtio-blk media, Network refs, Device/host-kvm, and an optional
display-wayland WaylandSession. No Provider state Volume is declared.

## RBAC requirements

The controller watches Guest, Volume, Network, Device, Process, and optional
WaylandSession resources. It requests only typed Core effects and never names
a broker operation, host path, socket path, executable path, argv, fd, or
numeric principal.

## Security posture

Host-global Device ownership is reserved before effects start and held until
media effects close. Process adoption verifies the complete identity tuple
before pidfd acquisition; ambiguous candidates are quarantined. Audit and
telemetry projections are bounded and redacted.

## State and telemetry

Operational state is status-first and restart-rehydratable from the Zone store,
operation ledger, and external process observations. No Provider state Volume
or secret/path-bearing diagnostics are used. Audit events and metric labels
use fixed semantic vocabularies.

## Build and test

```bash
bazel test //packages/d2b-provider-runtime-qemu-media:all
```
