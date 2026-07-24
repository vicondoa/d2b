# ADR 0046 Provider dossier: device-gpu

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-device-gpu` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 8 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-device-gpu` crate, GPU/video controller contracts, Nix graphics/video emitters |
| Depends on | `ADR-046-resources-device`, `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-telemetry-audit-and-support` |
| Supersedes | `ProcessRole::Gpu`, `ProcessRole::GpuRenderNode`, `ProcessRole::Video` in `packages/d2b-core/src/processes.rs`; Nix `nixos-modules/components/graphics.nix`; Nix `nixos-modules/components/video/guest.nix`; `d2b.vms.<vm>.graphics.*` options |

## Purpose

`Provider/device-gpu` is the combined GPU-acceleration and hardware-video-decode
Provider for d2b v3. It is a single crate (`packages/d2b-provider-device-gpu/`)
that manages:

- **Full virtio-gpu**: exclusive physical DRM device claim, crosvm `device gpu`
  sidecar, cross-domain Wayland channel, Vulkan/EGL/virglrenderer acceleration.
- **Render-node-only**: exclusive or explicitly shared DRM render-node claim,
  user-namespace crosvm GPU worker, no full virtio-gpu bind-mount.
- **Hardware video decode**: `crosvm device video-decoder --backend vaapi` sidecar
  (vhost-user-media, virtio-media, `virtio_id=48`), optionally with NVIDIA
  device passthrough. Always separate from the GPU worker Process; always
  depends on a full-GPU Device claim (`renderNodeOnly=false`).

The Provider owns: GPU/video worker Process lifecycle and sequencing; physical
DRM device probe and hotplug observation; exclusive/shared arbitration
enforcement; broker operation orchestration; and Nix option migration from
`d2b.vms.<vm>.graphics.*` to `d2b.zones.<zone>.resources.*`.

VFIO and SR-IOV passthrough are **not** part of the standard GPU device claim
and are reserved for a future Provider.

## Identity

```text
Provider/device-gpu
```

Crate: `packages/d2b-provider-device-gpu/`

### Crate layout

```text
packages/d2b-provider-device-gpu/
  src/
    lib.rs               Controller entry point; exports controller/worker binaries
    controller.rs        Async reconcile loop; ResourceClient; spec/status owner
    probe.rs             GpuEffectPort::probe_drm_device call; hotplug observe scheduler; three-strike counter
    arbitration.rs       Exclusive vs shared claim arbitration; conflict detection
    worker_gpu.rs        Full GPU and render-node Process creation/teardown
    worker_video.rs      Video-decoder Process creation/teardown; wire-contract check
    argv.rs              Thin re-export of d2b-host gpu_argv / video_argv generators
    broker.rs            Device claim registration; GpuEffectPort claim state (no execution authority; claim authority is Device resource status + core Operation ledger)
    status.rs            Status writer; condition builder; phase state machine
    audit.rs             Path-free audit record emitter
    error.rs             Typed error enum; stable closed-set error slugs
  tests/
    combined_reconcile.rs        GPU+video combined state machine; fake broker/supervisor
    render_node_enforcement.rs   shared+renderNodeOnly=false spec rejected at controller
    wire_constant_snapshot.rs    Byte-stable wire-contract constants vs video_argv.rs
    conformance.rs               Spec/settings serde round-trip; ResourceTypeSchema; deviceUsage/budget/endpoints/readiness field validation
    arbitration_conflict.rs      Exclusive claim from second Guest rejected
    video_dependency.rs          Video Process not started until GPU Process is Ready
    seccomp_policy_ref.rs        GPU/video/render-node seccomp policy ref names stable
  integration/
    gpu_worker_start/            GPU worker Process obtains broker tokens, reaches Ready
    render_node_shared/          Two Guests share same render-node Device simultaneously
    video_dependency/            Video Process starts only after GPU Process is Ready
    README.md                    How to invoke integration fixtures; hardware test note
  README.md
```

Workspace policy rejects the crate if `src/`, `tests/`, `integration/`, or
`README.md` is missing (these four paths only). The layout must allow moving
the crate to its own GitHub repository without splitting semantics or copying
daemon internals.

### Controller Process (static, created by core ProviderDeployment)

When the framework processes a `Provider` resource with
`spec.config.controllerExecutionRef`, core `ProviderDeployment` creates one
static controller Process resource. The device-gpu controller **does not**
create or own this Process; core creates and manages it, aggregates its status
into the Provider status, and deletes it when the Provider is deleted.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: device-gpu-controller
  zone: dev
  ownerRef: Provider/device-gpu
  managedBy: core         # created by core ProviderDeployment; not by the controller itself
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system   # resolved from Provider.spec.config.controllerExecutionRef
  domain: system
  processClass: controller
  template: device-gpu-controller
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts, cgroup]
    capabilityClasses: []
    seccompClass: w1-provider-controller
    startRoot: false
    userNamespace: null
  budget:
    cpu:
      request: "50m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "256Mi"
    pids:
      limit: 64
    fds:
      limit: 32
  networkUsage: null
  deviceUsage: []
  mounts: []
  readiness:
    class: provider-defined
    initialDelay: "1s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
  restartPolicy:
    class: on-failure
    backoffBase: "5s"
    backoffMax: "5m"
    backoffMultiplier: 2
    maxRestarts: 10
    resetAfter: "1h"
```

Core aggregates the controller Process status (phase, conditions) into the
`Provider/device-gpu` status. Per D087, `device-gpu` declares no Provider state
Volume; bounded non-secret controller operational state lives in Device/Provider
status and the core Operation ledger, and the controller has no `/state` mount.

## Device spec served by this Provider

### Canonical ResourceSpec

Normative D089 spec layering: Device base fields are ResourceType base
`spec.*` fields, including `spec.providerRef`, `deviceClass`,
`inventory.selector`, attachments, and arbitration. This Provider's
desired-only extension is the canonical `spec.provider = { schemaId:
"device-gpu.d2bus.org/Device/spec", schemaVersion, settings }` envelope; it is
manifest-registered/signed, strict deny-unknown, bounded, versioned and
digested,
validated against `spec.providerRef` at Nix build and API admission,
implementation-only, and may not shadow base fields. Shared fields are promoted
to the Device base. The Provider implements the exact base Device spec/status
version/fingerprint, accepts the canonical minimal valid base Spec, and rejects
unsupported optional base capabilities only through its signed capability matrix
and provider-neutral `unsupported-capability`. `spec.provider` aligns with
`status.provider`; generic CLI/controllers operate on the base spec and base
status only. No secret bytes are allowed in any spec layer, and no
credential material is allowed in `spec.provider.settings`.

```yaml
apiVersion: resources.d2bus.org/v3
type: Device
metadata:
  name: corp-vm-gpu
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: Guest/corp-vm
  finalizers: [device-gpu.d2bus.org/worker-stopped]
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:00:00Z
spec:
  providerRef: Provider/device-gpu
  deviceClass: physical
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector:
      busClass: drm
      label: host-gpu
      pciSlot: null
  provider:
    schemaId: "device-gpu.d2bus.org/Device/spec"
    schemaVersion: "1.0.0"
    settings:
      renderNodeOnly: false
      videoSidecar: false
      videoNvidiaDecode: false
      contextTypes: [cross-domain, virgl, virgl2]
      displays: [{hidden: true}]
      egl: true
      vulkan: true
      crossDomainTrusted: false
      virglVideo: false
status:
  observedGeneration: 1
  phase: Ready
  conditions: []
  lastReconciledAt: 2026-07-22T00:00:01Z
  device:
    present: true
    health: healthy
    holderRefs: [Guest/corp-vm]
    claims:
      - holderRef: Guest/corp-vm
        claim: exclusive
        passthrough: gpu-virtio
        claimedAt: 2026-07-22T00:00:01Z
        health: healthy
    provisionedAt: null
    lastProbedAt: 2026-07-22T00:00:00Z
    providerDiagnostic: null
