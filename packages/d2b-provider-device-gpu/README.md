# `d2b-provider-device-gpu`

`Provider/device-gpu` manages the combined GPU and hardware-video physical
`Device` realization. The crate contains no daemon, broker, host lifecycle,
or other Provider implementation dependency.

See [Create a Provider](../../docs/how-to/create-provider.md) for the
uniform crate layout, schema links, configuration, and test lanes.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `device-gpu` |
| Provider reference | `Provider/device-gpu` |
| ResourceType | `Device` |
| Package | `packages/d2b-provider-device-gpu/` |

The signed Provider descriptor supplies the component templates and binary
artifacts. Core remains the authority that resolves physical device grants
and worker LaunchTickets.

## Config schema

The Device extension is
`device-gpu.d2bus.org/Device/spec`, version `1.0`, with strict
deny-unknown settings:

| Setting | Bounds or rule |
| --- | --- |
| `renderNodeOnly` | shared arbitration is valid only when true |
| `videoSidecar` | starts the separate video worker; requires full-GPU mode |
| `videoNvidiaDecode` | valid only with `videoSidecar` |
| `contextTypes` | 1-3 distinct values from `virgl`, `virgl2`, `cross-domain` |
| `displays` | at most eight `{ hidden }` entries |
| `egl`, `vulkan`, `crossDomainTrusted`, `virglVideo` | bounded booleans; `virglVideo` conflicts with `videoSidecar` |

Nix authors declare the physical DRM `Device` under
`d2b.zones.<zone>.resources.<name>` with
`providerRef = "Provider/device-gpu"`. Device selectors, host paths, binary
paths, sockets, and capabilities are not Provider settings.

## Exported resource types

The Provider implements the standard `Device` ResourceType for one physical
DRM GPU. Full-GPU and render-node-only realizations share that Device
authority. An optional video sidecar is a separate Process child, not a
second public Device or a provider-named ResourceType.

## Controllers / services / workers / binaries

`GpuController` reconciles the GPU/render-node worker and, when requested,
the separate video worker. `GpuEffectPort::open_devices` receives only a
Core-derived `GpuEffectTokenSet` and returns an opaque `GpuLaunchTicket`.
The controller starts the GPU worker first and starts video only after that
worker is Ready.

The worker declarations are `device-<uid-short>-gpu`,
`device-<uid-short>-render-node`, and `device-<uid-short>-video`. The signed
component descriptor selects the crosvm and video-decoder artifacts. GPU and
video finalization is ordered video first, then GPU/render-node.

## Placement and dependencies

All Provider workers are Host-placed and are supervised through Core's
Process controller. The Provider reads its Zone Device and Display-related
dependencies, writes only its Device status/finalizer and owned Process
children, and depends on the neutral contracts plus serde. Core maps opaque
effects to audited `OpenDevice` and `SpawnRunner` operations.

## RBAC requirements

The Provider needs bounded read/watch access to its Device and attachment
dependencies, status/finalizer authority on its owned Device, and authority
to create or reconcile only its own GPU, render-node, and video Process
children. It has no direct broker or host-device permission. Core admits each
opaque effect token and LaunchTicket.

## Security posture

Full GPU claims are exclusive. Render-node sharing is permitted only when
`renderNodeOnly` is true and the Device arbitration is explicitly shared.
Video is a distinct worker and can start only after the GPU worker is Ready;
NVIDIA device grants are opt-in through the bounded video setting.

The Provider receives opaque tokens and tickets only. It never receives a
`/dev` path, Wayland socket, PID, capability, fd, or broker connection. The
privileged broker opens device fds before worker clone and applies the signed
allowlist. The Provider cannot widen that allowlist or bypass Core authority.

## State and telemetry

The Provider declares no state Volume. Bounded lifecycle observations remain
in Device status and the Core operation ledger; GPU/video payloads and
physical-device identity are not copied into status. Metrics use fixed
Provider, component, operation, outcome, and error labels only. Zone/resource
names, selectors, paths, sockets, PIDs, and device identity are excluded.
Core owns path-free audit records for broker effects.

## Build and test

```bash
cd packages
cargo test -p d2b-provider-device-gpu
cargo nextest run -p d2b-provider-device-gpu
cargo clippy -p d2b-provider-device-gpu --all-targets -- -D warnings
cargo run -p xtask -- check-provider-layout
```

The hermetic tests cover settings and unknown-field rejection, arbitration,
effect-token bounds, GPU-before-video sequencing, process selection, and the
frozen media wire contract. The declared `integration/` scenarios require the
existing Host/Guest lane and run through `make test-host-integration`; real
GPU hardware coverage remains under the repository hardware lane.

## Future standalone use

An extracted Provider repository would retain the Provider identity, signed
component descriptor, neutral contracts, wire-contract constants, and opaque
Core effect adapter while replacing workspace packaging and release metadata.
