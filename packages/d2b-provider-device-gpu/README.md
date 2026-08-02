# `d2b-provider-device-gpu`

This crate implements `Provider/device-gpu`, the combined GPU and video
physical `Device` Provider.

## Config and Nix authoring

The Device extension is `device-gpu.d2bus.org/Device/spec` version `1.0`.
Settings are strict and bounded: `renderNodeOnly`, `videoSidecar`,
`videoNvidiaDecode`, the closed `contextTypes` list, at most eight `displays`,
`egl`, `vulkan`, `crossDomainTrusted`, and `virglVideo`. Nix authors declare
the physical DRM `Device` under
`d2b.zones.<zone>.resources.<name>`. Shared arbitration is valid only with
`renderNodeOnly = true`; full GPU and video use exclusive arbitration.

## Controllers, workers, and placement

The controller manages one of `device-<uid-short>-gpu` or
`device-<uid-short>-render-node`, plus the optional
`device-<uid-short>-video` Process. All workers are Host-placed and
Provider-owned. Video starts only after the GPU/render-node worker is Ready.

## Dependencies and RBAC

The Provider reads the Device and Display dependencies, writes its Device
status/finalizer, and creates only its owned Process children. Core supplies
the opaque `GpuEffectPort` and maps it to audited `OpenDevice` and
`SpawnRunner` effects.

## Security and state ownership

The Provider receives only Core-derived effect tokens; it never receives
`/dev` paths, Wayland sockets, PIDs, capabilities, or broker connections.
Full GPU workers use exclusive claims. Render-node sharing is explicit in the
Device spec. The broker opens device fds before clone and applies the signed
allowlist.

## Telemetry and audit

Metrics use fixed Provider/operation/outcome/error labels and never include
Zone/resource names, device selectors, paths, or process IDs. Core owns the
path-free effect audit records.

## Build and test

```bash
cargo test -p d2b-provider-device-gpu
cargo xtask check-provider-layout
```

Hermetic tests use fake effect ports; the `integration/` scenarios run through
the existing container or Host/Guest lane.

## Future standalone use

An extracted Provider repository should retain `d2b-contracts`, the signed
component descriptor, the wire-contract constants, and the opaque Core effect
adapter while replacing only workspace packaging and release metadata.