```

## Root config schema

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `renderNodeOnly` | bool | `false` | — | If true, render-node-only mode; no full virtio-gpu bind-mount. Shared arbitration is permitted only when this is `true`. |
| `videoSidecar` | bool | `false` | — | If true, spawn a crosvm video-decoder Process alongside the GPU worker. Requires `renderNodeOnly=false`. |
| `videoNvidiaDecode` | bool | `false` | — | If true, include `nvidia-ctl`, `nvidia-device`, `nvidia-uvm` allowlist tokens for the video worker in addition to `dri`. No effect when `videoSidecar=false`. |
| `contextTypes` | list\<enum\> | `[cross-domain, virgl, virgl2]` | closed set | GPU context types: `virgl`, `virgl2`, `cross-domain`. Order is preserved and emitted in sorted order in canonical JSON. |
| `displays` | list\<object\> | `[{hidden: true}]` | 0–8 entries | Virtual display config. Each entry: `{hidden: bool}`. |
| `egl` | bool | `true` | — | EGL rendering via virglrenderer. |
| `vulkan` | bool | `true` | — | Vulkan rendering via venus. |
| `crossDomainTrusted` | bool | `false` | — | Enable the `cross-domain` virtio-gpu context type for cross-domain Wayland forwarding. Default `false`; set `true` only for VMs where Wayland forwarding is the primary use case (e.g., a Wayland-forwarding launchpad VM). Must be `false` for VMs running Docker or container workloads. |
| `virglVideo` | bool | `false` | — | Experimental virglrenderer/rutabaga video forwarding (separate from `videoSidecar`). Requires a crosvm build with `use_video=true` patch. Mutually exclusive with `videoSidecar=true` on the same Guest. |

The crosvm binary path and the crosvm video-decoder binary path are resolved
from the signed `d2b-provider-device-gpu` package closure. They are not
configurable fields in the Device spec.

## Discovery and probe

### DRM device probe via GpuEffectPort

The device-gpu controller probes physical GPU presence through its injected
opaque `GpuEffectPort`. The controller does **not** directly read `/dev/` or
`/sys/` paths; the effect port handles device inventory and allowlist fd opens
on the controller's behalf.

The `inventory.selector` for GPU devices always uses `busClass: drm`.

**Probe sequence on each observe trigger:**

1. Controller calls `GpuEffectPort::probe_drm_device(selector)`. The effect port
   matches the `label` (and optional `pciSlot`) from the selector against the
   trusted device table and returns a presence/health result.
2. A DRM card node present and accessible: `DevicePresent=True`, `health=healthy`.
3. If the DRM card node is absent at probe time: begin consecutive-failure
   counting (see Physical probe failure semantics below).
4. Render-node availability is checked in the same probe call; the effect port
   includes it in the returned result.

**Observe interval:** default 30 s; maximum 60 s. Configurable in the Provider
root config via `observeIntervalSecs: uint (10–60)`.

### Physical probe failure semantics

The standard three-strike semantics from `ADR-046-resources-device` apply
unchanged:

| Consecutive probe failures | Status transition |
| --- | --- |
| 1 (first) | phase → `Unknown`; `DevicePresent` status=`Unknown` |
| 2 | `DevicePresent` remains `Unknown`; phase remains `Unknown` |
| 3 | phase → `Degraded`; `DevicePresent` status=`False`, reason=`device-consecutive-probe-failures-exceeded` |
| Device returns | phase → `Ready`; `DevicePresent` status=`True` |

A single probe failure does not set `DevicePresent=False` or stop the GPU or
video workers. After three consecutive failures, all claimant Guest controllers
receive a `dependency-changed` trigger through the normal resource watch path.
The GPU and video worker Processes transition per the `owned-resource-changed`
reconcile trigger handling; the Guest controller may stop or degrade the Guest.

When the device returns, the Device controller sets phase `Ready` and
re-triggers all claimants.

## Arbitration

### Full GPU — always exclusive

A Device resource with `settings.renderNodeOnly=false` always uses
`arbitration: exclusive` and `maxConcurrentClaims: 1`. This covers:

- Full virtio-gpu passthrough: `crosvm device gpu` with card-node access,
  `kvm`, `dri`, `udmabuf`, and optionally `nvidia-ctl`/`nvidia-device`/`nvidia-uvm`
  broker allowlist tokens.
- Any video-decoder sidecar (`settings.videoSidecar=true`) that depends on a
  full-GPU claim.

A spec that sets `arbitration: shared` with `settings.renderNodeOnly=false`
is rejected at admission with error `shared-arbitration-requires-render-node-only`
and fails the NixOS eval.

### Render-node-only — exclusive default, explicitly shared

A Device resource with `settings.renderNodeOnly=true` may use either
`arbitration: exclusive` (default) or `arbitration: shared` (must be explicitly
set by the operator). When `arbitration: shared`, `maxConcurrentClaims` may be
1–16.

Render-node-only mode:
- Provider/system-minijail validates the LaunchTicket and requests
  `OpenDevice(dri)` via its injected `MinijailProcessEffectPort`; the core
  executor pre-opens the DRM render node fd. No full-card or auxiliary device
  tokens are included.
- Does **not** include `nvidia-ctl`, `nvidia-device`, `nvidia-uvm`, or
  `udmabuf` allowlist tokens. Those root:video-owned character devices are
  inaccessible inside the single-entry user namespace where in-NS UID/GID 0
  maps to the allocator-assigned worker principal's stable UID; they appear as
  UID 65534 (overflow) and DAC access is denied.
- Omits the full virtio-gpu display/cross-domain plumbing.
- Always uses `settings.videoSidecar=false`; `videoSidecar` requires
  `renderNodeOnly=false`.

The render node fd is inherited by the crosvm process via the privileged broker's
private fd-inheritance protocol (`packages/d2b-priv-broker/src/sys.rs`
`clone3_spawn_runner`). The fd survives the user-NS pivot without losing access
semantics because the kernel checks permissions at `openat2` time only.

### VFIO/SR-IOV — not included, reserved

VFIO and SR-IOV passthrough are **not** part of the `device-gpu` claim. They
are reserved for a future Device Provider that uses `busClass: pci` selectors
and a distinct broker operation. No existing code path or admitted Device spec
exercises VFIO; implementing it would require a separate hardware-validated
Provider dossier.

### Admission invariants

| Rule | Error slug |
| --- | --- |
| `arbitration=shared` requires `settings.renderNodeOnly=true` for `Provider/device-gpu` | `shared-arbitration-requires-render-node-only` |
| `settings.videoSidecar=true` requires `settings.renderNodeOnly=false` | `video-sidecar-requires-full-gpu` |
| `settings.virglVideo=true` and `settings.videoSidecar=true` on the same Guest | `virgl-video-and-sidecar-mutually-exclusive` |
| `settings.videoNvidiaDecode=true` requires `settings.videoSidecar=true` | `nvidia-decode-requires-video-sidecar` |

## Per-Guest claims

### Device attachment on Guest spec

```yaml
# Guest spec (desired state) — standard deviceAttachments; no custom claim/passthrough array
spec:
  deviceAttachments:
    - deviceRef: Device/corp-vm-gpu       # full virtio-gpu

    - deviceRef: Device/dev-vm-render     # shared render-node
```

GPU mode (full virtio-gpu vs render-node) and arbitration are determined by
the Device resource `spec.provider.settings` managed by the operator. The
common Guest
spec carries only standard `deviceAttachments` referencing Device resources; no
provider-specific `passthrough` or `claim` fields appear in the Guest spec.

Adding or removing a `deviceAttachments[]` entry on a Guest spec is a normal
`update-spec` RBAC verb. The device-gpu controller detects the change via its
reconcile loop, performs arbitration, and writes result to Device status. There
is no separate `claim-device` or `release-device` verb.

### Process device dependency entry

A Process that requires the GPU (e.g., the CH runner) declares a dependency:

```yaml
spec:
  deviceUsage:
    - deviceRef: Device/corp-vm-gpu
      access: shared
      purpose: gpu-socket
```

The Process controller verifies the Device is `Ready` and claimed by the owning
Guest before launching the Process. The GPU sidecar surface is an owned
`Endpoint` resource consumed as `Endpoint/<name>` by the CH runner; no socket
locator is expressed as a raw path in any resource spec or status.

## GPU worker Process: full GPU

### Process resource shape

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: device-<uid-short>-gpu
  zone: dev
  ownerRef: Device/corp-vm-gpu
  managedBy: controller
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: worker
  template: gpu-worker   # plain template ID matching ^[a-z][a-z0-9-]*$; declares mapped-root user-NS requirement
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts, cgroup, user]
    capabilityClasses: []
    seccompClass: w1-gpu
    startRoot: false
    userNamespace:
      mappingClass: process-principal-root   # uid/gid resolved privately by core from signed worker template
  budget:
    cpu:
      request: "500m"
      limit: "4000m"
    memory:
      request: "256Mi"
      limit: "2Gi"
    pids:
      limit: 32
    fds:
      limit: 16
  networkUsage: null
  deviceUsage:
    - deviceRef: Device/corp-vm-gpu
      access: exclusive
      purpose: gpu-virtio
  readiness:
    class: provider-defined
    initialDelay: "2s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
  restartPolicy:
    class: on-failure
    backoffBase: "5s"
    backoffMax: "5m"
    backoffMultiplier: 2
    maxRestarts: 10
    resetAfter: "1h"
```

The worker produces the stable GPU sidecar Endpoint separately:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: corp-vm-gpu-sidecar
  zone: dev
  ownerRef: Device/corp-vm-gpu
spec:
  providerRef: Provider/device-gpu
  producerRef: Process/device-<uid-short>-gpu
  endpointClass: device
  transport: unix
  purpose: gpu-sidecar
  serviceFingerprint: device-gpu.d2bus.org/gpu-sidecar/v1
  locality: cross-domain
  visibility: provider-internal
  attachmentPolicy: launch-ticket-only
  consumerPolicy: [Provider/runtime-cloud-hypervisor]
  lifecyclePolicy: recycle-with-producer
```

`uid-short` = first 12 hex characters of the owner Device resource UID.
VM or Guest human names never appear in Process resource names. Resolved
principal, cgroup placement, fd numbers, and socket paths are private
LaunchTicket/broker state not expressed in the resource spec.

### Broker Device allowlist (full GPU)

The broker opens device fds exclusively from the declared Device allowlist.
No `/dev` path crosses the public wire. Maximum 8 fds per Process launch.

| Allowlist token | Always | Conditional |
| --- | --- | --- |
| `kvm` | ✓ | |
| `dri` (render node) | ✓ | |
| `nvidia-ctl` | | when `videoNvidiaDecode=true` or NVIDIA graphics VM |
| `nvidia-device` | | when `videoNvidiaDecode=true` or NVIDIA graphics VM |
| `nvidia-uvm` | | when `videoNvidiaDecode=true` or NVIDIA graphics VM |
| `udmabuf` | ✓ | |

Source: `packages/d2b-core/src/bundle_resolver.rs` lines 1888–1894 (ProcessRole::Gpu
and ProcessRole::GpuRenderNode broker device token comment). Source:
`nixos-modules/minijail-profiles.nix` gpu profile `deviceBinds` list.

The full GPU profile uses **zero host capabilities** (`capabilities = []`).
CAP_SYS_NICE was previously listed in the capability matrix; it was confirmed
unnecessary at runtime on NVIDIA Quadro T1000 (virgl/venus/cross-domain operate
under SCHED_OTHER). The `sandbox.seccompClass` for this profile is `w1-gpu`,
enforced by `system-minijail` before exec; if the compiled BPF policy is
unavailable in the Provider closure, the launch fails closed.

### crosvm GPU sidecar argv (full GPU)

> **Current-source evidence** (`packages/d2b-host/src/gpu_argv.rs`,
> `implemented-and-reachable`). The argv shape below reflects the v3 baseline.
> Socket paths and binary path are resolved by Provider/system-minijail from
> the signed LaunchTicket; they are private LaunchTicket state and do not
> appear in Process resource specs, status, audit, or telemetry. The
> `--wayland-sock` argument is filled from the opaque display endpoint FD
> delivered by d2b-bus/ProviderSupervisor via the `display-wayland` dependency;
> the device-gpu Provider performs no host compositor socket resolution.

```text
crosvm device gpu \
  --socket <gpu-socket-path> \
  --wayland-sock <wayland-socket-path> \
  --params '{"context-types":"cross-domain:virgl:virgl2","displays":[{"hidden":true}],"egl":true,"vulkan":true}'
```

Source: `packages/d2b-host/src/gpu_argv.rs` (implemented-and-reachable).

The `GpuArgvInput` struct from `d2b-host` is re-exported from `argv.rs` in
the Provider crate. The provider builds the input struct from resolved Device
spec settings and the signed component descriptor paths; it never constructs
raw argv strings.

**`crossDomainTrusted=false` enforcement:** The signed component descriptor is
static and is not rewritten per Device. `crossDomainTrusted` is a validated
Device setting projected into the private LaunchTicket by Provider/system-minijail
at resolution time. When `settings.crossDomainTrusted=false` (the default),
system-minijail's argv builder omits `GpuContextType::CrossDomain` from the
`contextTypes` list in the runtime `--params` arg. When `crossDomainTrusted=true`,
`cross-domain` is included. This mirrors the Nix shell shim in
`nixos-modules/components/graphics.nix` that strips `cross-domain` at eval time.

**`implicit-render-server: true` and `external-blob: true`** are always emitted
in the `--params` JSON payload to enable the virglrenderer render server for
VA-API video decode and blob texture transfer. Source:
`nixos-modules/components/graphics.nix` `crosvmWithRenderServer` wrapper.

**`--gpu-device-node`** is appended when the render node fd is present.
This enables virglrenderer's `get_drm_fd` callback for VA-API video decode on
the host. Without it, rutabaga_gfx logs "no valid GPU path provided" and video
decode falls back to software. The privileged broker pre-opens the fd and passes
it via the private fd-inheritance protocol from the privileged broker; no device path appears
in the resource spec or argv.

**Venus Vulkan** is added to `context-types` when `vulkan=true`:
`contextTypes` output appends `venus` via the same `unique | join(":")` logic
as the Nix wrapper. The provider's argv builder handles this addition.

### Wayland cross-domain endpoint (full GPU only)

The GPU worker Process consumes an opaque cross-domain display endpoint from
its configured `display-wayland` dependency. The device-gpu Provider has **no
compositor authority** and performs no host socket resolution:

- The `display-wayland` dependency is declared in the GPU worker's signed
  component descriptor. d2b-bus/ProviderSupervisor resolves the dependency and
  routes the opaque display endpoint FD directly into the GPU worker LaunchTicket
  at execution time. The device-gpu controller and Provider never hold or request
  compositor socket handles.
- Device status exposes typed EndpointRefs such as
  `crossDomainEndpointRef: Endpoint/<display-endpoint>` and
  `gpuSidecarEndpointRef: Endpoint/<gpu-sidecar>`; no socket path, Wayland
  display name, UID-scoped path, or opaque endpoint ID appears in status, audit,
  or telemetry.
- The GPU worker uses the endpoint FD to expose the cross-domain Wayland channel
  to the Guest via the virtio-gpu context. The GPU worker produces virtio-gpu
  context/output consumed by the Guest Runtime; the host-side display endpoint
  is an input, not an output of this Provider.

The **video worker Process** does **not** hold this endpoint. See § Video
worker: denied host sockets.

### Endpoint resources (D092)

`Provider/device-gpu` declares conformance to the standard `Endpoint` base
schema. Stable GPU sidecar, video sidecar, and cross-domain display identities
are owned `Endpoint` resources with `producerRef` to the producing
`Process`/`Device`, closed `endpointClass`/`transport`, and no raw locator in
spec/status/CLI. Consumers use `Endpoint/<name>` ResourceRefs.
Core/ProviderSupervisor resolves Unix sockets or fd attachments only through
authorized EffectPort/LaunchTicket flows; unauthorized resolve fails
`endpoint-resolve-denied`. A producer restart bumps `endpointGeneration`, which
triggers CH/video dependencies through `dependency-changed`.

### Retained opaque handles

Retained opaque values are pidfds, LaunchTicket fd indexes, render-node fd
leases, transient DRM inventory handles, `OwnedTransport`, operation IDs, and
per-session compositor connection handles. They are high-churn, internal to the
controller/effect port, or have no independent lifecycle, so D092 does not
promote them to resources.

### Cloud Hypervisor connection

Cloud Hypervisor connects to the GPU sidecar via the `--gpu socket=...` flag,
appended by the CH runner Process via the signed provider component descriptor.
The private socket locator is resolved from `Endpoint/<gpu-sidecar>` only through
the authorized EffectPort/LaunchTicket path and is not a spec field, status
value, audit record, or telemetry attribute.

## GPU worker Process: render-node-only

### Process resource shape

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: device-<uid-short>-render-node
  zone: dev
  ownerRef: Device/dev-vm-render
  managedBy: controller
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: worker
  template: render-node-worker   # plain template ID matching ^[a-z][a-z0-9-]*$; declares mapped-root user-NS requirement
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts, cgroup, user]
    capabilityClasses: []
    seccompClass: w1-gpu-render-node
    startRoot: false
    userNamespace:
      mappingClass: process-principal-root   # uid/gid resolved privately by core from signed worker template
  budget:
    cpu:
      request: "100m"
      limit: "2000m"
    memory:
      request: "64Mi"
      limit: "1Gi"
    pids:
      limit: 16
    fds:
      limit: 8
  networkUsage: null
  deviceUsage:
    - deviceRef: Device/dev-vm-render
      access: shared
      purpose: gpu-render-node
  readiness:
    class: provider-defined
    initialDelay: "2s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
  restartPolicy:
    class: on-failure
    backoffBase: "5s"
    backoffMax: "5m"
    backoffMultiplier: 2
    maxRestarts: 10
    resetAfter: "1h"
```

Resolved principal, cgroup placement, and fd numbers are private LaunchTicket/broker
state not expressed in the resource spec.

### User namespace (ADR 0021 model)

The render-node-only mode uses the ADR 0021 broker-pre-NS model. Upon
receiving a `SpawnRunner(gpu-render-node)` effect request (routed via
`MinijailProcessEffectPort` → core EffectPort adapter → privileged broker),
the **privileged broker** performs:

1. `OpenDevice` for the render node fd in the parent process (before
   `clone3(CLONE_NEWUSER)`).
2. `clone3(CLONE_NEWUSER | CLONE_PIDFD)` to create the child with a new user
   namespace.
3. Writes a single-entry `uid_map`/`gid_map`: in-NS UID/GID 0 → the
   allocator-assigned worker principal's stable host UID/GID (private broker
   state, not expressed in the resource spec).
4. Transfers the pre-opened render node fd to the child via the private
   fd-inheritance protocol before signaling ready.
5. crosvm receives the render device node via the inherited fd; the specific
   fd number and path form are private broker-layer state not expressed in the
   resource spec.
6. The crosvm process runs fake-root inside the user NS with **zero host
   capabilities**. No bind-mount is performed for the render node; the fd is
   passed via fd inheritance.

Source: `nixos-modules/minijail-profiles.nix` `gpu-render-node` profile (lines
490–545 approximately); `packages/d2b-priv-broker/src/sys.rs`
(`clone3_spawn_runner`); `packages/d2b-core/src/bundle_resolver.rs`
(test `gpu_render_node_user_namespace_propagates_to_resolved_intent` at line 4419).

**Key constraint**: no `deviceBinds` in the render-node profile. The render
node fd is pre-opened by the privileged broker and passed via private
fd-inheritance; no bind-mount action is executed for user-NS spawns.

**Shared arbitration**: When `arbitration: shared`, multiple Guest Processes
may each hold a `gpu-render-node` passthrough claim simultaneously. The device-gpu
controller creates one `device-<uid-short>-render-node` Process per active claim.
Provider/system-minijail sends a separate `SpawnRunner` effect request via
`MinijailProcessEffectPort` for each Process; the privileged broker opens a
separate render-node fd for each. The render node DRM device supports concurrent
unprivileged readers.

## Video decoder Process

### Process resource shape

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: device-<uid-short>-video
  zone: dev
  ownerRef: Device/corp-vm-gpu
  managedBy: controller
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: worker
  template: video-worker   # plain template ID matching ^[a-z][a-z0-9-]*$
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts, cgroup]
    capabilityClasses: []
    seccompClass: w1-video
    startRoot: false
    userNamespace: null   # video worker does not use a user namespace; tested invariant
  budget:
    cpu:
      request: "250m"
      limit: "2000m"
    memory:
      request: "128Mi"
      limit: "1Gi"
    pids:
      limit: 16
    fds:
      limit: 8
  networkUsage: null
  deviceUsage:
    - deviceRef: Device/corp-vm-gpu
      access: exclusive
      purpose: video-decode
  readiness:
    class: provider-defined
    initialDelay: "2s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
  restartPolicy:
    class: on-failure
    backoffBase: "5s"
    backoffMax: "5m"
    backoffMultiplier: 2
    maxRestarts: 10
    resetAfter: "1h"
```

The video worker produces the stable video sidecar Endpoint separately:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: corp-vm-video-sidecar
  zone: dev
  ownerRef: Device/corp-vm-gpu
spec:
  providerRef: Provider/device-gpu
  producerRef: Process/device-<uid-short>-video
  endpointClass: data
  transport: unix
  purpose: video-sidecar
  serviceFingerprint: device-gpu.d2bus.org/video-sidecar/v1
  locality: cross-domain
  visibility: provider-internal
  attachmentPolicy: launch-ticket-only
  consumerPolicy: [Provider/runtime-cloud-hypervisor]
  lifecyclePolicy: recycle-with-producer
```

The video and GPU workers carry distinct allocator-assigned principals, enforced
as a LaunchTicket invariant by Provider/system-minijail at resolution time and
carried in the effect request to the privileged broker. This invariant confines the
video worker to the `w1-video` seccomp class and its declared Device allowlist,
with no access to host Wayland, PipeWire, or Pulse sockets. Principal names are
private broker state and are not expressed in the resource spec.

### Dependency on GPU Process readiness

The video decoder Process is created only **after** the GPU worker Process
reaches phase `Ready`. The controller's reconcile loop enforces this:

1. Controller receives `spec-generation-changed` with `settings.videoSidecar=true`.
2. Controller creates `device-<uid-short>-gpu` Process.
3. Controller watches for `owned-resource-changed` on the GPU Process.
4. When GPU Process phase transitions to `Ready`, controller creates
   `device-<uid-short>-video` Process.
5. If the GPU Process fails or restarts, the video Process is stopped first,
   then re-created after the GPU Process is Ready again.

This ordering is tested in `tests/video_dependency.rs` (fake supervisor) and
`integration/video_dependency/` (container fixture).

### crosvm video-decoder sidecar argv

> **Current-source evidence** (`packages/d2b-host/src/video_argv.rs`,
> `implemented-and-reachable`). Runtime socket path and binary path are
> resolved by Provider/system-minijail from the signed LaunchTicket; they
> are private LaunchTicket state and do not appear in Process resource specs,
> status, audit, or telemetry.

```text
crosvm device video-decoder \
  --socket-path <video-socket-path> \
  --backend vaapi
```

Source: `packages/d2b-host/src/video_argv.rs` (implemented-and-reachable).

The video Process uses `VideoBackend::Vaapi` always. No other backend is
currently supported; the enum is open for future backends. The binary path
is resolved from the signed Provider descriptor; it points to a crosvm build
with `cargoBuildFeatures += [video-decoder, vaapi, media]`. The stock nixpkgs
crosvm binary is **never** used for the video sidecar; the controller fails
closed if the signed descriptor does not supply the patched binary path.
Source: `packages/d2b-core/src/bundle_resolver.rs`
`video_runner_has_no_stock_crosvm_legacy_fallback` (test at line 4235).

### Cloud Hypervisor vhost-user-media connection

Cloud Hypervisor connects to the video decoder via the `--vhost-user-media
socket=...` flag, appended by the CH runner Process via the signed component
descriptor. The private socket locator is resolved from
`Endpoint/<video-sidecar>` only through the authorized EffectPort/LaunchTicket
path and is not a spec field, status value, audit record, or telemetry
attribute.

### Broker Device allowlist (video worker)

The broker opens device fds exclusively from the declared Device allowlist.
No `/dev` path crosses the public wire. Maximum 8 fds per Process launch.

| Allowlist token | Always | Conditional |
| --- | --- | --- |
| `dri` (render node) | ✓ | |
| `nvidia-ctl` | | when `settings.videoNvidiaDecode=true` |
| `nvidia-device` | | when `settings.videoNvidiaDecode=true` |
| `nvidia-uvm` | | when `settings.videoNvidiaDecode=true` |

Source: `nixos-modules/minijail-profiles.nix` video profile `deviceBinds` list,
`lib.optionals (vm.graphics.videoNvidiaDecode or false)` guard.

The video profile uses **zero host capabilities** (`capabilities = []`),
`namespaces.pid = true`. The `sandbox.seccompClass` for this profile is
`w1-video`, enforced by `system-minijail` before exec; if the compiled BPF
policy is unavailable in the Provider closure, the launch fails closed.

### NVIDIA opt-in — explicit operator action required

NVIDIA hardware video decode (`nvidia-ctl`, `nvidia-device`, `nvidia-uvm`
allowlist tokens) is gated behind **two** explicit opt-in fields that must
both be `true`:

1. `settings.videoSidecar: true` — enable the video decoder Process.
2. `settings.videoNvidiaDecode: true` — add NVIDIA device tokens to the video
   worker's `deviceUsage[]` entries.

Neither field defaults to `true`. An operator who sets only `videoSidecar=true`
gets a video decoder with VA-API via Mesa virtio-GPU; NVIDIA device tokens are
**not** added unless `videoNvidiaDecode=true` is also set. Both fields must
appear explicitly in the Device spec and in the Nix configuration.

Source: `nixos-modules/minijail-profiles.nix` video profile
`lib.optionals (vm.graphics.videoNvidiaDecode or false)`.

### Denied host sockets (video worker)

The video worker does NOT receive Wayland, PipeWire, or PulseAudio socket
access:

- The `template: video-worker` descriptor declares no
  cross-domain or audio endpoint capability.
- The video minijail profile has no `bindMounts` for Wayland or audio sockets.
- The distinct allocator-assigned principal maintained by the LaunchTicket
  invariant ensures broker and activation ACLs can deny these sockets to the
  video worker without affecting the GPU worker.

This is a load-bearing architectural invariant. From
`AGENTS.md §Critical subsystems — handle with care` (Video sidecar): "The
video runner MUST use the dedicated `d2b-<vm>-video` principal, not
`d2b-<vm>-gpu`, so broker/activation ACLs can deny host Wayland/PipeWire/Pulse
sockets to video without breaking GPU cross-domain." The principal names are
private broker state; the invariant is enforced by the LaunchTicket, not
expressed in the resource spec.

## Frozen wire-contract constants

The following constants from `packages/d2b-host/src/video_argv.rs` are frozen
virtio-media wire contract values. Changing any of them requires updating the
CH patch `pkgs/spectrum-ch/cloud-hypervisor/0003-vhost-user-media-device.patch`
and a separate hardware-validated review:

| Constant | Value | Source in CH patch |
| --- | --- | --- |
| `VIRTIO_ID_MEDIA` | `48` | `const VIRTIO_ID_MEDIA: u32 = 48` |
| `VHOST_USER_MEDIA_NUM_QUEUES` | `2` | `const NUM_QUEUES: u16 = QUEUE_SIZES.len() as _` |
| `VHOST_USER_MEDIA_QUEUE_SIZE` | `256` | `const QUEUE_SIZES: &[u16] = &[256, 256]` |
| `VHOST_USER_MEDIA_SHM_REGION_BYTES` | `268435456` (256 MiB) | `VhostSharedMemoryRegion { length: 256 * 1024 * 1024 }` |
| `VHOST_USER_MEDIA_VRING_BASE` | `0` | `activate()` SET_VRING_BASE override |
| `VHOST_USER_MEDIA_PROTOCOL_FLAGS` | `BACKEND_REQ\|REPLY_ACK\|SHMEM_MAP_CROSVM` | `acked_protocol_features` |
| `VHOST_USER_MEDIA_MMIO_ALLOCATOR` | `pci-mem64` | PCI MMIO allocator for SHM region |

The `wire_contract_snapshot()` function in `video_argv.rs` renders these as a
deterministic single line; `tests/wire_constant_snapshot.rs` byte-compares the
output against a committed golden vector. Any future drift in the CH patch
surfaces as a CI golden diff before any argv change.

## Runtime Cloud Hypervisor dependency

### CH/crosvm compatibility contract

The device-gpu Provider has a hard dependency on the patched Cloud Hypervisor
build (`pkgs/spectrum-ch`) and the matching crosvm revision:

- **CH patch set**: `pkgs/spectrum-ch/cloud-hypervisor/0003-vhost-user-media-device.patch`
  introduces `vhost-user-media` with `virtio_id=48`. Any CH rev bump must
  update this patch and the frozen wire-contract constants above.
- **crosvm compatibility**: the vhost-user-gpu sidecar uses standardized
  vhost-user shmem message numbers (`GET_SHMEM_CONFIG=44`, `SHMEM_MAP=9`,
  `SHMEM_UNMAP=10`) matching `rust-vmm/vhost @ vhost-user-backend-v0.22.0`.
  The `spectrumCH.passthru.testedWithCrosvmRev` assertion in
  `nixos-modules/components/graphics.nix` enforces crosvm revision parity.
- **Eval guard**: if `spectrumCH.passthru.testedWithCrosvmRev ≠ pkgs.crosvm.src.rev`,
  the NixOS evaluation fails with a clear message before any hardware is touched.
- **v3 target**: this compatibility contract is preserved. The provider crate
  ships a `RuntimeCompatibilityDescriptor` that records the expected
  `testedWithCrosvmRev` digest; the controller validates it at startup against
  the installed CH package descriptor. A mismatch fails closed.

### seccomp enforcement

The `w1-gpu`, `w1-video`, and `w1-gpu-render-node` seccomp classes are enforced
by `system-minijail` before exec. If the compiled BPF policy for a class is
unavailable in the Provider closure, the launch fails closed; there is no
policy-less fallback and no runtime flag wiring is required.

## Process execution boundary

The `device-gpu` controller creates and manages Process resource records with
`deviceUsage[]` entries. It does **not** have broker authority or fd access of
any kind. It never calls `SpawnRunner`, `OpenDevice`, or any fd-inheritance
operation.

Provider/system-minijail, when processing a Process with `template: gpu-worker`,
`render-node-worker`, or `video-worker`, uses its injected **`MinijailProcessEffectPort`**
to request execution effects. The core EffectPort adapter maps opaque intents from
`MinijailProcessEffectPort` to broker requests. The **privileged broker alone**
performs `OpenDevice`, `clone3`, `uid_map`/`gid_map` writes, and FD transfer;
neither system-minijail nor the device-gpu controller has direct broker access.

### Effect operations (requested via MinijailProcessEffectPort; executed by privileged broker)

Provider/system-minijail sends the following effect requests through its injected
`MinijailProcessEffectPort`; the core EffectPort adapter routes them to the
privileged broker which executes them:

| Effect op | Effect | Audit | Rate limit |
| --- | --- | --- | --- |
| `SpawnRunner` (gpu role) | Privileged broker spawns crosvm GPU worker in broker-pre-NS | Yes | 1 per Device per Guest start cycle |
| `SpawnRunner` (gpu-render-node role) | Privileged broker spawns crosvm render-node worker in user NS via fd inheritance | Yes | 1 per active claim |
| `SpawnRunner` (video role) | Privileged broker spawns crosvm video-decoder Process | Yes | 1 per Device |
| `OpenDevice` (kvm, dri, udmabuf, nvidia*) | Privileged broker opens GPU device fds before clone; passes to worker | Yes | ≤8 fds per Process launch |

Source: `packages/d2b-contracts/src/broker_wire.rs` `RunnerRole::Gpu`,
`RunnerRole::Video`; `packages/d2b-core/src/bundle_resolver.rs` device token
sets (lines 1882–1894).

### No blanket device grant

No Provider process receives a blanket device-path grant, raw device node string,
or ambient host capability. The **privileged broker**:

1. Validates all inputs against the trusted bundle before any effect.
2. Opens fds in the parent process for GPU workers (before `clone3`).
3. Passes fds to the child via the private fd-inheritance protocol; no device
   path crosses the wire.

No device path crosses the public wire or any effect port as a string.

### User namespace pre-spawn (ADR 0021)

Full GPU workers run inside a broker-pre-established user namespace where:
- in-NS UID/GID 0 maps to the allocator-assigned worker principal's stable host UID.
- Device fds are pre-opened before `clone3(CLONE_NEWUSER)`.
- The worker has zero ambient host capabilities.

Source: ADR 0021; `nixos-modules/minijail-profiles.nix` gpu profile;
`packages/d2b-priv-broker/src/sys.rs` `clone3_spawn_runner` (privileged broker implementation).

The render-node-only worker always uses the user-NS model (ADR 0021).
The full-GPU worker uses it as well (no `userNamespace: null` exception;
the full GPU profile also transitions to broker-pre-NS in v3, aligned with
the render-node model).

## Status, conditions, and phase semantics

### Status shape

Per D088, ResourceType-common Device observation lives in
`status.resource`: the provider-neutral claim/arbitration/presence base that is
identical across Device implementations. GPU-specific observations (DRM/render
node mode and availability, worker refs/readiness, video sidecar and
wire-contract observations, bounded diagnostics) live only in `status.provider`
with `providerRef`, qualified `schemaId` `device-gpu.d2bus.org/Device/status`,
`schemaVersion`, `observedProviderGeneration`, and strict bounded redacted
`details`
(≤32 KiB, unknown-field-denied). The controller writes all present layers
atomically in one status mutation; shared fields are never duplicated
into `status.provider`, and the extension schema is registered and signed in the
Provider manifest.

### Currency and expedited reconcile (D091/D090)

D091 currency is universal status, not GPU provider detail. The controller
implements `assess_update`, `plan_upgrade`, and `execute_upgrade`, populates
universal `status.update`, and keeps shared currency fields out of
`status.provider`; GPU-specific observations may appear only under
`status.provider.details`. Driver/provider generation, artifact, spec, or
security-policy changes that require interrupting active claimants MUST set
`status.update.state = Blocked` while dependent `Process`/`Guest` resources are
running, then `UpgradeRequired` when an upgrade operation is planned, with
`reasons = [ProviderGenerationChanged]`, `[ArtifactChanged]`, `[SpecChanged]`,
or `[SecurityPolicyChanged]`, `disruption = Recycle`, and
`preserveState = true`. Non-disruptive changes reconcile normally. The
dependency-aware planner drains dependent Processes/Guests, recycles the GPU
realization, and restarts dependents; no surprise disruption is permitted and
device identity is preserved.

D090 expedited `waitForReconcile` on `Create`/`UpdateSpec`/`Delete` performs no
external effect, finalizer change, or status mutation until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. The
one-pass response returns the committed object, projected layered status,
disposition `Converged|Progressing|Blocked|UpgradeRequired|Failed`, and
`statusPersistence = pending|committed`; the durable commit is never rolled back
after a reconcile timeout. Effect idempotency keys derive from
`(UID,generation,revision,operationId)`, and the expedited pass uses the bounded
priority lane inside the same per-resource single-flight.

```yaml
status:
  observedGeneration: 1
  phase: Ready | Pending | Degraded | Failed | Unknown
  conditions:
    - type: DevicePresent
      status: "True" | "False" | "Unknown"
      reason: device-probed-present | device-not-found | device-consecutive-probe-failures-exceeded | device-probe-failed
    - type: DeviceClaimed
      status: "True" | "False"
      reason: exclusive-claim-held | no-active-claim
    - type: DeviceHealthy
      status: "True" | "False" | "Unknown"
      reason: worker-healthy | worker-failed | worker-not-started
    - type: GpuWorkerReady
      status: "True" | "False" | "Unknown"
      reason: gpu-worker-ready | gpu-worker-starting | gpu-worker-failed | gpu-worker-not-created
    - type: VideoWorkerReady
      status: "True" | "False" | "Unknown"
      reason: video-worker-ready | video-worker-starting | video-worker-failed | video-worker-not-created | video-sidecar-disabled
    - type: ClaimConflict
      status: "True" | "False"
      reason: exclusive-claim-conflict | no-conflict
    - type: GpuEffectAvailable
      status: "True" | "False"
      reason: gpu-effect-available | gpu-effect-unavailable
  lastReconciledAt: null
  resource:
    present: true | false | null
    health: healthy | degraded | failed | unknown
    holderRefs: []
    claims: []
    provisionedAt: null
    lastProbedAt: null
  provider:
    providerRef: Provider/device-gpu
    schemaId: "device-gpu.d2bus.org/Device/status"
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      gpu:
        mode: full | render-node
        renderNodeAvailable: true | false | null
        workerRefs: []
        videoWorkerRef: null
        wireContract: ok | failed | unknown
        providerDiagnostic: null  # bounded redacted one-line; never paths/secrets
```

### Phase semantics

| Phase | Meaning |
| --- | --- |
| `Pending` | Device spec committed; probe not yet complete or GPU effect port not yet available |
| `Ready` | Device present, claim held, GPU worker (and video worker if enabled) healthy |
| `Degraded` | One condition impaired (e.g., three consecutive probe failures, video worker failed but GPU still running) |
| `Failed` | Current spec generation cannot complete under retry policy (e.g., arbitration conflict, three probe failures with no device recovery) |
| `Unknown` | Controller cannot currently prove device state (e.g., first or second probe failure) |

### Condition types (GPU-specific)

| Type | Meaning |
| --- | --- |
| `DevicePresent` | DRM render node sysfs-visible |
| `DeviceClaimed` | At least one active claim held |
| `DeviceHealthy` | GPU worker (and video worker if enabled) are responsive |
| `GpuWorkerReady` | crosvm GPU/render-node Process in phase `Ready` |
| `VideoWorkerReady` | crosvm video-decoder Process in phase `Ready`; always `False` when `videoSidecar=false` |
| `ClaimConflict` | Exclusive Device received a second concurrent claim request |
| `GpuEffectAvailable` | Injected `GpuEffectPort` is available for device probing and execution |

## Errors

Stable GPU-specific error classes (subset of common device errors):

| Error | Meaning |
| --- | --- |
| `device-not-found` | DRM device absent from sysfs/udev at probe time |
| `device-claim-conflict` | Exclusive GPU device already claimed by another Guest |
| `device-arbitration-violation` | Shared claim attempted on `renderNodeOnly=false` Device |
| `shared-arbitration-requires-render-node-only` | `arbitration=shared` set with `renderNodeOnly=false` |
| `video-sidecar-requires-full-gpu` | `videoSidecar=true` with `renderNodeOnly=true` |
| `virgl-video-and-sidecar-mutually-exclusive` | Both `virglVideo=true` and `videoSidecar=true` on same Guest |
| `nvidia-decode-requires-video-sidecar` | `videoNvidiaDecode=true` without `videoSidecar=true` |
| `device-worker-failed` | GPU or video worker Process entered `Failed` phase |
| `device-wire-contract-mismatch` | Video or GPU wire constant diverged from frozen snapshot |
| `gpu-effect-unavailable` | Injected `GpuEffectPort` returned unavailable; device probe and worker launch cannot proceed |
| `render-node-fd-inheritance-failed` | Broker fd pre-open or private fd-inheritance transfer for render-node worker failed |
| `ch-crosvm-rev-mismatch` | `testedWithCrosvmRev` diverges from installed crosvm; controller refuses to spawn |
| `video-binary-not-found` | Signed descriptor missing patched video-decoder binary; fails closed |

All error messages are bounded, UTF-8 validated, and must not contain device
paths, sysfs node names, `/dev/dri/` entries, GPU socket paths, or credential
material.

## Audit records

Each broker operation emits a path-free audit record:

| Field | Value |
| --- | --- |
| `subject` | Provider Process identity digest |
| `zone` | Zone name |
| `op` | Broker op tag (`SpawnRunnerGpu`, `SpawnRunnerRenderNode`, `SpawnRunnerVideo`, `OpenDeviceGpu`) |
| `resource_type` | `Device` |
| `resource_name_digest` | SHA-256 of the Device resource name; never a raw path or device node |
| `outcome` | `success` \| `failure` \| `denied` |
| `error_class` | Stable error slug from the table above |
| `correlation_id` | Operation/trace ID |
| `timestamp` | RFC 3339 UTC |

The audit record excludes: raw GPU device paths, DRM card node names, render
node paths, GPU socket paths, CH argv, crosvm argv, nvidia device paths,
video socket paths, and any credential material.

## OTEL telemetry

All telemetry placement — span vs resource attribute classification,
`d2b.device.zone` cardinality, `d2b.device.provider` label level, and full
label set boundaries — is defined in `ADR-046-telemetry-audit-and-support`.
This dossier does not compete with those constraints.

GPU-specific label rules:

- No device path, DRM card node, render node path, PCI slot, GPU socket path,
  video socket path, or process PID may appear in any OTEL span attribute or
  metric label.
- `d2b.device.provider = device-gpu` is a fixed string label.
- `d2b.gpu.mode` ∈ `{full, render-node}` is a closed-set label (low cardinality).
- `d2b.gpu.video_sidecar` ∈ `{enabled, disabled}` is a closed-set label.
- `d2b.gpu.arbitration` ∈ `{exclusive, shared}` is a closed-set label.
- No `vm_name`, `guest_name`, or human-readable VM identifier appears in metric
  labels; VM identity is carried only in OTEL resource attributes and trace context.

## RBAC

The device-gpu Provider uses the standard Device RBAC roles from
`ADR-046-resources-device`:

| Role | Verbs | Scope | Subjects |
| --- | --- | --- | --- |
| `device-manager` | get, list, watch, create, update-spec, delete | Zone | `Provider/device-gpu` controller |
| `device-status-owner` | update-status | Zone | `Provider/device-gpu` controller only |
| `device-finalizer-owner` | update-finalizers | Zone | `Provider/device-gpu` controller only |
| `device-reader` | get, list, watch | Zone | Guest runtime Provider, CH runner, CLI |
| `device-claimant` | get, watch | Zone | Guest runtime Provider holding a GPU claim |

No Role grants wildcard `*` over all Device resources. The `device-gpu`
controller's RoleBinding covers only Device resources whose `spec.providerRef`
resolves to `Provider/device-gpu`.

The `device-gpu` controller also requires `process-manager` and
`process-status-reader` RBAC for the Process resources it creates (`device-<uid-short>-gpu`
and `device-<uid-short>-video`). It holds no other resource authority.

## Async reconcile loop

The controller implements the standard async reconcile contract from
`ADR-046-resource-reconciliation`. All reconciliation APIs are asynchronous.
A dedicated watch task reads while per-resource tasks reconcile in parallel.

### Reconcile triggers and handler actions

| Trigger | Handler |
| --- | --- |
| `spec-generation-changed` | Re-evaluate settings; update argv/broker-token set; if `renderNodeOnly` or `videoSidecar` changed, stop/restart affected Processes; update arbitration mode. |
| `deletion-requested` | Issue Delete on owned video Process (if running); issue Delete on GPU/render-node Process; wait for both to commit `phase=Deleted`; release OS resources; clear finalizer `device-gpu.d2bus.org/worker-stopped`. |
| `dependency-changed` | If owning Guest stops or degrades, release active claims; set phase `Degraded`. If GPU Process transitions to `Ready` and `videoSidecar=true`, create video Process. If GPU Process fails, stop video Process, update `VideoWorkerReady=False`. |
| `scheduled-observe` | Probe DRM sysfs presence; update `DevicePresent` condition; apply three-strike failure semantics. |
| `owned-resource-changed` | GPU Process phase changed: update `GpuWorkerReady`; trigger video dependency check. Video Process phase changed: update `VideoWorkerReady`; set `DeviceHealthy` accordingly. |

### Process fast-path

GPU and video worker Processes follow the standard Process fast-path:
commit-to-controller-handler ≤5 ms p95; launch-attempt ≤20 ms p95.
The device-gpu controller creates the Process resource; the system-minijail
Process controller manages the launch.

### Deletion sequence (finalizer `device-gpu.d2bus.org/worker-stopped`)

1. `deletionRequestedAt` set on Device resource.
2. Controller receives `deletion-requested` trigger.
3. Controller issues Delete requests on owned video Process (if present) and
   GPU/render-node Process; waits for their finalizers to clear.
4. Controller clears finalizer `device-gpu.d2bus.org/worker-stopped`.
5. Core commits one revision event with `phase=Deleted` for the Device, removes
   the resource row and all indexes atomically in one redb transaction, then
   emits the audit record after the commit. No `phase=Deleted` row persists in
   the store.

GPU device state (DRM card, render node) is physical; there is no persistent
emulated state to preserve or delete. The Device finalizer does not delete any
Volume resource. There is no force-cleanup path; the finalizer sequence is the
only deletion path.

## Provider state

### ProviderStateSet (query-time grouping)

Per D087, `device-gpu` declares **no Provider state Volume**. A
`ProviderStateSet(zone, device-gpu)` is the optional query-time grouping of
Provider state Volumes declared by the Provider descriptor; for this Provider it
is empty.

```text
ProviderStateSet(zone, device-gpu) = {}
```

GPU device state (DRM card, render node, video sidecar attachment) is physical
or Process/Device runtime state, not a durable Provider payload. GPU has no
Device-payload Volume; render-node access is a Device attachment resolved through
LaunchTicket and the privileged broker, not a Provider state Volume.

### Status-first operational state

Bounded non-secret operational state is written to the owning `Device.status`,
`Provider.status`, and the core Operation ledger. The common Device
claim/arbitration/presence summary lives in `status.resource`; GPU-specific
render-node/card availability, Process references, readiness observations,
finalizer progress, restart/adoption observations, and wire-contract check
results live in `status.provider.details.gpu`. Status is revisioned,
optimistic-status-writer controlled, RBAC-readable, redacted,
observation-only, bounded, and written only on material change. After restart,
the controller re-lists Device and Process resources, revalidates external
DRM/render-node and worker reality, and updates status; status never acts as
host-mutation or repair authority.

Worker Processes (`gpu-worker`, `render-node-worker`, `video-worker`) declare no
Provider state Volume and no `/state` mount. Their live fds and process-local
observations remain transient runtime data. Genuine runtime/device attachments
remain intact; only the former identity-only Provider state Volume is removed.

Storage-need test rationale: device-gpu operational state contains no durable
secret recovery payload, no large/binary/file content, no private data unsafe for
authorized status readers, and no bounded-but-revision-unsuitable data with a
demonstrated recovery need. Physical GPU state is reobserved, and claim/adoption
state is already represented by Device status and Operation rows.

## Nix configuration


### Authoring shape

```nix
# Full GPU, exclusive, with video sidecar
d2b.zones.<zone>.resources."<vm>-gpu" = {
  type = "Device";
  metadata.ownerRef = "Guest/<vm>";
  spec = {
    providerRef  = "Provider/device-gpu";
    deviceClass  = "physical";
    arbitration  = "exclusive";
    inventory.selector = {
      busClass = "drm";
      label    = "host-gpu";
    };
    provider = {
      schemaId = "device-gpu.d2bus.org/Device/spec";
      schemaVersion = "1.0.0";
      settings = {
        videoSidecar      = true;
        videoNvidiaDecode = false;    # must be explicit when videoSidecar=true
        contextTypes      = ["cross-domain" "virgl" "virgl2"];
        crossDomainTrusted = false;
      };
    };
  };
};

# Render-node only, shared (up to 4 concurrent Guests)
d2b.zones.<zone>.resources."<vm>-render" = {
  type = "Device";
  metadata.ownerRef = "Guest/<vm>";
  spec = {
    providerRef         = "Provider/device-gpu";
    deviceClass         = "physical";
    arbitration         = "shared";
    maxConcurrentClaims = 4;
    inventory.selector  = {
      busClass = "drm";
      label    = "host-gpu";
    };
    provider = {
      schemaId = "device-gpu.d2bus.org/Device/spec";
      schemaVersion = "1.0.0";
      settings = {
        renderNodeOnly = true;
        contextTypes   = ["virgl2"];
        egl            = true;
        vulkan         = false;
      };
    };
  };
};
```

`spec` field names, types, bounds, and defaults in Nix are identical to the
canonical ResourceTypeSchema. A `spec` field absent from the schema fails eval
with `invalid-provider-settings`.

### Eval-time assertions (GPU-specific)

Added to `nixos-modules/assertions.nix`:

| Assertion | Error |
| --- | --- |
| `settings.videoSidecar=true` requires `settings.renderNodeOnly=false` | `video-sidecar-requires-full-gpu` |
| `settings.virglVideo=true` and `settings.videoSidecar=true` on the same Guest | `virgl-video-and-sidecar-mutually-exclusive` |
| `settings.videoNvidiaDecode=true` requires `settings.videoSidecar=true` | `nvidia-decode-requires-video-sidecar` |
| `arbitration=shared` requires `settings.renderNodeOnly=true` for this Provider | `shared-arbitration-requires-render-node-only` |
| No two Device resources in the same Zone have the same `inventory.selector.label` | `duplicate-device-label` |
| Guest with `graphics.enable=true` must declare a GPU Device resource | `graphics-enable-requires-device-gpu-resource` |

### Canonical ResourceSpec JSON

**Full GPU, exclusive, with video sidecar:**

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Device",
  "metadata": {
    "name": "corp-vm-gpu",
    "ownerRef": "Guest/corp-vm",
    "zone": "dev"
  },
  "spec": {
    "arbitration": "exclusive",
    "deviceClass": "physical",
    "inventory": {
      "selector": {
        "busClass": "drm",
        "label": "host-gpu",
        "pciSlot": null
      }
    },
    "maxConcurrentClaims": 1,
    "providerRef": "Provider/device-gpu",
    "settings": {
      "contextTypes": ["cross-domain", "virgl", "virgl2"],
      "crossDomainTrusted": false,
      "displays": [{"hidden": true}],
      "egl": true,
      "renderNodeOnly": false,
      "videoNvidiaDecode": false,
      "videoSidecar": true,
      "virglVideo": false,
      "vulkan": true
    }
  }
}
```

**Render-node only, shared, up to 4 Guests:**

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Device",
  "metadata": {
    "name": "dev-vm-render",
    "ownerRef": "Guest/dev-vm",
    "zone": "dev"
  },
  "spec": {
    "arbitration": "shared",
    "deviceClass": "physical",
    "inventory": {
      "selector": {
        "busClass": "drm",
        "label": "host-gpu",
        "pciSlot": null
      }
    },
    "maxConcurrentClaims": 4,
    "providerRef": "Provider/device-gpu",
    "settings": {
      "contextTypes": ["virgl2"],
      "crossDomainTrusted": false,
      "displays": [],
      "egl": true,
      "renderNodeOnly": true,
      "videoNvidiaDecode": false,
      "videoSidecar": false,
      "virglVideo": false,
      "vulkan": false
    }
  }
}
```

Array elements sorted lexicographically. `null`-default optional selector fields
included explicitly. Absent settings fields receive schema defaults.

### Artifact catalog

The `d2b-provider-device-gpu` package closure is referenced from the Zone's
Provider resource via `spec.artifactId`:

```nix
d2b.artifacts.provider-device-gpu = {
  package = pkgs.d2b-provider-device-gpu;
  type    = "provider";
};
```

The Provider resource spec:

```nix
d2b.zones.<zone>.resources."provider-device-gpu" = {
  type = "Provider";
  spec = {
    artifactId = "provider-device-gpu";
    config.controllerExecutionRef = "Host/host-system";
  };
};
```

`spec.config.controllerExecutionRef` declares the execution context for the
controller Process. The `ProviderStateSet` for `Provider/device-gpu` is empty:
`device-gpu` declares no Provider state Volume, does not export `Volume` as a
ResourceType, and has no controller `/state` mount. Device resource status,
Provider status, and core Operation rows are the authority for bounded
non-secret operational state.

The Device spec carries **no** `artifactId` field. Binary paths for crosvm
(GPU and video) are resolved from the **signed component descriptor** inside the
`d2b-provider-device-gpu` package closure. Store paths never appear in Device
`spec.provider.settings`, resource status, audit records, or telemetry.

### Activation and publication path

1. NixOS build produces `/etc/d2b/zones/<zone>/resources.json` (root:d2bd 0640).
2. Zone runtime reads the bundle at startup and on generation-change signal.
3. Resources with `managedBy=configuration` absent from the new generation
   receive `deletionRequestedAt`; the Zone transitions to `Degraded/pending-cleanup`.
4. GPU Device resources are reconciled asynchronously; new workloads start
   immediately.

### Removed configured-resource cleanup

When a GPU Device resource is removed from the Nix config:

1. Zone activates new generation; Device gets `deletionRequestedAt`.
2. Device-gpu controller handles `deletion-requested` trigger:
   stops video Process, then GPU Process, clears finalizer.
3. Zone status transitions `Degraded/pending-cleanup` → `Ready` after the
   Device's `phase=Deleted` revision event is committed and the row is removed.
4. Associated controller-created Processes (`managedBy=controller`) are deleted
   by the Device finalizer, never by generation cleanup.
5. The guest-side virtio-media kernel module and CH `--vhost-user-media` arg
   remain in the Guest runtime Nix module until the Guest's next generation
   change.

## Crate and source layout detail

### `src/` — implementation

| File | Content |
| --- | --- |
| `lib.rs` | Crate entry; re-exports controller, worker binaries; Provider identity const |
| `controller.rs` | Async reconcile loop; ResourceClient; `spec-generation-changed`, `deletion-requested`, `dependency-changed`, `scheduled-observe`, `owned-resource-changed` handlers |
| `probe.rs` | `GpuEffectPort::probe_drm_device(selector)` caller; `observe_interval_secs` scheduler; three-strike failure counter |
| `arbitration.rs` | Exclusive vs shared claim check; `ClaimConflict` condition; second-claim rejection |
| `worker_gpu.rs` | Full GPU and render-node Process resource builder; argv construction via `d2b-host::gpu_argv`; broker token set |
| `worker_video.rs` | Video-decoder Process resource builder; argv construction via `d2b-host::video_argv`; NVIDIA opt-in gating; wire-contract check at startup |
| `argv.rs` | Re-export `GpuArgvInput`, `VideoArgvInput`, `GpuContextType` from `d2b-host`; no new argv logic |
| `broker.rs` | Device claim registration; tracks in-memory claim admission state per Device/Guest via `GpuEffectPort`; claim authority is `Device` resource `spec`/`status` (holderRefs, conditions) and the core Operation ledger managed via `ResourceClient` — no file-backed allocation table and no Volume writes; does **not** hold execution authority (Provider/system-minijail sends effect requests via `MinijailProcessEffectPort`; the core EffectPort adapter routes them to the privileged broker which executes them) |
| `status.rs` | `StatusWriter`; condition builder; phase state machine; bounded `providerDiagnostic` |
| `audit.rs` | Path-free `GpuAuditRecord` builder; correlation ID threading |
| `error.rs` | `DeviceGpuError` enum; closed-set slug strings |

### `tests/` — hermetic Cargo integration

| File | Coverage |
| --- | --- |
| `combined_reconcile.rs` | Full GPU+video state machine: probe → claim → gpu-Process Ready → video-Process Ready; fake broker and supervisor |
| `render_node_enforcement.rs` | `shared + renderNodeOnly=false` spec rejected at controller admission; `exclusive + renderNodeOnly=true` accepted |
| `wire_constant_snapshot.rs` | `video_argv::wire_contract_snapshot()` byte-matches committed golden string; any constant change surfaces here |
| `conformance.rs` | `DeviceSpec` and `DeviceGpuSettings` serde round-trip vs ResourceTypeSchema golden; unknown-field denial; `deviceUsage`, `budget`, `endpoints`, `readiness`, and `mounts` field names and structure validated against canonical schema |
| `arbitration_conflict.rs` | Second Guest attempts exclusive claim; controller writes `ClaimConflict` condition, sets second Device phase `Degraded` |
| `video_dependency.rs` | Video Process not created until GPU Process reaches `Ready`; video Process stopped when GPU Process fails |
| `seccomp_policy_ref.rs` | `sandbox.seccompClass` for gpu/video/render-node Process templates are `w1-gpu`, `w1-video`, `w1-gpu-render-node` (stable regression guard; confirms system-minijail enforces correct class before exec) |
| `status_state.rs` | Provider descriptor declares no Provider state Volume; ProviderStateSet query is empty; controller and worker Process templates have no `/state` mounts; no GPU Device-payload Volume exists; bounded operational observations are written to Device/Provider status and Operation rows; render-node access remains a Device attachment rather than Provider state |

### `integration/` — container/Host/Guest fixtures

| Path | Scenario |
| --- | --- |
| `gpu_worker_start/` | Full GPU worker Process obtains broker device tokens (`kvm`, `dri`, `udmabuf`), crosvm starts, Process reaches `Ready`. Requires x86_64 container host. |
| `render_node_shared/` | Two Guests each claim the same render-node Device with `arbitration=shared`; both worker Processes reach `Ready` simultaneously. Requires x86_64 + `/dev/dri/renderD128`. |
| `video_dependency/` | `videoSidecar=true`: video Process created only after GPU Process `Ready`; GPU Process crash causes video Process stop and re-sequenced restart. |
| `README.md` | How to invoke integration fixtures (`make test-integration`); hardware test note (see § Hardware tests). |

### `integration/README.md` content requirements

The `integration/README.md` file must include, at minimum:

1. How to run the container integration fixtures:
   ```
   make test-integration
   # or directly:
   cargo test -p d2b-provider-device-gpu --test integration
   ```
2. The host requirements for each fixture (x86_64, `/dev/dri/renderD128`,
   `/dev/kvm`).
3. The explicit statement that hardware-GPU and NVIDIA tests are **not** part
   of the container fixtures; they are manual-only and described in
   `tests/host-integration/hardware/` (see § Hardware tests below).
4. A note that the `render_node_shared` fixture uses a mock render node fd and
   does not require a live GPU; the `gpu_worker_start` fixture requires
   `/dev/dri/renderD128` to be present.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-device-gpu --lib --tests` completes in
≤2 s warm-cache execution time (compilation excluded). They use a deterministic
fake clock/RNG and the toolkit fakes/FakeEffectPort only — no process spawn,
container, network, DBus, systemd, broker daemon, Nix eval/build, KVM,
USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny temp
fixtures. Any scenario needing those lives only in `integration/`, which keeps
a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

## Hardware tests (explicit and manual only)

The following tests require a physical GPU and are **not** part of any
automated CI lane. They are manual-only and must be invoked explicitly:

```bash
# Requires: x86_64-linux host, /dev/dri/renderD128, KVM, running d2b Zone
D2B_LIVE=1 bash tests/host-integration/hardware/gpu-minijail-live.sh

# Requires: x86_64-linux, NVIDIA GPU, /dev/nvidiactl + /dev/nvidia-uvm
D2B_LIVE=1 bash tests/host-integration/hardware/gpu-nvidia-video-decode.sh

# Requires: full d2b host with a running Guest using a GPU Device
D2B_LIVE=1 bash tests/host-integration/hardware/gpu-virtio-guest-connect.sh
```

Hardware tests:
- Are gated on `D2B_LIVE=1`; without it they print a skip message and exit 0.
- Write evidence to `/var/lib/d2b/validated/gpu-<test-name>.json` after passing.
- Are never required for PR merge eligibility or automated CI.
- Require a deployed d2b host with the matching hardware.

The `integration/README.md` must document these tests and their hardware
requirements explicitly.

## ProcessRole disposition

| Current v3 ProcessRole | v3 resource target | Evidence class |
| --- | --- | --- |
| `ProcessRole::Gpu` (`packages/d2b-core/src/processes.rs`) | `Process/device-<uid-short>-gpu` owned by `Provider/device-gpu` | implemented-and-reachable |
| `ProcessRole::GpuRenderNode` (`packages/d2b-core/src/processes.rs`) | `Process/device-<uid-short>-render-node` owned by `Provider/device-gpu` | implemented-and-reachable |
| `ProcessRole::Video` (`packages/d2b-core/src/processes.rs`) | `Process/device-<uid-short>-video` owned by `Provider/device-gpu` | implemented-and-reachable |
| `d2b.vms.<vm>.graphics.*` Nix options | `d2b.zones.<zone>.resources.<name>` Device spec; settings mirror | generated-or-eval-contract |
| `nixos-modules/components/graphics.nix` | Subset of functionality moves to Guest `runtime-cloud-hypervisor` Nix module (CH arg injection, virglVideo crosvm patch, seccomp policies); Provider controller owns worker spawn | implemented-and-reachable (Nix) |
| `nixos-modules/components/video/guest.nix` | Guest-side `virtio_media` module, CH `--vhost-user-media` arg: stay in Guest `runtime-cloud-hypervisor` Nix module | generated-or-eval-contract |

No ProcessRole variant or Nix component is removed until the Provider/Process
resource successor integration is live and all current tests pass against the
new model. The removal condition is: `d2b-provider-device-gpu` crate
integration tests pass; GPU and video worker Processes reach `Ready` in a live
Zone; the NixOS eval test `device-gpu-eval.nix` passes; the ProcessRole
disposition contract test passes.

## Implementation work items

### ADR046-gpu-001: Create `d2b-provider-device-gpu` crate scaffold

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-001` |
| Dependency/owner | `ADR046-resources-device` accepted; `ADR046-provider-model-and-packaging` accepted; workspace root must add crate |
| Current source | No v3 source; `packages/d2b-host/src/gpu_argv.rs` and `packages/d2b-host/src/video_argv.rs` (implemented-and-reachable) provide argv generators |
| Reuse source | `packages/d2b-host/src/gpu_argv.rs` (baseline `b5ddbed`), `packages/d2b-host/src/video_argv.rs` (baseline `b5ddbed`) |
| Reuse action | extract |
| Destination | `packages/d2b-provider-device-gpu/` with `src/`, `tests/`, `integration/`, `README.md`; add to workspace `Cargo.toml` members list (alphanumerically sorted) |
| Detailed design | Crate scaffold: `Cargo.toml` with `d2b-host`, `d2b-contracts`, `d2b-provider-toolkit`, `d2b-core` dependencies; `lib.rs` exporting controller binary entry points; `error.rs` with `DeviceGpuError` closed-set enum; placeholder `controller.rs` Primary reuse disposition: `extract`. Preserved source-plan detail: `extract` both argv files into `d2b-provider-device-gpu/src/argv.rs` as re-exports; do not copy logic. |
| Integration | Workspace policy test must pass; crate must build; `src/`, `tests/`, `integration/`, `README.md` must exist |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `cargo build -p d2b-provider-device-gpu`; workspace policy crate-layout check passes |
| Removal proof | N/A (new crate) |

### ADR046-gpu-002: Implement async reconcile controller

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-002` |
| Dependency/owner | ADR046-gpu-001; `ADR-046-resource-reconciliation` implementation present; Provider toolkit `ResourceClient` available |
| Current source | `packages/d2bd/src/usbip_state_machine.rs` (implemented-and-reachable) as reconcile loop pattern reference. GPU/video reconcile state is `ADR-only`. |
| Reuse source | Pattern only: `packages/d2bd/src/usbip_state_machine.rs` (baseline). No code copy. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-device-gpu/src/controller.rs` |
| Detailed design | Five triggers: `spec-generation-changed`, `deletion-requested`, `dependency-changed`, `scheduled-observe`, `owned-resource-changed`. Each trigger handler writes optimistic `ResourceMutationBatch`. Status writer in `status.rs`. Async watch task + per-resource reconcile tasks. Independent resources in parallel. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` — implement the five-trigger reconcile loop using Provider toolkit async reconciler. |
| Integration | Resource API (ADR046 store) must be present; fake ResourceClient available from Provider toolkit; `tests/combined_reconcile.rs` validates trigger dispatch |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `cargo test -p d2b-provider-device-gpu --test combined_reconcile`; all five trigger handlers must reach their expected output state |
| Removal proof | Current ProcessRole::Gpu/Video/GpuRenderNode retained until this test passes; see ProcessRole disposition table |

### ADR046-gpu-003: DRM sysfs probe and observe scheduler

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-003` |
| Dependency/owner | ADR046-gpu-002 |
| Current source | `nixos-modules/assertions.nix` x86_64-linux guard; `packages/d2b-core/src/processes.rs` ProcessRole::Gpu/GpuRenderNode; no existing sysfs probe module |
| Reuse source | None; probe is `ADR-only` |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-gpu/src/probe.rs` |
| Detailed design | Call `GpuEffectPort::probe_drm_device(selector)` on each `scheduled-observe` trigger; the effect port resolves device presence against the trusted device table and returns a presence/health result without exposing raw sysfs or device paths to the controller. Three-strike failure counter; `observe_interval_secs` (10–60, default 30); emit `DevicePresent` condition and update `lastProbedAt`. |
| Integration | `scheduled-observe` trigger from reconcile loop calls `probe::check_drm_device` |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/conformance.rs` contains probe-mock path; `cargo test` passes |
| Removal proof | N/A (new module) |

### ADR046-gpu-004: Exclusive/shared arbitration enforcement

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-004` |
| Dependency/owner | ADR046-gpu-002 |
| Current source | `packages/d2b-core/src/bundle_resolver.rs` `validate_graphics_vm_invariants` (assertion guard) — `ADR-only` for resource-level arbitration |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-gpu/src/arbitration.rs` |
| Detailed design | On `spec-generation-changed` and each new claim: check `arbitration` vs `maxConcurrentClaims` vs current `holderRefs` length. Exclusive: reject any second claim with `ClaimConflict` condition, set requesting Device phase `Degraded`. Shared render-node: accept up to `maxConcurrentClaims`. Admission: `shared + renderNodeOnly=false` fails with `shared-arbitration-requires-render-node-only`. |
| Integration | Tested by `tests/arbitration_conflict.rs`; integration fixture `render_node_shared/` |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `cargo test -p d2b-provider-device-gpu --test arbitration_conflict`; `cargo test -p d2b-provider-device-gpu --test render_node_enforcement` |
| Removal proof | N/A (new module) |

### ADR046-gpu-005: GPU and render-node worker Process management

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-005` |
| Dependency/owner | ADR046-gpu-002; `ADR046-components-processes-and-sandbox` (Provider/system-minijail present and able to handle Process resources with `gpu-worker`/`render-node-worker` templates) |
| Current source | `packages/d2b-host/src/gpu_argv.rs` (implemented-and-reachable); `packages/d2b-core/src/bundle_resolver.rs` lines 1888–1894 (device token set); `packages/d2b-core/src/processes.rs` `ProcessRole::Gpu`, `ProcessRole::GpuRenderNode` (implemented-and-reachable); `nixos-modules/minijail-profiles.nix` gpu/gpu-render-node profiles (implemented-and-reachable) |
| Reuse source | `packages/d2b-host/src/gpu_argv.rs` (baseline `b5ddbed`): `GpuArgvInput`, `GpuParams`, `GpuContextType`, `GpuDisplayConfig`; `packages/d2b-core/src/bundle_resolver.rs` device token constant comment |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-device-gpu/src/worker_gpu.rs` |
| Detailed design | Build and commit `Process` resource record with `template: gpu-worker` or `template: render-node-worker`; set `sandbox.seccompClass` (`w1-gpu` or `w1-gpu-render-node`), `sandbox.userNamespace: {mappingClass: process-principal-root}` (uid/gid resolved privately by core from signed worker template — controller does NOT write numeric values), `sandbox.namespaceClasses`, `sandbox.capabilityClasses=[]`, `sandbox.startRoot=false`; set `deviceUsage[{deviceRef,access,purpose}]`, `networkUsage: null`, `endpoints[{name,transport,purpose}]`, `budget` (including `pids` and `fds` bounded limits), `readiness` (with `class`, `initialDelay`, `timeout`, `failureThreshold`, `successThreshold`), and `restartPolicy` (with `class`, `backoffBase`, `backoffMax`, `backoffMultiplier`, `maxRestarts`, `resetAfter`). Provider/system-minijail validates and resolves the LaunchTicket and sends effect requests via `MinijailProcessEffectPort`; the core EffectPort adapter routes them to the **privileged broker** which performs `SpawnRunner`, `OpenDevice`, `clone3`, `uid_map`/`gid_map` writes, and fd transfer — the device-gpu controller does not have execution authority or fd access. `crossDomainTrusted` gating: the signed descriptor is static; `crossDomainTrusted` is projected from the Device setting into the LaunchTicket by Provider/system-minijail, which omits `GpuContextType::CrossDomain` from runtime argv when false. Primary reuse disposition: `adapt`. Preserved source-plan detail: `extract` argv builder logic into `argv.rs` as re-export from `d2b-host` (used by Provider/system-minijail at LaunchTicket resolution time; the signed component descriptor is static and is not rewritten per Device); `adapt` device allowlist token set from `bundle_resolver.rs` into `worker_gpu.rs` `GPU_DEVICE_ALLOWLIST` constant for `deviceUsage` population. |
| Integration | `integration/gpu_worker_start/`; `integration/render_node_shared/`; `packages/d2b-contract-tests/tests/minijail_gpu.rs` (reused existing test) |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `cargo test -p d2b-provider-device-gpu`; `cargo test -p d2b-contract-tests --test minijail_gpu` continues to pass |
| Removal proof | `ProcessRole::Gpu` and `ProcessRole::GpuRenderNode` removed from `processes.rs` only after both integration tests pass and the ProcessRole disposition contract test confirms zero remaining references |

### ADR046-gpu-006: Video decoder Process management and wire-contract check

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-006` |
| Dependency/owner | ADR046-gpu-005 (video depends on GPU Process being Ready) |
| Current source | `packages/d2b-host/src/video_argv.rs` (implemented-and-reachable): `VideoArgvInput`, `VideoBackend`, `wire_contract_snapshot()`; `packages/d2b-contract-tests/tests/video_binary_contract.rs` (implemented-and-reachable); `packages/d2b-contract-tests/tests/minijail_swtpm_video.rs` video section (implemented-and-reachable); `nixos-modules/minijail-profiles.nix` video profile (implemented-and-reachable) |
| Reuse source | `packages/d2b-host/src/video_argv.rs` (baseline `b5ddbed`): argv generator, wire-contract constants, `wire_contract_snapshot()` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-device-gpu/src/worker_video.rs`, `tests/wire_constant_snapshot.rs` |
| Detailed design | Controller creates `Process/device-<uid-short>-video` only after `GpuWorkerReady=True`. `worker_video.rs` builds `VideoArgvInput` from resolved device spec and signed descriptor binary path. Validates `wire_contract_snapshot()` matches committed golden at startup; fails closed if mismatch (error `device-wire-contract-mismatch`). NVIDIA device gating: include `nvidia-ctl`, `nvidia-device`, `nvidia-uvm` tokens in `deviceUsage[]` entries only when `videoNvidiaDecode=true`; the **privileged broker** opens the fds when executing the effect request from the core EffectPort adapter. Distinct allocator-assigned principal enforced by LaunchTicket (internal invariant; not expressed in the resource spec); `template: video-worker` descriptor declares no Wayland/audio endpoint capability. `sandbox.seccompClass: w1-video`; `sandbox.namespaceClasses` includes `pid`; `userNamespace: null` (explicit, tested invariant). Primary reuse disposition: `adapt`. Preserved source-plan detail: `extract` argv generator (re-export from `argv.rs`); `copy-unchanged` wire-contract constants into `tests/wire_constant_snapshot.rs` golden comparison. |
| Integration | `integration/video_dependency/`; `packages/d2b-contract-tests/tests/video_binary_contract.rs` (reused); `packages/d2b-contract-tests/tests/minijail_swtpm_video.rs` video section (reused) |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `cargo test -p d2b-provider-device-gpu --test video_dependency`; `cargo test -p d2b-provider-device-gpu --test wire_constant_snapshot`; `cargo test -p d2b-contract-tests --test video_binary_contract` continues to pass |
| Removal proof | `ProcessRole::Video` removed from `processes.rs` only after `integration/video_dependency/` passes and the video Process reaches `Ready` in a live Zone |

### ADR046-gpu-007: Nix option migration and eval validation

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-007` |
| Dependency/owner | ADR046-gpu-001; ADR 0046 Zone Nix emitter wired; `ADR-046-nix-configuration` Nix emitter present |
| Current source | `nixos-modules/options-realms-workloads.nix` `d2b.vms.<vm>.graphics.*` options (generated-or-eval-contract); `nixos-modules/assertions.nix` graphics assertions; `nixos-modules/components/graphics.nix` (host-side); `nixos-modules/components/video/guest.nix` (guest-side) |
| Reuse source | Settings schema field names/defaults/bounds from `nixos-modules/options-realms-workloads.nix` options documentation |
| Reuse action | adapt |
| Destination | `nixos-modules/assertions.nix` (new GPU Device eval assertions); `tests/unit/nix/cases/device-gpu-eval.nix` (new Nix eval case); committed settings schema `docs/reference/schemas/v3/providers/device-gpu.settings.json` |
| Detailed design | Eval assertions as documented in § Nix configuration / Eval-time assertions. Canonical JSON golden as documented. Settings schema drift gate via `make test-drift`. `d2b.vms.<vm>.graphics.*` options are deprecated (emit deprecation warning) until a transition generation removes them; they are not removed in the same commit that adds the Device spec option. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` — map old `d2b.vms.<vm>.graphics.*` fields to `d2b.zones.<zone>.resources.<name>` Device spec settings fields; add eval assertions. |
| Integration | `nix flake check`; `tests/unit/nix/cases/device-gpu-eval.nix`; `make test-drift` |
| Data migration | Consumer config migration guide: replace `d2b.vms.<vm>.graphics.enable = true` with a Device resource declaration. Old options emit deprecation warnings, not hard failures, during the transition window. |
| Validation | `nix-unit tests/unit/nix/cases/device-gpu-eval.nix`; `make test-drift`; `make test-flake` |
| Removal proof | `d2b.vms.<vm>.graphics.*` options removed only after migration guide ships and the deprecation warning has been live for one minor release |

### ADR046-gpu-008: Assert status-first Provider state

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-008` |
| Dependency/owner | ADR046-gpu-001; D087 status-first state model present in the foundational ADR-046 specs |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-gpu/` component descriptor; controller/status tests |
| Detailed design | Do **not** declare a controller Provider state Volume. The device-gpu component descriptor declares an empty ProviderStateSet; controller and worker Process templates contain no `/state` mount. Bounded non-secret operational state is published to Device/Provider status and the core Operation ledger. GPU has no Device-payload Volume; render-node access remains a Device attachment resolved by LaunchTicket and broker policy. Primary reuse disposition: `create`. Preserved source-plan detail: `new` — status-first state assertions in the component descriptor and controller tests. |
| Integration | `tests/status_state.rs`; `integration/gpu_worker_start/` verifies controller startup is gated by resource dependencies and status writer authority, not by a Provider state Volume |
| Data migration | None — no Provider state Volume exists to migrate. |
| Validation | `cargo test -p d2b-provider-device-gpu --test status_state`; component descriptor golden has no Provider state Volume declaration; controller Process template has no `/state` mount; ProviderStateSet query is empty; status/core-ledger fields carry bounded operational observations |
| Removal proof | `StorageRoot`/`StoragePathSpec` lifecycle tracking entries for GPU/video roles in `d2b-core/src/storage.rs` removed after Device/Process status-first lifecycle and restart-adoption integration tests pass in a live Zone |

### ADR046-gpu-009: Provider `README.md`


| Field | Value |
| --- | --- |
| Work item ID | `ADR046-gpu-009` |
| Dependency/owner | ADR046-gpu-001 |
| Current source | None; new file |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-gpu/README.md` |
| Detailed design | Must include: Provider identity, supported ResourceTypes, controller/service/worker binary descriptions, placement (Host, system domain), dependencies (system-minijail, volume-local, observability-otel), RBAC roles, security model summary, state/telemetry contract, build command (`cargo build -p d2b-provider-device-gpu`), test commands (`cargo test -p d2b-provider-device-gpu`), integration command (`make test-integration`), hardware test note (see `integration/README.md`), standalone-repository consumption stub. |
| Integration | Workspace policy checks for `README.md` presence |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `make test-policy` (workspace crate layout policy check) |
| Removal proof | N/A (new file) |

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | **GPU/video process roles**: `packages/d2b-core/src/processes.rs` `ProcessRole::Gpu`, `ProcessRole::GpuRenderNode`, `ProcessRole::Video` (`implemented-and-reachable`). **GPU argv**: `packages/d2b-host/src/gpu_argv.rs` `GpuArgvInput`, `GpuParams`, `GpuContextType`, `GpuDisplayConfig` (`implemented-and-reachable`). **Video argv + wire constants**: `packages/d2b-host/src/video_argv.rs` `VideoArgvInput`, `VideoBackend`, `wire_contract_snapshot()`, all `VHOST_USER_MEDIA_*` constants (`implemented-and-reachable`). **GPU device token set**: `packages/d2b-core/src/bundle_resolver.rs` lines 1882–1894, ProcessRole::Gpu/GpuRenderNode arm (`implemented-and-reachable`). **Minijail profiles**: `nixos-modules/minijail-profiles.nix` gpu, video, gpu-render-node profiles with device binds, seccomp refs, user NS config (`implemented-and-reachable`). **Broker ops**: `packages/d2b-contracts/src/broker_wire.rs` `RunnerRole::Gpu`, `RunnerRole::Video` (`implemented-and-reachable`). **Nix host graphics**: `nixos-modules/components/graphics.nix` (crosvm wrapper, virglVideo patch, CH rev guard, crossDomainTrusted enforcement) (`implemented-and-reachable`). **Nix guest video**: `nixos-modules/components/video/guest.nix` (`virtio_media` module, CH `--vhost-user-media` arg) (`generated-or-eval-contract`). **Contract tests**: `packages/d2b-contract-tests/tests/minijail_gpu.rs`, `minijail_swtpm_video.rs`, `video_binary_contract.rs` (`implemented-and-reachable`). **Provider crate**: `packages/d2b-provider-device-gpu/` (`ADR-only`). |
| Evidence class | GPU/video process role enum and argv generators: `implemented-and-reachable`. GPU device token set and minijail profiles: `implemented-and-reachable`. Broker RunnerRole::Gpu/Video: `implemented-and-reachable`. CH/crosvm version guard: `implemented-and-reachable`. Video wire-contract constants: `implemented-and-reachable`. Device ResourceType schema: `ADR-only`. Provider crate and reconcile loop: `ADR-only`. |
| Behavior retained | GPU device allowlist token set (kvm/dri/udmabuf/nvidia*); video wire-contract constants frozen; distinct allocator-assigned video vs GPU worker principal (LaunchTicket invariant; private broker state); render-node fd pre-opened by the **privileged broker** and inherited via private fd-inheritance protocol; user-namespace zero-host-caps (ADR 0021); no Wayland/audio sockets for video role; EndpointRef-based cross-domain trust projected from Device setting into LaunchTicket at resolution time; argv builder omits CrossDomain from runtime args when false; NVIDIA opt-in gating for video; CH/crosvm rev compatibility guard; `videoSidecar` + `videoNvidiaDecode` mutual independence; `virglVideo` + `videoSidecar` mutual exclusion. |
| Required delta | `d2b-provider-device-gpu` crate, async reconcile controller, Device ResourceType schema for GPU settings, Provider resource registration, process-name templates from Device UID, wire-contract check at startup, shared render-node arbitration enforcement, generation-based lifecycle via Zone resource plane; D087 status-first state assertion in the component descriptor — no Provider state Volume, empty ProviderStateSet, no controller `/state` mount, bounded operational state in status and Operation rows (ADR046-gpu-008). |
| Reuse path | Re-export `gpu_argv.rs` and `video_argv.rs` from `d2b-host` unmodified. Adapt device token set constant from `bundle_resolver.rs` into `worker_gpu.rs` `GPU_DEVICE_ALLOWLIST` for `deviceUsage` population. Adapt minijail profile field names to `Process` resource spec fields. uid/gid mapping is resolved privately by core from the signed worker template — the device-gpu controller does not write hostUid/hostGid into any resource spec field. |
| Replacement/deletion | `ProcessRole::Gpu`, `ProcessRole::GpuRenderNode`, `ProcessRole::Video` in `processes.rs` retained until Provider integration parity. `d2b.vms.<vm>.graphics.*` Nix options deprecated (with warning) until consumer migration window closes. Nix `components/graphics.nix` host-side worker-spawn logic removed after `worker_gpu.rs` is live; CH arg injection and crosvm patches stay in Guest runtime Nix module. `StorageRoot`/`StoragePathSpec` entries for GPU/video roles in `d2b-core/src/storage.rs` removed after status-first Device/Process lifecycle integration passes. |
| Feasibility proof | GPU worker process broker token set: `packages/d2b-contract-tests/tests/minijail_gpu.rs` (existing, reachable). Video wire-contract constant snapshot: `packages/d2b-host/src/video_argv.rs` `wire_contract_snapshot()` + `tests/video_binary_contract.rs` (existing, reachable). Render-node user-NS propagation: `packages/d2b-core/src/bundle_resolver.rs` test `gpu_render_node_user_namespace_propagates_to_resolved_intent` (existing, reachable). |
| Future owner | `packages/d2b-provider-device-gpu/` crate; work items ADR046-gpu-001 through ADR046-gpu-009 |
