# ADR 0046 Provider: runtime-qemu-media

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-runtime-qemu-media` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `runtime-qemu-media` Provider crate, QEMU media lifecycle |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-volume`, `ADR-046-resources-network`, `ADR-046-resources-device`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-resource-reconciliation`, `ADR-046-current-code-migration-map` |
| Supersedes | `docs/adr/0036-qemu-media-runtime.md` |

---

## Table of contents

1. [Identity and crate](#1-identity-and-crate)
2. [Provider ResourceSpec](#2-provider-resourcespec)
3. [Provider config schema](#3-provider-config-schema)
4. [Guest ResourceSpec](#4-guest-resourcespec)
5. [Guest `spec.provider.settings` schema](#5-guest-specprovidersettings-schema)
6. [Volume resources](#6-volume-resources)
7. [Network dependency](#7-network-dependency)
8. [Device dependencies](#8-device-dependencies)
9. [WaylandSession dependency (display-wayland)](#9-waylandsession-dependency)
10. [Process templates](#10-process-templates)
11. [Controller reconcile and finalize loop](#11-controller-reconcile-and-finalize-loop)
12. [Guest boot sequence](#12-guest-boot-sequence)
13. [QMP protocol via Endpoint resource](#13-qmp-protocol-via-endpoint-resource)
14. [Broker operations](#14-broker-operations)
15. [RBAC and permission claims](#15-rbac-and-permission-claims)
16. [Status, phases, and conditions](#16-status-phases-and-conditions)
17. [Audit events](#17-audit-events)
18. [Telemetry and metrics](#18-telemetry-and-metrics)
19. [Nix configuration](#19-nix-configuration)
20. [ProviderStateSet](#20-providerstateset)
21. [Implementation work items](#21-implementation-work-items)
22. [Tests](#22-tests)
23. [Removal proofs and migration map](#23-removal-proofs-and-migration-map)

---

## 1 Identity and crate

```text
Provider/runtime-qemu-media
```

**Crate:** `packages/d2b-provider-runtime-qemu-media/`

**Dossier:** `docs/specs/providers/ADR-046-provider-runtime-qemu-media.md` (this file)

**Implements:** `Guest` — one Guest resource per QEMU virtual machine that boots
removable or physical-block media under QMP supervision.

**Does not implement:** Process, EphemeralProcess, Volume, Network, Device,
WaylandSession, audio, or clipboard. This Provider owns the QEMU VMM worker
process only. All display proxy work is delegated to `Provider/display-wayland`
via `WaylandSession` resource dependency.

**Relation to baseline ADR 0036 (`docs/adr/0036-qemu-media-runtime.md`):**
ADR 0036 established the manual-only posture, QMP-mediated attach/detach
protocol, and the `QemuMedia*` broker op family. This Provider supersedes
that design. The broker op family is retired; media is delivered through
Volume virtio-blk attachments. The manual-only posture is preserved as a
typed `Guest.status.provider.details.providerPhase = "paused-at-boot"` state.

**Required crate layout** (workspace policy gate `make test-policy`):

```
packages/d2b-provider-runtime-qemu-media/
  src/          # controller, runner binaries; colocated unit tests
  tests/        # hermetic Cargo integration, conformance, schema, fault tests
  integration/  # container/Host/Guest cross-process fixtures (at least one .rs source file)
  README.md     # min 200 bytes; Provider identity, config, ResourceTypes,
                # controllers, workers, binaries, placement, dependencies,
                # RBAC, security, state lifecycle, telemetry, build/test commands
```

A nested `integration/README.md` is optional; the workspace policy requires only
the four top-level paths above.

---

## 2 Provider ResourceSpec

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: runtime-qemu-media
  zone: corp
  ownerRef: null
spec:
  artifactId:    runtime-qemu-media   # plain bounded ID; resolves in d2b.artifacts
  config:
    controllerExecutionRef: Host/host-system
    qemuBinaryArtifactId:   qemu-system-x86_64
    qmpReadyTimeoutSeconds: 30
    qmpOperationTimeoutSeconds: 60
    pausedAtBootDefault:    true
    displayProviderRef:     Provider/display-wayland     # optional
    networkProviderRef:     Provider/network-local       # required
    volumeProviderRef:      Provider/volume-local        # required
    runtimeTmpfsQuotaBytes: 10485760   # 10 MiB
    runtimeTmpfsQuotaInodes: 1024
status:
  phase: Pending
```

`config` is validated against the Provider's signed JSON Schema before the
Provider resource reaches `Ready`. `controllerExecutionRef` is required; the
controller Process runs on `Host/<name>` declared by this field. All other
`config` values are projected to the controller component only; worker
processes receive no root config, no ResourceAPI authority, and no d2b-bus
authority.

`packageDigest` is populated by the Nix compiler from the artifact catalog.
It is never specified directly in Nix.

---

## 3 Provider config schema

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `controllerExecutionRef` | ResourceRef | **yes** | — | `Host/<n>` | Host on which the runtime-qemu-media controller Process runs |
| `qemuBinaryArtifactId` | string | yes | `"qemu-system-x86_64"` | `^[a-z][a-z0-9-]*$` | Artifact catalog ID for the QEMU binary closure |
| `qmpReadyTimeoutSeconds` | u32 | no | `30` | 5–300 | Deadline for initial QMP greeting after process start |
| `qmpOperationTimeoutSeconds` | u32 | no | `60` | 5–300 | Per-QMP-command timeout |
| `pausedAtBootDefault` | bool | no | `true` | — | Default `pauseAtBoot` if not set in Guest spec.provider.settings |
| `displayProviderRef` | ResourceRef? | no | `null` | `Provider/<n>` | Provider for WaylandSession resources; required when any Guest sets `displayWindow: true` |
| `networkProviderRef` | ResourceRef | yes | — | `Provider/<n>` | Network Provider for tap/bridge delivery |
| `volumeProviderRef` | ResourceRef | yes | — | `Provider/<n>` | Volume Provider for media and runtime volumes |
| `runtimeTmpfsQuotaBytes` | u64 | no | `10485760` | 1 MiB–256 MiB | Per-Guest runtime tmpfs size cap |
| `runtimeTmpfsQuotaInodes` | u32 | no | `1024` | 64–65536 | Per-Guest runtime tmpfs inode cap |

`qemuBinaryArtifactId` resolves to a `d2b.artifacts.<id>` entry with
`type = "provider"` or `type = "config-bundle"` containing the QEMU binary
closure. The resolved store path is consumed internally at launch time and
never appears in any public resource spec, status field, audit record, or
OTEL telemetry.

---

## 4 Guest ResourceSpec

A `Guest` resource managed by `runtime-qemu-media` has the following shape:

```yaml
apiVersion: resources.d2bus.org/v3
type: Guest
metadata:
  name: corp-iso-boot
  zone: corp
  ownerRef: null
  finalizers:
    - runtime-qemu-media.d2bus.org/guest-cleanup
spec:
  providerRef: Provider/runtime-qemu-media
  systemArtifactId: null                    # no NixOS guest system; media-boot only
  vcpu: 4
  memoryMib: 8192
  networkAttachments:
    - networkRef: Network/corp-net
      macAddress: null                      # null → stable-derived by controller
      ipv4Address: null                     # null → DHCP
  deviceAttachments:
    - deviceRef: Device/host-kvm            # KVM acceleration; explicit required dependency
      exclusive: false
  volumeDefaults: {}
  provider:                                  # see §5
    schemaId: runtime-qemu-media.d2bus.org/Guest/spec
    schemaVersion: 1.0.0
    settings:
      bootMediaRef: Volume/corp-iso-boot-media
      bootMediaView: guest-attach
      removableVolumeRefs:
        - volumeRef:  Volume/corp-usb-stick
          view:       guest-attach
      cpuModel: host
      machineType: q35
      bios: ovmf
      pauseAtBoot: true
      displayWindow: false
      serialConsole: true
      tablet: true
      rtcBase: utc
      extraFeatures: []
status:
  phase: Pending
  resource:
    observedLifecyclePhase: pending
    runtimeReady: false
  provider:
    providerRef: Provider/runtime-qemu-media
    schemaId: runtime-qemu-media.d2bus.org/Guest/status
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      providerPhase: ""
```

`spec.systemArtifactId` is `null` for media-boot Guests; no NixOS guest system
closure is required. For Guests that also need a guest NixOS closure
(rare mixed-media case), `systemArtifactId` can refer to a `"nixos-system"`
artifact, but the initial release supports `null` only.

`spec.deviceAttachments[*].deviceRef: Device/host-kvm` is required for every
`runtime-qemu-media` Guest that uses hardware KVM acceleration. The
controller watches `Device/host-kvm` status asynchronously and gates the
runner launch until the Device is `Ready`. If `Device/host-kvm` is absent or
`Failed`, the controller sets `phase: Degraded` with condition `DeviceReady`
False and reason `kvm-device-unavailable`.

---

## 5 Guest `spec.provider.settings` schema

`spec.provider.settings` is a bounded map validated against the Provider's
signed JSON Schema. It contains Guest-level implementation settings for the VMM;
no raw paths, executable paths, argv fragments, or credential bytes appear here.

**D089 spec extension contract:** this Provider's implementation-only desired
configuration is carried in `spec.provider.settings` under
`runtime-qemu-media.d2bus.org/Guest/spec`; the schema is registered/signed in the
manifest, deny-unknown, bounded, versioned, and validated against
`spec.providerRef` at Nix build and API admission. Base fields stay at `spec.*`;
shared semantics are promoted to the Guest base and never placed in
`spec.provider`. This Provider implements the exact base spec/status schema
version/fingerprint, accepts the canonical minimal valid base Spec, and rejects
an unsupported optional base capability only through its signed capability matrix
plus provider-neutral `unsupported-capability`. `spec.provider` aligns with
`status.provider` for `Provider/runtime-qemu-media`.

`vcpu` and `memoryMib` are promoted Guest base fields (`spec.vcpu` and
`spec.memoryMib`); they are not Provider extension fields.

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `bootMediaRef` | ResourceRef? | no | `null` | `Volume/<n>` | Primary boot Volume; nil = direct kernel boot if kernelArtifactId set (not yet supported) |
| `bootMediaView` | string | no | `"guest-attach"` | `^[a-z][a-z0-9-]*$` | View within the boot Volume from which the controller derives the virtio-blk attachment |
| `removableVolumeRefs` | list | no | `[]` | max 4 entries | Runtime-hotpluggable media Volumes |
| `removableVolumeRefs[].volumeRef` | ResourceRef | yes | — | `Volume/<n>` | Removable media Volume |
| `removableVolumeRefs[].view` | string | yes | — | `^[a-z][a-z0-9-]*$` | View within the Volume for guest access |
| `cpuModel` | string | no | `"host"` | `host\|max\|qemu64` | CPU model string; sealed set |
| `machineType` | string | no | `"q35"` | `q35\|pc` | QEMU machine type |
| `bios` | string | no | `"ovmf"` | `ovmf\|seabios` | Firmware type |
| `pauseAtBoot` | bool | no | `true` | — | If true, start QEMU in `\-S` mode (paused); operator issues QMP `cont` to release |
| `displayWindow` | bool | no | `false` | — | If true, controller creates a `WaylandSession` resource for `Provider/display-wayland` |
| `serialConsole` | bool | no | `true` | — | Expose serial console via owned Endpoint resource |
| `tablet` | bool | no | `true` | — | USB tablet input device (absolute pointer for Wayland) |
| `rtcBase` | string | no | `"utc"` | `utc\|localtime` | RTC base |
| `extraFeatures` | list\<string\> | no | `[]` | closed enum | Reserved; only values in Provider's signed capability descriptor are accepted |

### Media Volume refs

`bootMediaRef` and every `removableVolumeRefs[].volumeRef` must be Volume
resources in the same Zone as the Guest. The controller watches the referenced
Volumes for `Ready` status before constructing the runner's LaunchTicket. The
Volume must have:

- a `virtio-blk` attachment for the Guest (`executionRef: Guest/<name>`); or
- a declared view with `rights` including `read` (for read-only boot image);
  optionally `write` for writable removable media.

The Volume controller/broker opens the backing file or block device and
delivers the fd to the runner via the LaunchTicket's inherited fd table. No
raw host path for any media file crosses the public Provider surface, Process
spec, Process status, or audit event.

---

## 6 Volume resources

### 6.1 Runtime tmpfs Volume (controller-created)

The controller creates one runtime Volume per Guest. This Volume is
ephemeral, lives in the per-Guest cgroup domain, and holds the QEMU Unix
socket endpoints (QMP, serial) and any small runtime scratch state.

```yaml
apiVersion: resources.d2bus.org/v3
type: Volume
metadata:
  name: <guest-uid-short>-runtime
  zone: corp
  ownerRef: Guest/corp-iso-boot     # controller sets ownerRef to the owning Guest
  finalizers:
    - runtime-qemu-media.d2bus.org/runtime-volume
spec:
  providerRef: Provider/volume-local
  source:
    executionRef: Host/host-system   # same host as controllerExecutionRef
    settings:
      kind: tmpfs
      sourcePolicyId: runtime-qemu-media-runtime-tmpfs
  kind: ephemeral
  layout:
    - path: ""
      type: directory
      ownerRef: User/d2b-system
      groupRef: User/d2b-system
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-absent
      repairPolicy: exact-owner
      cleanupPolicy: vm-stop-with-proof
      adoptionPolicy: quarantine-on-ambiguity
      restartPolicy: preserve-across-controller-restart
      leaseClass: process-pidfd
      invariants: [no-symlink, no-magic-link]
    - path: qmp.sock
      type: unix-socket
      ownerRef: User/d2b-system
      groupRef: User/d2b-system
      mode: "0600"
      sensitivity: private
      createPolicy: process-creates
      repairPolicy: none
      cleanupPolicy: vm-stop-with-proof
      adoptionPolicy: quarantine-on-ambiguity
      restartPolicy: clear-on-runner-restart
      leaseClass: process-pidfd
      invariants: [no-symlink, no-magic-link]
    - path: serial.sock
      type: unix-socket
      ownerRef: User/d2b-system
      groupRef: User/d2b-system
      mode: "0600"
      sensitivity: private
      createPolicy: process-creates
      repairPolicy: none
      cleanupPolicy: vm-stop-with-proof
      adoptionPolicy: quarantine-on-ambiguity
      restartPolicy: clear-on-runner-restart
      leaseClass: process-pidfd
      invariants: [no-symlink, no-magic-link]
  views:
    runner:
      path: ""
      rights: [read, write, create, delete, traverse]
    controller-observe:
      path: ""
      rights: [read, traverse]
  attachments: []
  quota:
    maxBytes: 10485760     # 10 MiB; capped by config.runtimeTmpfsQuotaBytes
    maxInodes: 1024
    enforcement: hard
```

The `cleanupPolicy: vm-stop-with-proof` entries require that the owning
runner Process's pidfd has signaled exit before the Volume is torn down. The
controller does not unilaterally delete the Volume; it sets `desiredLifecycle:
stopped` on the runner and waits for the Process to reach `Succeeded` or
`Failed` before clearing the finalizer.

### 6.2 Operator-authored media Volume (boot or removable)

Persistent boot images and physical block devices are operator-authored
Volumes, not controller-created. The operator declares them independently
from the Guest. This decouples media lifecycle from Guest lifecycle.

Example for a raw or qcow2 disk image:

```yaml
apiVersion: resources.d2bus.org/v3
type: Volume
metadata:
  name: corp-iso-boot-media
  zone: corp
  ownerRef: null          # operator-owned; persists beyond Guest deletion
spec:
  providerRef: Provider/volume-local
  source:
    executionRef: Host/host-system
    settings:
      kind: block-image
      sourcePolicyId: corp-iso-boot-media   # opaque configured source policy ID;
                                            # volume-local resolves to the backing
                                            # file or block node; never a public path
  kind: durable
  layout:
    - path: ""
      type: file
      ownerRef: User/d2b-system
      groupRef: User/d2b-system
      mode: "0440"
      sensitivity: restricted
      createPolicy: observe-only    # operator places the file; framework validates
      repairPolicy: none
      cleanupPolicy: never
      adoptionPolicy: adopt-with-live-owner-proof
      restartPolicy: preserve-across-controller-restart
      leaseClass: file-record
      invariants: [no-symlink, no-magic-link]
  views:
    guest-attach:
      path: ""
      rights: [read]      # boot images are read-only by default
  attachments:
    - executionRef: Guest/corp-iso-boot
      transport: virtio-blk
      view: guest-attach
      access: read-only
  quota: null
```

For a writable removable USB image:

```yaml
  views:
    guest-attach:
      path: ""
      rights: [read, write]
  attachments:
    - executionRef: Guest/corp-iso-boot
      transport: virtio-blk
      view: guest-attach
      access: read-write
```

The Volume controller (Provider/volume-local) resolves the opaque
`source.settings.sourcePolicyId` against its operator-configured source
policy registry and opens the backing file or block node through its
broker-mediated path. The resulting fd is included in the runner Process's
LaunchTicket inherited fd table. QEMU receives the fd via a sealed fd slot
in its launch configuration; no file path appears in the runner process spec
or any public surface.

### 6.3 Physical block device Volume (optional)

For raw USB or NVMe media to be passed directly (not via image file), the
operator declares a Volume with a source kind that Provider/volume-local
resolves to a physical block node. The fd delivery mechanism is identical to
the image file path; QEMU receives an owned fd.

### 6.4 ProviderStateSet

`ProviderStateSet(zone, "runtime-qemu-media")` is the optional, query-time set
of the *declared* Volume resources in the Zone whose `metadata.ownerRef` equals
`Provider/runtime-qemu-media`. It is not a ResourceType and is not stored; it is
derived by querying the Volume owner index, and is empty for this Provider.

The controller declares **no** Provider state Volume; its `ProviderStateSet` is
empty. All controller recovery data is derivable from the Zone resource store,
the core Operation ledger, and independent external observation (running QEMU
runner processes re-adopted from declared cgroup leaves and fresh pidfds). Its
bounded non-secret operational state — reconcile stage, per-Guest launch/
adoption observations, bounded counters, and closed-enum error detail — lives in
the owning resource's `status` subresource and the core Operation ledger (D087).
Because that operational state is fully derivable, the controller payload fails
the storage-need test: there is no controller state namespace, no controller
state Volume, no `/state` mount, and no dedicated `User/runtime-qemu-media-system`
state-layout principal. There is no empty identity-only Volume.

Controller-created Guest runtime Volumes carry `ownerRef: Guest/<name>` and
do not appear in the ProviderStateSet. Operator-authored media Volumes carry
`ownerRef: null` and also do not appear. These are genuine media/runtime
payloads owned by their respective resources and are retained unchanged.

---

## 7 Network dependency

The controller declares a dependency alias `network → Provider/network-local`
(bound via `config.networkProviderRef`). For each Guest with a non-empty
`spec.networkAttachments`, the controller watches the referenced opaque
`Network/<name>` resources for `Ready` status. It never requests or receives a
tap fd and never sees a network broker operation.

At Process launch, Core authorizes the Guest-to-Network attachment. The
network-local controller declares the opaque attachment through
`NetworkEffectPort`; its Core-owned adapter maps that semantic effect to the
canonical `CreatePersistentTap`, applies `SetBridgePortFlags`, and gives the
already-authorized connected `OwnedFd` directly to ProviderSupervisor.
ProviderSupervisor places that fd in the QEMU Process LaunchTicket attachment.
The fd never traverses the qemu Provider/controller, ResourceAPI,
ComponentSession, or d2b-bus serialization. QEMU receives the tap through the
inherited fd table; no bridge name, interface name, or host network path crosses
the public surface.

The adapter owns the `OwnedFd` with `FD_CLOEXEC` set until ProviderSupervisor
accepts it. ProviderSupervisor retains a CLOEXEC parent copy, creates only the
declared child fd slot immediately before exec, and closes its copy after
successful spawn. QEMU then owns the child copy until exit. Cancellation,
LaunchTicket rejection, or spawn failure closes every copy first and invokes
the generation-fenced `DeletePersistentTap`; the opaque realization remains
retained until deletion is confirmed. Normal teardown likewise waits for QEMU
fd closure before `DeletePersistentTap`.

If `spec.networkAttachments` is empty, the runner starts with no network
interface (isolated).

---

## 8 Device dependencies

### 8.1 Device/host-kvm

KVM acceleration is an explicit `Device` resource dependency, not an
implicit Host capability. The operator must declare `Device/host-kvm` in the
Zone and the Guest must list `Device/host-kvm` in `spec.deviceAttachments`.

```yaml
apiVersion: resources.d2bus.org/v3
type: Device
metadata:
  name: host-kvm
  zone: corp
spec:
  providerRef: Provider/device-kvm   # built-in device Provider; validates /dev/kvm accessibility
  deviceClass: physical
  arbitration: shared
  maxConcurrentClaims: null          # unlimited; KVM is shared across Guests
  inventory:
    selector:
      busClass: kvm
```

The runner Process declares `deviceUsage` for this device:

```yaml
deviceUsage:
  - deviceRef: Device/host-kvm
    access: shared
    purpose: kvm-acceleration
```

The controller watches `Device/host-kvm.status.phase`. If `Phase = Failed`
or the device is absent, the controller sets `Guest.status.phase = Degraded`
with `condition.DeviceReady = False, reason = kvm-device-unavailable`.

When `Device/host-kvm` is `Ready`, the device Provider delivers a verified
`/dev/kvm` fd to the runner's LaunchTicket. The fd is claimed at runner
start; it is not persisted.

If the operator omits `Device/host-kvm` from `spec.deviceAttachments`, the
runner launches without KVM (TCG emulation). The controller sets
`condition.KvmAcceleration = False, reason = kvm-not-requested` as a
non-fatal informational condition.

---

## 9 WaylandSession dependency

`Provider/runtime-qemu-media` does not own any display proxy Process or
Wayland socket. When `spec.provider.settings.displayWindow = true`, the controller
creates a `display-wayland.d2bus.org.WaylandSession` resource in the same Zone,
using the exact ResourceSpec defined by `Provider/display-wayland`'s dossier
(including its required `guestRef`, `hostRef`, `userRef`, `policy`, `identity`,
and `device` fields as applicable). The `runtime-qemu-media` controller is the
resource owner (`ownerRef: Guest/<name>`) and is responsible for creating,
updating, and deleting it; it does not invent additional spec fields.

```yaml
apiVersion: resources.d2bus.org/v3
type: display-wayland.d2bus.org.WaylandSession
metadata:
  name: <guest-uid-short>-display
  zone: corp
  ownerRef: Guest/corp-iso-boot     # controller sets; no managedBy field
spec:
  # Exact spec as defined by Provider/display-wayland's dossier.
  # Field names (guestRef, hostRef, userRef, policy, identity, device
  # requirements) are owned by that Provider; do not reproduce or extend them here.
  providerRef: Provider/display-wayland
  guestRef: Guest/corp-iso-boot
```

The controller watches `display-wayland.d2bus.org.WaylandSession/<guest-uid-short>-display`
for `phase: Ready`. When `display-wayland` sets the session `Ready`, it
writes a typed endpoint attachment to the session's status. The
`runtime-qemu-media` controller reads this opaque attachment (whose exact
status field names are defined by the `display-wayland` dossier) and
includes the corresponding display fd in the runner LaunchTicket.

The `display-wayland` Provider owns all proxy Process instances internally.
`runtime-qemu-media` only:

1. creates/updates/deletes the `display-wayland.d2bus.org.WaylandSession`
   resource as an owner, and
2. consumes the EndpointRef attachment from that session's `Ready` status.

If `spec.provider.settings.displayWindow = false`, the controller does not create a
`WaylandSession` resource. QEMU runs headless (`-display none`).

If `config.displayProviderRef` is null and `displayWindow = true`, the
controller sets `Guest.status.phase = Failed` with reason
`display-provider-not-configured`.

---

## 10 Process templates

`Provider/runtime-qemu-media` declares exactly **one** worker Process
template: `qemu-media-runner`. There is no host-reconcile EphemeralProcess
and no display-proxy Process. The controller reconciles Host/Device/Network/
Volume dependencies asynchronously through the resource watch mechanism —
no separate preflight worker is required.

### 10.1 Full canonical Process ResourceSpec — qemu-media-runner

The controller creates one instance of this resource per Guest:

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: <guest-uid-short>-qemu-runner    # deterministic; uid-short = first 12 hex chars of Guest UID
  zone: corp
  ownerRef: Guest/corp-iso-boot
  # No controller-set finalizers; Process Provider owns its own finalizer.
spec:
  # --- ExecutionSpec common fields ---
  providerRef: Provider/system-minijail
  executionRef: Host/host-system       # resolved from config.controllerExecutionRef
  domain: system
  userRef: null
  processClass: worker
  template: qemu-media-runner           # plain ID within Provider's signed component descriptor
  configRef: null                       # no per-process config; driven by LaunchTicket
  credentialRefs: []

  # --- Mounts ---
  mounts:
    - volumeRef:  Volume/<guest-uid-short>-runtime   # controller-created tmpfs Volume
      view:       runner
      mountPath:  /run/qemu
      access:     read-write
      required:   true

  # --- Sandbox ---
  sandbox:
    namespaceClasses: [pid, mount]     # isolated PID namespace; private mount NS for /run/qemu
    capabilityClasses: []              # zero host capabilities
    seccompClass: qemu-media-runner    # provider-authored seccomp class; reviewed artifact
    noNewPrivileges: true
    startRoot: false                   # never starts as root; broker ensures principal-correct launch
    readOnlyRoot: true
    environmentClass: minimal          # only QEMU-needed env vars; no HOME, USER, SHELL, etc.
    oomScoreAdj: 200

  # --- Budget ---
  budget:
    cpu:
      request: null                    # no guaranteed slice; operator-tuned
      limit:   null
    memory:
      request: null
      limit:   null                    # enforced by cgroup memory.max from Host budget
    pids:
      limit: 512
    fds:
      limit: 1024

  # --- Network ---
  # Core/ProviderSupervisor resolves the opaque Network ref and inserts the
  # pre-authorized tap OwnedFd directly into the LaunchTicket inherited-fd
  # table. The controller never receives the fd; networkUsage remains null.
  networkUsage: null

  # --- Device ---
  deviceUsage:
    - deviceRef: Device/host-kvm
      access: shared
      purpose: kvm-acceleration

  # Stable QMP/serial endpoints are owned Endpoint resources, not inline Process fields.

  # --- Telemetry ---
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false

  # --- Lifecycle ---
  desiredLifecycle: running

  restartPolicy:
    class: never                       # VMM must not be auto-restarted; Guest lifecycle owns teardown
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"

  readiness:
    initialDelay: "0s"
    timeout: "30s"                     # must match config.qmpReadyTimeoutSeconds
    failureThreshold: 1
    successThreshold: 1
    class: provider-defined            # ready when QMP greeting received via EndpointRef attachment

  healthCheck:
    enabled: true
    interval: "10s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined            # periodic QMP query-status; detects silent VMM hang

  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

**Invariants for this Process:**

- `processClass: worker`. Workers have no ResourceAPI or d2b-bus authority.
  No resource watch, no resource write, and no bus-registered service inside
  the runner process.
- `providerRef: Provider/system-minijail`. The runner is sandboxed via the
  broker-validated minijail plan. `clone3(CLONE_PIDFD)` is used; d2b
  owns wait/reap.
- `executionRef` is resolved at creation time from `config.controllerExecutionRef`.
  It is never derived from Guest labels or from a dynamic runtime query.
- `restartPolicy.class: never`. When QEMU exits for any reason (normal
  shutdown, crash, Guest reboot), the controller observes the exit via
  Process status conditions and drives the Guest lifecycle state machine.
  The controller may create a fresh Process resource to re-launch.
- `readiness.class: provider-defined`. Readiness is declared by the
  controller after the first successful QMP capability negotiation, received
  through the `qmp` endpoint connection attachment returned by the Process
  Provider. The controller does not probe any raw socket path.
- No raw principals, argv strings, host paths, executable paths, endpoint locators, or
  credential bytes appear in this spec.

The runner's stable control surfaces are created as owned Endpoint resources:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: media-vm-qmp
  zone: dev
  ownerRef: Guest/media-vm
spec:
  providerRef: Provider/runtime-qemu-media
  producerRef: Process/media-vm-qemu
  endpointClass: control
  transport: unix
  purpose: qmp-control
  serviceFingerprint: runtime-qemu-media.d2bus.org/qmp/v1
  locality: host-local
  visibility: provider
  attachmentPolicy: launch-ticket-only
  consumerPolicy:
    allowedSubjects: [Provider/runtime-qemu-media]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
---
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: media-vm-serial
  zone: dev
  ownerRef: Guest/media-vm
spec:
  providerRef: Provider/runtime-qemu-media
  producerRef: Process/media-vm-qemu
  endpointClass: data
  transport: unix
  purpose: serial-console
  serviceFingerprint: runtime-qemu-media.d2bus.org/serial/v1
  locality: host-local
  visibility: provider
  attachmentPolicy: launch-ticket-only
  consumerPolicy:
    allowedSubjects: [Provider/runtime-qemu-media]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
```

Consumers use `Endpoint/media-vm-qmp` and `Endpoint/media-vm-serial`; no raw
socket path, fd number, or address appears in spec or status.
`visibility` uses the Endpoint schema's exact closed values
`owner | provider | zone`; these examples use `provider`.
`consumerPolicy` provides the finer restriction to
`Provider/runtime-qemu-media` and does not invent another visibility value.

---

## 11 Controller reconcile and finalize loop

### 11.1 Components

| Component | Type | Process class | Binary | Notes |
| --- | --- | --- | --- | --- |
| `runtime-qemu-media-controller` | controller | controller | `d2b-provider-runtime-qemu-media-controller` | Owns Guest ResourceType; runs on `Host/<controllerExecutionRef>` |
| `qemu-media-runner` | worker | worker | `qemu-system-x86_64` (resolved via artifact catalog) | Sandboxed QEMU VMM; one per Guest; spawned by ProviderSupervisor |

The controller is a `processClass: controller` Process resource created by
core as part of `ProviderDeployment`. It is not created by the controller
itself. Its canonical ResourceSpec is:

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: runtime-qemu-media-controller
  zone: system
  ownerRef: Provider/runtime-qemu-media   # owned by the ProviderDeployment
  # No controller-set finalizers; Process Provider owns its own finalizer.
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system          # resolved from config.controllerExecutionRef
  domain: system
  userRef: null
  processClass: controller
  template: runtime-qemu-media-controller # plain template ID within signed descriptor
  configRef: null
  credentialRefs: []
  mounts: []                              # no Provider state Volume; operational state is in status/core ledger (D087)
  sandbox:
    namespaceClasses: [pid]
    capabilityClasses: []
    seccompClass: runtime-qemu-media-controller
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
    oomScoreAdj: 0
  budget:
    cpu:
      request: null
      limit: null
    memory:
      request: null
      limit: null
    pids:
      limit: 256
    fds:
      limit: 512
  networkUsage: null        # controller uses ComponentSession over d2b-bus; no tap
  deviceUsage: []
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  healthCheck:
    enabled: false
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

`config.controllerExecutionRef` is projected into the controller
component only; worker Processes do not inherit it.

### 11.2 Watched resources

The controller watches:

| ResourceType | Watch scope | Purpose |
| --- | --- | --- |
| `Guest` | Zone-wide; provider filter | Primary reconcile trigger |
| `Volume` | Guest ownerRef | runtime Volume, media Volumes |
| `Network` | Zone-wide | Network readiness for tap delivery |
| `Device` | Zone-wide | Device/host-kvm availability |
| `display-wayland.d2bus.org.WaylandSession` | Guest ownerRef | Display compositor readiness |
| `Process` | Guest ownerRef | Runner process lifecycle |

No asynchronous pre-launch EphemeralProcess probes any of these resources.
All readiness checks are performed by observing resource `status.phase` in
the watch loop.

### 11.3 Reconcile logic

```
OBSERVE: Guest Created/Updated or receives deletion-requested hint
  IF metadata.deletionRequestedAt != null → go to Finalize
  FOR each dependency in [Device/host-kvm, Network/*, Volume/boot-media, Volume/removable*, WaylandSession/*]:
    IF dep.phase != Ready:
      set Guest.conditions[dep type] = False, reason = <dep>-not-ready
      set Guest.status.provider.details.providerPhase = "waiting-dependencies"
      return (requeue on dep watch event)
  ensure runtime tmpfs Volume exists and is Ready
  ensure runner Process spec is current (create or UpdateSpec)
  IF runner Process.phase = Ready:
    set Guest.phase = Ready
    set Guest.status.provider.details.providerPhase = "paused-at-boot" (if pauseAtBoot) or "running"
  ELIF runner Process.phase = Failed:
    set Guest.phase = Failed
    set Guest.status.provider.details.providerPhase = "runner-failed"
  ELIF runner Process.phase = Pending | Degraded:
    set Guest.phase = Degraded
    set Guest.status.provider.details.providerPhase = "runner-starting"
```

The controller does not hold a queue slot during dependency waits; it
returns immediately and is re-triggered when the dependency watch event fires.

### 11.4 Finalize logic

```
OBSERVE: Guest.finalizers contains runtime-qemu-media.d2bus.org/guest-cleanup
  set runner Process.desiredLifecycle = stopped (or delete Process resource)
  wait for Process.phase = Succeeded | Failed (via Process watch event)
  IF displayWindow: delete WaylandSession resource; wait for Deleted
  set runtime Volume.desiredLifecycle = deleted (via Volume finalizer drain)
  wait for Volume.phase = Deleted (via Volume watch event)
  remove runtime-qemu-media.d2bus.org/guest-cleanup from Guest.finalizers
  → core removes Guest row; emits ResourceDeleted audit event
```

The finalizer never forcefully unlinks socket paths or sends signals to
processes outside the provider-owned resource graph. The runner's runtime
Volume finalizer (`runtime-qemu-media.d2bus.org/runtime-volume`) ensures the
tmpfs is unmounted only after the runner Process pidfd signals exit.

---

## 12 Guest boot sequence

1. **Dependencies check (async watch):** Controller observes via watch events
   that `Device/host-kvm.phase = Ready`, all `Volume/<media>.phase = Ready`
   (for each ref in spec.provider.settings), and (if displayWindow)
   `WaylandSession.phase = Ready`. No blocking loop; controller re-queues on
   watch events.

2. **Runtime Volume:** Controller creates `<guest-uid-short>-runtime` Volume
   if absent. Waits for `Volume.phase = Ready`.

3. **Network attachment resolution:** Controller supplies only the opaque
   `Network/<name>` ref. Core authorizes the attachment; the network-local
   `NetworkEffectPort` adapter performs `CreatePersistentTap`, applies
   `SetBridgePortFlags`, and transfers the connected CLOEXEC `OwnedFd` directly
   to ProviderSupervisor for the LaunchTicket. The controller never receives
   the broker operation or fd.

4. **Media fd acquisition:** Volume controller (provider/volume-local) makes
   the boot media fd available via the Volume's virtio-blk attachment. The
   fd is included in the runner LaunchTicket as a sealed slot.

5. **KVM fd acquisition:** `device-kvm` Provider delivers the `/dev/kvm` fd
   via the LaunchTicket device fd table.

6. **Display handle (if displayWindow):** Controller reads the opaque
   endpoint attachment from
   `display-wayland.d2bus.org.WaylandSession/<guest-uid-short>-display`
   status (field names owned by the `display-wayland` dossier). Includes
   the corresponding display fd in the LaunchTicket for QEMU's display
   backend.

7. **Runner Process creation:** Controller creates (or UpdateSpec) the
   `<guest-uid-short>-qemu-runner` Process resource and supplies only opaque
   Network/Endpoint refs to Core's attachment resolver. Core seals the
   LaunchTicket only after every private fd attachment is authorized and
   resolved.

8. **ProviderSupervisor launch:** `system-minijail` ProviderSupervisor
   verifies the LaunchTicket, compiles the minijail sandbox plan, and spawns
   QEMU via `clone3(CLONE_PIDFD)` into the correct cgroup leaf. QEMU receives
   all required fds via its inherited fd table; no paths cross the supervisor
   boundary. Launch rejection or spawn failure closes the tap `OwnedFd` before
   the Network adapter invokes generation-fenced `DeletePersistentTap`.

9. **QMP readiness:** After spawn, the Process Provider monitors the `qmp`
   endpoint. When QEMU writes its initial capabilities JSON to the QMP socket,
   the Process Provider delivers the validated `qmp` endpoint connection
   attachment (an owned fd; not a raw socket path) to the controller via the
   ProviderSupervisor ComponentSession channel.

10. **Pause-at-boot (if pauseAtBoot):** Controller receives the QMP
    attachment. QEMU starts in `-S` (paused) state. Controller records
    `providerPhase = "paused-at-boot"`. Operator issues `d2b vm resume` or
    equivalent to issue QMP `cont` through the controller.

11. **Running:** Controller issues QMP `cont` (if not pauseAtBoot or after
    operator resume). Sets `Guest.status.phase = Ready`,
    `providerPhase = "running"`.

---

## 13 QMP protocol via Endpoint resource

The QMP socket is represented by the owned `Endpoint/<guest>-qmp` resource with
`producerRef: Process/<guest>-qemu`, `endpointClass: control`, and
`transport: unix`. It is not a broker operation target and no public path is
exposed.

### 13.1 Connection attachment delivery

When the runner Process reaches `Ready`, the `system-minijail` Process
Provider delivers a validated local connection attachment for each authorized EndpointRef to the controller via the ProviderSupervisor
ComponentSession channel. The attachment for `Endpoint/<guest>-qmp` is a sealed
connection handle (an owned fd to the QMP socket). The attachment for
`Endpoint/<guest>-serial` is delivered similarly.

The controller uses the `qmp` attachment fd to negotiate the QMP capability
exchange and then issue commands. Neither the fd number nor any socket path
is written to the resource store, status fields, audit events, or OTEL spans.

### Endpoint resources (D092)

`Provider/runtime-qemu-media` declares conformance to the standard `Endpoint`
base schema. Stable QMP and serial-control identities are owned `Endpoint`
resources with `producerRef: Process/<guest>-qemu`; future stable vhost-user
sound/video data surfaces follow the same pattern with `endpointClass: data`.
Consumers use `Endpoint/<name>` ResourceRefs; raw socket paths, fd numbers, CIDs,
ports, and credentials never appear in resource spec/status or CLI output.
Core/ProviderSupervisor resolves private transports only through authorized
EffectPort/LaunchTicket flows; unauthorized resolution fails
`endpoint-resolve-denied`. A QEMU runner restart bumps `endpointGeneration` and
triggers dependent consumers through `dependency-changed`.

### Retained opaque handles

The retained opaque values are the per-session QMP connection handle, serial
connection handle, LaunchTicket fd indexes, pidfd/process observations,
`OwnedTransport`, and operation IDs. They are controller-internal, high-churn,
or lack independent lifecycle, so they are not promoted to resources by the
D092 promotion test.

### 13.2 QMP command dispatch

| Operation | QMP command | Trigger |
| --- | --- | --- |
| Query initial capabilities | `qmp_capabilities` | On QMP attachment received |
| Release from pause | `cont` | On operator resume request or if `pauseAtBoot = false` |
| Graceful shutdown | `system_powerdown` | On Guest desiredPhase = stopped |
| Query VM status | `query-status` | Health check; `system-minijail` health check class |
| Media hotplug attach | `blockdev-add` + `device_add` | On removable Volume attachment added |
| Media hotplug detach | `device_del` + `blockdev-del` | On removable Volume attachment removed |
| Query block devices | `query-block` | Volume attachment status reporting |

### 13.3 Hotplug protocol

Removable media is attached and detached at runtime by updating the Guest
`spec.provider.settings.removableVolumeRefs` list. The controller observes
the change, requests the Volume fd from `volume-local` ComponentSession, and
issues `blockdev-add` + `device_add` over the QMP attachment. Detach is the
reverse: `device_del` + `blockdev-del` after quiescing.

The controller records the hotplug result in the Volume's attachment status.
On a failed hotplug, the Volume attachment reverts to `Pending` and the
controller retries with backoff.

---

## 14 Broker operations

With the v3 model, the majority of the current `QemuMedia*` broker ops are
replaced by resource-mediated fd delivery. The following table shows the
disposition of every current baseline broker op:

| Baseline broker op | v3 disposition |
| --- | --- |
| `QemuMediaBoot` (opens image fd + starts QEMU) | Replaced by Volume virtio-blk attachment fd in LaunchTicket + Process resource creation |
| `QemuMediaAttach` (opens image/usb fd, QMP blockdev-add) | Replaced by Volume attachment update + QMP command via EndpointRef connection |
| `QemuMediaDetach` (QMP device_del, closes fd) | Replaced by Volume attachment delete + QMP command via EndpointRef connection |
| `QemuMediaStop` (SIGTERM → SIGKILL) | Replaced by Process `desiredLifecycle: stopped`; `system-minijail` issues SIGTERM via pidfd |
| `QemuMediaStatus` (parse /proc/pid/status) | Replaced by `Process.status` conditions; no /proc path access by Provider |
| `QemuMediaQueryBlock` (QMP query-block) | Controller-internal; result stored in Volume attachment status |
| `QemuMediaResume` (QMP cont) | Controller method call over QMP attachment; triggered by operator resource verb |
| `QemuMediaOpenDev` (opens /dev/kvm fd) | Replaced by `Device/host-kvm` device fd in LaunchTicket via device-kvm Provider |

**Broker operations that remain** (net-new, requiring privileged host effect):

| Op | Purpose | Condition |
| --- | --- | --- |
| `OpenDevice(kvm)` | Deliver `/dev/kvm` fd to `device-kvm` Provider for LaunchTicket inclusion | Issued by `device-kvm` Provider controller; never by `runtime-qemu-media` directly |
| `SpawnRunner` (qemu-media-runner role) | Launch QEMU binary in cgroup leaf via `clone3(CLONE_PIDFD)` | Issued by `system-minijail` ProviderSupervisor on behalf of Process controller |
| `CreatePersistentTap` | Create/adopt the opaque Guest Network attachment realization | Issued only by the Core-owned NetworkEffectPort adapter after network-local declares the semantic effect |
| `SetBridgePortFlags` | Apply isolation and neighbor-suppression policy before launch | Issued by the same NetworkEffectPort adapter before fd handoff |
| `DeletePersistentTap` | Remove the generation-fenced realization after fd closure | Issued by the NetworkEffectPort adapter on failed launch and normal teardown |

`runtime-qemu-media` does not issue or receive any broker operation directly.
Its controller supplies only opaque Network and Endpoint refs. The connected
tap `OwnedFd` moves directly from the Core-owned NetworkEffectPort adapter to
ProviderSupervisor and then the child fd table; it is never serialized on
d2b-bus or delivered through the controller's ComponentSession.

---

## 15 RBAC and permission claims

### 15.1 Declared permission claims (in Provider manifest)

| Claim | Target type | Verbs | Purpose |
| --- | --- | --- | --- |
| `guest-reconcile` | Guest | get, list, watch, create, update-spec, delete | Own ResourceType |
| `process-manage` | Process | get, list, watch, create, update-spec, delete | Runner process lifecycle |
| `volume-watch-media` | Volume | get, list, watch | Watch media Volume status |
| `volume-create-runtime` | Volume | get, list, watch, create, update-spec, delete | Create/delete runtime tmpfs Volume |
| `network-watch` | Network | get, list, watch | Watch Network readiness |
| `device-kvm-watch` | Device | get, list, watch | Watch Device/host-kvm status |
| `waylandsession-manage` | display-wayland.d2bus.org.WaylandSession | get, list, watch, create, update-spec, delete | Create/delete WaylandSession for display |
| `user-watch` | User | get, list, watch | Resolve Guest userRef |

### 15.2 Operator Role and RoleBinding required

The operator must grant the controller's identity (auto-created Service
Account for `Provider/runtime-qemu-media`) at least the following:

```yaml
# Minimum Role rules for the controller
rules:
  - resourceTypes: [Guest]
    verbs: [get, list, watch, create, update-spec, delete]
  - resourceTypes: [Process, EphemeralProcess]
    verbs: [get, list, watch, create, update-spec, delete]
  - resourceTypes: [Volume]
    verbs: [get, list, watch, create, update-spec, delete]
  - resourceTypes: [Network]
    verbs: [get, list, watch]
  - resourceTypes: [Device]
    verbs: [get, list, watch]
  - resourceTypes: [display-wayland.d2bus.org.WaylandSession]
    verbs: [get, list, watch, create, update-spec, delete]
```

The matching RoleBinding binds that Role to the Provider identity without
adding expiry or authority:

```yaml
roleRef: Role/runtime-qemu-media-controller
subjects: [Provider/runtime-qemu-media]
externalPrincipalSelector: null
scopeNarrowing: null
```

### 15.3 Worker process authority

The `qemu-media-runner` worker has **no** ResourceAPI authority, **no**
d2b-bus authority, and **no** ComponentSession authority. It receives:

- inherited fds from the LaunchTicket (kvm fd, tap fd, media fds, display fd);
- its mounted runtime Volume view (`runner`);
- the compiled sandbox.

It cannot register services, issue resource updates, or communicate with any
d2b component except through the standard fd-inherited interfaces.

---

## 16 Status, phases, and conditions

### 16.1 Common phase

`Guest.status.phase` uses only the common closed set:

| Phase | Meaning |
| --- | --- |
| `Pending` | Controller created resource; dependencies not yet ready |
| `Ready` | QEMU runner is running and QMP reports VM state |
| `Degraded` | Runner is available but one or more non-critical conditions are degraded |
| `Failed` | Runner exited non-zero, QMP timeout, or a required dependency failed; Guest requires operator action |
| `Deleted` | Terminal phase; row removed after `ResourceDeleted` audit event emitted |
| `Unknown` | Controller has lost contact with runner Process; status not current |

### 16.2 providerPhase

D088 status layering is normative: the controller populates the Guest
ResourceType-common `status.resource` with runtime readiness, capabilities,
observed lifecycle phase, bootstrap readiness, and active process count in the
same shape as sibling Guest runtime providers. QEMU media-specific QMP/runner
lifecycle detail, including `providerPhase`, lives only in
`status.provider.details` with `providerRef: Provider/runtime-qemu-media`,
qualified `schemaId` (`runtime-qemu-media.d2bus.org/Guest/status`), `schemaVersion`,
and `observedProviderGeneration`. Controller status writes include all present
layers atomically in one status mutation; shared fields are never duplicated
into `status.provider`, and the strict, ≤32 KiB, redacted extension schema is
registered and signed in the Provider manifest.

#### Currency and expedited reconcile (D091/D090)

D091 currency is universal status, not QEMU media provider detail. The
controller implements `assess_update`, `plan_upgrade`, and `execute_upgrade`,
populates universal `status.update`, and keeps shared currency fields out of
`status.provider`; backend-specific observations may appear only under
`status.provider.details`. A new NixOS system/image generation, provider
package generation, or disruptive runtime spec change MUST set
`status.update.state = UpgradeRequired`, with `reasons =
[ImageOrSystemGenerationChanged]`, `[ProviderGenerationChanged]`, or
`[SpecChanged]`, `disruption = Recycle|Restart`, and `preserveState = true`
rather than applying in place. Non-disruptive spec changes reconcile normally.
`execute_upgrade` recycles the QEMU runner `Process` and endpoints while
preserving the Guest UID/spec identity, durable/data Volumes, and TPM identity
supplied by `Provider/device-tpm`; the dependency-aware planner restarts the
Guest after the recycle.

D090 expedited `waitForReconcile` on `Create`/`UpdateSpec`/`Delete` performs no
external effect, finalizer change, or status mutation until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. The
one-pass response returns the committed object, projected layered status,
disposition `Converged|Progressing|Blocked|UpgradeRequired|Failed`, and
`statusPersistence = pending|committed`; the durable commit is never rolled back
after a reconcile timeout. Effect idempotency keys derive from
`(UID,generation,revision,operationId)`, and the expedited pass uses the bounded
priority lane inside the same per-resource single-flight.

`status.provider.details.providerPhase` is a bounded string carrying backend
lifecycle detail. Transitions are controlled by the controller only:

| providerPhase | Meaning |
| --- | --- |
| `""` (empty) | Resource just created; no action yet |
| `"waiting-dependencies"` | One or more of: Device/host-kvm, Volume, Network, WaylandSession not Ready |
| `"creating-runtime-volume"` | Controller creating runtime tmpfs Volume |
| `"launching-runner"` | LaunchTicket sealed; ProviderSupervisor has not yet confirmed spawn |
| `"waiting-qmp"` | Runner spawned; waiting for QMP capabilities greeting |
| `"paused-at-boot"` | QMP ready; VM paused (pauseAtBoot = true); awaiting operator `cont` |
| `"running"` | VM running; QMP status reports `running` |
| `"stopping"` | `system_powerdown` sent; waiting for VM exit |
| `"runner-failed"` | Runner Process in Failed phase; Guest requires operator action |
| `"finalize-pending"` | Deletion in progress; finalizer drain under way |

`providerPhase` length is bounded to 64 characters. The string set is
closed; the controller never writes a value outside this table.

### 16.3 Conditions

| Condition | Ready=True | Reason codes (False) |
| --- | --- | --- |
| `Scheduled` | Controller assigned | `controller-unavailable` |
| `ProviderReady` | Provider/runtime-qemu-media Ready | `provider-generation-mismatch` |
| `DeviceReady` | Device/host-kvm Ready (or not requested) | `kvm-device-unavailable`, `kvm-device-failed` |
| `NetworkReady` | All spec.networkAttachments Networks Ready | `network-not-ready`, `network-failed` |
| `VolumeReady` | Boot and all removable Volumes Ready | `volume-not-ready`, `volume-failed`, `volume-access-denied` |
| `DisplayReady` | WaylandSession Ready (or not requested) | `display-provider-not-configured`, `wayland-session-failed` |
| `RunnerReady` | Runner Process in Ready phase | `runner-launching`, `runner-qmp-timeout`, `runner-exited`, `runner-failed` |
| `KvmAcceleration` | KVM acceleration active (informational) | `kvm-not-requested`, `kvm-tcg-fallback` |

### 16.4 Error inventory

| Error code | Phase | Description |
| --- | --- | --- |
| `kvm-device-unavailable` | Degraded | Device/host-kvm not Ready; KVM inaccessible |
| `media-volume-not-ready` | Pending | Boot media Volume not Ready |
| `runtime-volume-create-failed` | Failed | Cannot create runtime tmpfs Volume |
| `runner-launch-timeout` | Failed | ProviderSupervisor did not confirm spawn within deadline |
| `qmp-greeting-timeout` | Failed | QMP greeting not received within `qmpReadyTimeoutSeconds` |
| `qmp-command-failed` | Degraded | A QMP command returned an error; see condition detail |
| `hotplug-media-failed` | Degraded | blockdev-add/device_add QMP command failed |
| `runner-exited-unexpectedly` | Failed | Runner Process exited while Guest desiredPhase was running |
| `display-provider-not-configured` | Failed | displayWindow=true but config.displayProviderRef=null |
| `wayland-session-failed` | Failed | WaylandSession owned by this Guest is in Failed phase |
| `network-tap-unavailable` | Degraded | Core could not authorize or resolve the Network attachment for launch |

---

## 17 Audit events

All audit events follow the ADR 0046 audit format. Sensitive fields
(executable path, argv, fds, socket paths, raw QEMU output, VM memory
contents, host paths) are never included in any audit payload.

| Event kind | Phase / trigger | Payload fields (bounded) |
| --- | --- | --- |
| `GuestCreated` | Guest.phase: Pending initial | zone, guestRef, providerRef, vcpu, memMib |
| `RunnerLaunching` | providerPhase: launching-runner | zone, guestRef, processRef |
| `QmpReady` | providerPhase: waiting-qmp → paused-at-boot/running | zone, guestRef, processRef, qmpVersion |
| `GuestRunning` | providerPhase: running | zone, guestRef, processRef |
| `GuestPausedAtBoot` | providerPhase: paused-at-boot | zone, guestRef |
| `GuestResumed` | operator issued cont | zone, guestRef, actorRef |
| `MediaHotplugAttached` | Volume hotplug success | zone, guestRef, volumeRef, deviceIndex |
| `MediaHotplugDetached` | Volume hotplug detach | zone, guestRef, volumeRef |
| `GuestStopping` | desiredPhase → stopped | zone, guestRef, actorRef, stopReason |
| `RunnerExited` | runner Process Succeeded/Failed | zone, guestRef, processRef, exitClass |
| `GuestDeleted` | finalizer cleared; post-commit | zone, guestRef |
| `DependencyDegraded` | a dependency watch event degrades | zone, guestRef, depType, depRef, reason |
| `ProviderPhaseFailed` | phase → Failed | zone, guestRef, errorCode, detail |
| `WaylandSessionCreated` | WaylandSession resource created | zone, guestRef, sessionRef |
| `WaylandSessionDeleted` | WaylandSession deleted during finalize | zone, guestRef, sessionRef |

`detail` is a bounded string (max 256 chars) containing the human-readable
error summary. It never contains raw process output, host paths, socket
addresses, or argv.

---

## 18 Telemetry and metrics

All metric labels use closed semantic sets. Cardinality is bounded; no label
key or value may contain Zone/VM/resource identity, user identity, executable
path, or VM memory content. Zone identity remains in the bounded `d2b.zone`
OTEL resource attribute.

| Metric | Type | Labels | Notes |
| --- | --- | --- | --- |
| `d2b_guest_reconcile_total` | counter | provider, outcome (success/failure) | Reconcile loop outcomes |
| `d2b_guest_reconcile_duration_seconds` | histogram | provider | Reconcile latency |
| `d2b_guest_phase_transitions_total` | counter | provider, from_phase, to_phase | Phase machine |
| `d2b_guest_runner_launches_total` | counter | provider, outcome | Launch attempts |
| `d2b_guest_qmp_ready_seconds` | histogram | provider | Time from runner spawn to QMP greeting |
| `d2b_guest_qmp_operations_total` | counter | provider, operation, outcome | Per-operation QMP results |
| `d2b_guest_media_hotplug_total` | counter | provider, operation (attach/detach), outcome | |
| `d2b_guest_dependency_wait_seconds` | histogram | provider, dep_type | Time waiting for dependency |
| `d2b_guest_active` | gauge | provider, phase | Active Guest count per phase |
| `d2b_guest_runner_restart_total` | counter | provider | Runner exits (controller re-creates) |

OTEL trace spans:

| Span | Parent | Attributes |
| --- | --- | --- |
| `guest.reconcile` | — | phase, outcome |
| `guest.runner.launch` | `guest.reconcile` | outcome |
| `guest.qmp.connect` | `guest.runner.launch` | outcome |
| `guest.qmp.command` | `guest.reconcile` | command (closed set), outcome |
| `guest.media.hotplug` | `guest.reconcile` | operation (attach/detach), outcome |
| `guest.finalize` | `guest.reconcile` | outcome |

Span attributes are fixed semantic classifiers and outcomes only. No span
attribute may carry Zone, Guest, Process, Provider-resource, or other resource
identity in any form, including a name, UID, short UID, digest, ResourceRef, or
derived token. Identity belongs only in allow-listed OTEL Resource attributes
such as `d2b.zone` and `d2b.provider`, or in an authorized bounded audit record.
Spans also exclude argv, executable paths, VM memory, host filesystem paths,
fds, socket paths, raw process output, and user-supplied opaque data.

---

## 19 Nix configuration

### 19.1 Artifact declarations

```nix
d2b.artifacts = {
  runtime-qemu-media = {
    package = pkgs.d2b-provider-runtime-qemu-media;
    type    = "provider";
  };
  qemu-system-x86_64 = {
    package = pkgs.qemu-kvm;   # or pkgs.qemu_full, depending on site policy
    type    = "config-bundle"; # sealed QEMU binary closure; not a provider
  };
};
```

### 19.2 Provider catalog entry

```nix
d2b.providerCatalog.runtime-qemu-media = {
  artifactId = "runtime-qemu-media";
  trust      = { publisherRef = "d2b-official"; };
};
```

### 19.3 Provider resource install

```nix
d2b.zones.corp.resources.runtime-qemu-media = {
  type = "Provider";
  spec = {
    artifactId = "runtime-qemu-media";
    config = {
      controllerExecutionRef  = "Host/host-system";  # required
      qemuBinaryArtifactId    = "qemu-system-x86_64";
      networkProviderRef      = "Provider/network-local";
      volumeProviderRef       = "Provider/volume-local";
      displayProviderRef      = "Provider/display-wayland";  # omit if no display Guests
      pausedAtBootDefault     = true;
      qmpReadyTimeoutSeconds  = 30;
    };
  };
};
```

### 19.3B Controller operational state (status-first; no state Volume)

The controller declares **no** Provider state Volume; there is no
ProviderDeployment-created controller state Volume and no
`User/runtime-qemu-media-system` state-layout principal to provision. The
controller's bounded non-secret operational state lives in the owning
resource's `status` subresource and the core Operation ledger (D087), and all
recovery data is re-derived on restart from the Zone resource store, the core
Operation ledger, and independent external observation (running QEMU runners
re-adopted from declared cgroup leaves and fresh pidfds). See §6.4 and §20.

### 19.4 KVM Device resource

The operator must declare `Device/host-kvm` in the Zone:

```nix
d2b.zones.corp.resources.host-kvm = {
  type = "Device";
  spec = {
    providerRef = "Provider/device-kvm";  # built-in; validates /dev/kvm access
    deviceClass = "physical";
    arbitration = "shared";
    inventory.selector.busClass = "kvm";
  };
};
```

### 19.5 Operator-authored media Volume

```nix
d2b.zones.corp.resources.corp-iso-boot-media = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings = {
        kind = "block-image";
        sourcePolicyId = "corp-iso-boot-media";  # opaque configured source policy ID
      };
    };
    kind = "durable";
    views.guest-attach = {
      path   = "";
      rights = ["read"];
    };
    attachments = [{
      executionRef = "Guest/corp-iso-boot";
      transport    = "virtio-blk";
      view         = "guest-attach";
      access       = "read-only";
    }];
  };
};
```

### 19.6 Guest resource with media and KVM

```nix
d2b.zones.corp.resources.corp-iso-boot = {
  type = "Guest";
  spec = {
    providerRef = "Provider/runtime-qemu-media";
    vcpu        = 4;
    memoryMib   = 8192;
    networkAttachments = [{
      networkRef = "Network/corp-net";
    }];
    deviceAttachments = [{
      deviceRef = "Device/host-kvm";
      exclusive = false;
    }];
    provider = {
      schemaId = "runtime-qemu-media.d2bus.org/Guest/spec";
      schemaVersion = "1.0.0";
      settings = {
        bootMediaRef  = "Volume/corp-iso-boot-media";
        bootMediaView = "guest-attach";
        cpuModel      = "host";
        machineType   = "q35";
        bios          = "ovmf";
        pauseAtBoot   = true;
        displayWindow = false;
        serialConsole = true;
        tablet        = true;
        rtcBase       = "utc";
      };
    };
  };
};
```

### 19.7 Guest with display window and removable media

```nix
d2b.zones.corp.resources.corp-media-station = {
  type = "Guest";
  spec = {
    providerRef = "Provider/runtime-qemu-media";
    vcpu        = 2;
    memoryMib   = 4096;
    networkAttachments = [];
    deviceAttachments = [{
      deviceRef = "Device/host-kvm";
      exclusive = false;
    }];
    provider = {
      schemaId = "runtime-qemu-media.d2bus.org/Guest/spec";
      schemaVersion = "1.0.0";
      settings = {
        bootMediaRef  = "Volume/corp-win-installer";
        bootMediaView = "guest-attach";
        removableVolumeRefs = [{
          volumeRef = "Volume/corp-drivers-usb";
          view      = "guest-attach";
        }];
        pauseAtBoot   = false;
        displayWindow = true;     # controller will create WaylandSession
        serialConsole = false;
        tablet        = true;
      };
    };
  };
};
```

### 19.8 Evaluation assertions

The following eval-time assertions are added in
`nixos-modules/assertions.nix`:

| Assertion | Error message |
| --- | --- |
| Every `Guest` with `providerRef = runtime-qemu-media` must have `deviceAttachments` containing `Device/host-kvm` when `kvmRequired = true` in site config | `Guest <n>: runtime-qemu-media Guest must declare Device/host-kvm in deviceAttachments` |
| Every `bootMediaRef` must name a Volume declared in the same Zone | `Guest <n>: bootMediaRef Volume/<v> not declared in zone <z>` |
| Every `removableVolumeRefs[].volumeRef` must name a Volume in the same Zone | `Guest <n>: removableVolumeRef Volume/<v> not declared in zone <z>` |
| If `displayWindow = true`, `config.displayProviderRef` must be non-null | `Provider/runtime-qemu-media: displayProviderRef required when any Guest sets displayWindow=true` |
| `config.controllerExecutionRef` must name a Host declared in the same Zone | `Provider/runtime-qemu-media: controllerExecutionRef Host/<h> not declared in zone <z>` |

---

## 20 ProviderStateSet

`ProviderStateSet(zone, "runtime-qemu-media")` is the optional, query-time
grouping of the *declared* Volume resources in the Zone whose
`metadata.ownerRef` resolves to `Provider/runtime-qemu-media`. It is not a
ResourceType, not a stored artifact, and has no "compartments". The set is
derived by querying the Zone resource store's owner index, and is empty for this
Provider.

### 20.1 No controller state Volume

The controller declares **no** Provider state Volume. Its bounded non-secret
operational state — reconcile stage, per-Guest launch/adoption observations,
bounded counters, and closed-enum error detail — lives in the owning resource's
`status` subresource and the core Operation ledger (D087). All recovery data is
re-derived on restart from the Zone resource store, the core Operation ledger,
and independent external observation (running QEMU runners re-adopted from
declared cgroup leaves and fresh pidfds). Because that state is fully derivable,
the controller payload fails the storage-need test: there is no controller
state namespace, no controller state Volume, no `/state` mount, and no dedicated
`User/runtime-qemu-media-system` state-layout principal. There is no empty
identity-only Volume.

A future revision that introduces a durable per-provider payload passing the
storage-need test (for example, a large or secret source-policy cache that
cannot live in `status`) would add a `stateNamespace` to the component
descriptor and create an additional Volume with
`ownerRef: Provider/runtime-qemu-media`.

### 20.2 What is not in the ProviderStateSet

- Runtime tmpfs Volumes for Guest runners carry `ownerRef: Guest/<name>` —
  they are Guest resources, not Provider state, and are retained unchanged.
- Operator-authored boot/removable media Volumes carry `ownerRef: null` —
  they are not Provider state.

### 20.3 Destruction

When `Provider/runtime-qemu-media` receives `metadata.deletionRequestedAt`,
`ProviderDeployment` (core):

1. Signals the controller Process to drain (`desiredLifecycle: stopped`);
   waits for Process finalizer to complete.
2. Removes the Provider row and emits `ResourceDeleted`. The controller's
   `status` disappears with the resource row and its revision; there is no
   separate state-Volume disposition because the Provider declares none.

---

## 21 Implementation work items

Each work item includes the source it adapts (baseline `b5ddbed6`) and the
destination in `packages/d2b-provider-runtime-qemu-media/`.

---

### ADR046-qemu-media-001 Crate scaffold and layout gate

| Field | Value |
| --- | --- |
| Dependency/owner | P0; blocks all other runtime-qemu-media work items; owner: `runtime-qemu-media` Provider crate |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/{src/lib.rs,tests/provider_layout.rs,integration/mod.rs,README.md} |
| Detailed design | Crate scaffold and layout gate: create the crate with the four required paths, commit a README.md stub meeting §1 requirements, and wire the workspace policy gate so the crate cannot land without `src/`, `tests/`, `integration/`, and `README.md`. |
| Integration | Workspace/Cargo policy consumes the new crate layout; later Guest schema, controller, QMP, Nix, and integration work items build inside this crate. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `make test-policy` (workspace policy gate) |
| Removal proof | None — net-new; no prior owner to remove |

---

### ADR046-qemu-media-002 Guest ResourceType schema and serde

| Field | Value |
| --- | --- |
| Dependency/owner | P0; depends on ADR046-qemu-media-001; owner: runtime-qemu-media type/schema implementation |
| Current source | `packages/d2b-core/src/host.rs` — `HostQemuMedia`, `QemuMediaSourceIntent` field names/types only; raw path/credential fields are discarded |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/src/types/guest.rs |
| Detailed design | Guest ResourceType schema and serde: define `GuestSpec`, `GuestStatus`, and `GuestProviderSpecSettings` with serde and `schemars` JSON Schema. Fields must match §4, §5, and §16 exactly. Enforce `bootMediaRef` as a `Volume/<n>` ResourceRef, `removableVolumeRefs` max 4 entries, `providerPhase` max 64 chars with the closed value set, and no argv/path/credential bytes in any serialized type. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt selected baseline field concepts; discard raw paths, argv, and credential-carrying fields. |
| Integration | Nix-rendered Guest resources and ResourceAPI admission use these types; the controller consumes the validated spec and writes matching status; conformance and schema tests consume the generated schema. |
| Data migration | Full d2b 3.0 reset; media guests are reauthored as `Guest`/`Volume`/`Device` resources rather than importing v2 host media config |
| Validation | `tests/guest_schema_roundtrip.rs`; `tests/guest_provider_settings_bounds.rs` |
| Removal proof | `HostQemuMedia`/`QemuMediaSourceIntent` raw path surfaces are superseded once all media Guest specs use ResourceRefs and schema tests prove no path/argv/credential fields remain |

---

### ADR046-qemu-media-003 Provider config schema and projection

| Field | Value |
| --- | --- |
| Dependency/owner | P0; depends on ADR046-qemu-media-001; owner: runtime-qemu-media Provider config/schema implementation |
| Current source | `packages/d2b-core/src/runtime.rs` — timeout/quota concepts only |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/src/config.rs |
| Detailed design | Provider config schema and projection: define `ProviderConfig`, derive JSON Schema, require `controllerExecutionRef`, validate bounds, and project config only to the controller component. Worker processes receive no root config, no ResourceAPI authority, and no d2b-bus authority. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt bounded timeout/quota concepts into v3 Provider config; project only to the controller component. |
| Integration | Provider ResourceSpec admission validates this schema; ProviderDeployment injects the projected config into the controller; controller uses the provider refs and quotas when reconciling Guest, Volume, Network, Device, Endpoint, and Process resources. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `tests/config_schema_projection.rs` |
| Removal proof | None — config projection is a new Provider resource surface; no prior owner is removed by this item |

---

### ADR046-qemu-media-004 Controller status-first operational state (no state Volume)

| Field | Value |
| --- | --- |
| Dependency/owner | P0; depends on ADR046-qemu-media-001 and ADR046-qemu-media-003; owner: runtime-qemu-media controller descriptor/state implementation |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/{descriptor.rs,state.rs}; no Volume management code for Provider state |
| Detailed design | Controller status-first operational state (no state Volume): controller component descriptor declares an empty `stateNamespaces` list; ProviderDeployment creates no controller state Volume; controller writes reconcile stage, per-Guest launch/adoption observations, bounded counters, and closed-enum error detail to `status` on material change without secrets, paths, argv, PIDs, or unit names; restart re-derives observed state from the Zone resource store, core Operation ledger, and independent external observation with fresh pidfds. Worker Processes and the controller receive no state-Volume mount. |
| Integration | ProviderDeployment reads the descriptor; the controller projects bounded observations to Guest status and the Operation ledger; restart/adoption logic consumes resource-store, ledger, and external runner observations rather than private state storage. |
| Data migration | None — status-first controller state only; no runtime state is migrated into a Provider state Volume |
| Validation | `tests/state_status_spec.rs`; `tests/state_status_restart.rs`; `tests/state_mount_exclusivity.rs` |
| Removal proof | None — this item prevents creation of a new Provider state Volume and has no prior state owner to remove |

---

### ADR046-qemu-media-005 Runtime tmpfs Volume resource

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-001 and ADR046-qemu-media-003; owner: runtime-qemu-media controller Volume reconciliation |
| Current source | `packages/d2b-host/src/qemu_media_argv.rs` — `run_dir` and socket naming pattern only; raw path construction is discarded |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/volume.rs |
| Detailed design | Runtime tmpfs Volume resource: controller creates the per-Guest runtime tmpfs Volume specified in §6.1. The emitted spec must exactly match the canonical YAML, including all layout entries, views, quota, and `cleanupPolicy: vm-stop-with-proof`. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt naming intent into controller-created Volume resources; replace raw runtime directory paths with `Volume` specs. |
| Integration | Guest reconcile creates/updates this Volume through the ResourceAPI; `volume-local` materializes the tmpfs and returns attachments to the Process launch flow; finalize proves cleanup before Guest finalization. |
| Data migration | Full d2b 3.0 reset; runtime tmpfs state is ephemeral and not imported from v2 run directories |
| Validation | `tests/runtime_volume_spec.rs`; `tests/volume_cleanup_policy.rs` |
| Removal proof | Legacy raw run-directory handling from `qemu_media_argv.rs` is superseded once runtime storage is represented only by controller-created Volume resources |

---

### ADR046-qemu-media-006 Media Volume watch and virtio-blk attachment validation

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-002 and ADR046-qemu-media-005; owner: runtime-qemu-media media dependency controller |
| Current source | `packages/d2b-core/src/host.rs` `QemuMediaSourceKind` — media kind enumeration only |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/media_watch.rs |
| Detailed design | Media Volume watch and virtio-blk attachment validation: controller watches `bootMediaRef` and `removableVolumeRefs` Volumes for `Ready` status and validates that each has a `virtio-blk` attachment for the owning Guest. It performs no path inspection. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt media kind concepts to Volume source-kind assertions and ResourceRef watches. |
| Integration | Guest reconcile gates Process launch on watched Volume readiness; Volume attachment status feeds LaunchTicket media fd assembly and Guest conditions. |
| Data migration | Full d2b 3.0 reset; operator-authored media is declared as Volume resources rather than imported from raw qemu-media source paths |
| Validation | `tests/media_volume_watch.rs`; `tests/media_attachment_validation.rs` |
| Removal proof | Legacy media source path handling is superseded once media is delivered only through Volume ResourceRefs and virtio-blk attachments |

---

### ADR046-qemu-media-007 KVM Device watch

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-002; owner: runtime-qemu-media Device dependency controller |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/device_watch.rs |
| Detailed design | KVM Device watch: controller watches `Device/host-kvm` from `spec.deviceAttachments` for `Ready` status and gates runner launch on it, propagating Pending/Ready/Failed transitions to Guest conditions. |
| Integration | Device resource status drives Guest reconcile dependency gating; a Ready KVM Device contributes the sealed kvm fd slot to the LaunchTicket through the Process provider chain. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `tests/kvm_device_watch.rs` |
| Removal proof | None — Device-gated KVM readiness is a new v3 Resource dependency, not a removal item |

---

### ADR046-qemu-media-008 WaylandSession resource management

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-002 and the `display-wayland` Provider dossier; owner: runtime-qemu-media display integration |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/display.rs |
| Detailed design | WaylandSession resource management: when `spec.provider.settings.displayWindow = true`, controller creates, updates, deletes, and watches a `display-wayland.d2bus.org.WaylandSession` resource using the exact ResourceSpec from the display-wayland dossier. It reads the EndpointRef attachment from status using only display-wayland-defined field names. Primary reuse disposition: `create`. Preserved source-plan detail: net-new against the display-wayland Resource contract. |
| Integration | Guest reconcile produces WaylandSession resources; display-wayland publishes Endpoint attachments; LaunchTicket assembly consumes the display fd only when the session is Ready; finalize deletes the session. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `tests/wayland_session_create.rs`; `tests/wayland_session_attachment_read.rs`; `tests/wayland_session_missing_provider.rs` |
| Removal proof | None — display proxy work is delegated to a new WaylandSession Resource dependency rather than removing a baseline owner in this item |

---

### ADR046-qemu-media-009 Process spec builder and LaunchTicket assembly

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-002, ADR046-qemu-media-005, ADR046-qemu-media-006, ADR046-qemu-media-007, ADR046-qemu-media-008, and ADR046-qemu-media-012; owner: runtime-qemu-media Process launch builder |
| Current source | `packages/d2b-host/src/qemu_media_argv.rs` fd-index arg shape; `packages/d2b-core/src/processes.rs` `ProcessRole::QemuMediaRunner` sandbox/budget baseline |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/process_builder.rs |
| Detailed design | Process spec builder and LaunchTicket attachment resolution: build the canonical `qemu-media-runner` Process ResourceSpec from §10.1 and supply only opaque Network/Endpoint refs to Core's attachment resolver. Core, not the qemu controller, resolves authorized kvm, tap, media, and optional display attachments and seals their fd slots in the LaunchTicket. The qemu Provider/controller receives no broker operation or fd. No raw path, argv, executable path, or principal appears in any public field. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt sandbox/budget concepts to canonical Process resources and Core-sealed LaunchTickets; do not copy raw argv strings or path construction. |
| Integration | Controller emits Process resources; Core resolves private attachments; system-minijail/Process Provider consumes the sealed LaunchTicket. The NetworkEffectPort adapter transfers its connected tap `OwnedFd` directly to ProviderSupervisor without ResourceAPI, ComponentSession, or d2b-bus serialization; QEMU receives only the declared child fd slot. Endpoint resources represent QMP/serial connections. |
| Data migration | Full d2b 3.0 reset; existing QEMU runner process state is not imported and launch state is rebuilt from resources |
| Validation | `tests/process_spec_golden.rs`; `tests/launch_ticket_fd_slots.rs`; `tests/launch_ticket_tap_fd_lifetime.rs`; `tests/no_controller_fd_or_broker_op.rs`; `tests/no_raw_argv_in_spec.rs` |
| Removal proof | `ProcessRole::QemuMediaRunner` and raw qemu-media argv launch surfaces are removable after canonical Process specs and LaunchTickets cover every runner launch |

---

### ADR046-qemu-media-010 QMP endpoint attachment handling

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-009; owner: runtime-qemu-media QMP client implementation |
| Current source | `packages/d2b-host/src/media.rs` QMP command set; `packages/d2b-contracts/src/broker_wire.rs` `QemuMedia*` command payload shapes only |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/src/qmp/ |
| Detailed design | QMP endpoint attachment handling: consume `qmp` and `serial` Endpoint connection attachments delivered by the ProviderSupervisor ComponentSession channel; implement QMP capability negotiation, command dispatch, and health check using only the delivered fd, never direct socket path access. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt QMP command payloads to internal DTOs; discard broker wire ops and all socket path/fd-open code. |
| Integration | Process Provider publishes Endpoint attachments; the controller QMP client consumes those fds through ComponentSession; Guest status and health checks reflect QMP outcomes. |
| Data migration | Full d2b 3.0 reset; no v2 QMP socket path/session state is imported |
| Validation | `tests/qmp_capability_negotiation.rs`; `tests/qmp_command_dispatch.rs`; `tests/qmp_greeting_timeout.rs`; `tests/qmp_health_check.rs` |
| Removal proof | `QemuMedia*` broker wire operations are superseded as public control surfaces once QMP is driven solely through Endpoint attachments and internal DTOs |

---

### ADR046-qemu-media-011 Hotplug attach/detach protocol

| Field | Value |
| --- | --- |
| Dependency/owner | P2; depends on ADR046-qemu-media-006 and ADR046-qemu-media-010; owner: runtime-qemu-media hotplug controller |
| Current source | `packages/d2b-contracts/src/broker_wire.rs` `QemuMediaAttach` and `QemuMediaDetach` command bodies only |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/hotplug.rs |
| Detailed design | Hotplug attach/detach protocol: on `removableVolumeRefs` update, request a Volume fd from the `volume-local` ComponentSession service and issue `blockdev-add`/`device_add` QMP commands; reverse the sequence for detach; QMP failures set Degraded with `hotplug-media-failed`. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt QMP hotplug command bodies; delete broker op wiring. |
| Integration | Guest spec updates trigger controller reconcile; volume-local supplies media fds; QMP client executes attach/detach; Guest status records hotplug outcomes. |
| Data migration | Full d2b 3.0 reset; removable media hotplug state is reconciled from Guest spec and Volume status, not imported from broker op history |
| Validation | `tests/hotplug_attach_sequence.rs`; `tests/hotplug_detach_sequence.rs`; `tests/hotplug_qmp_failure.rs` |
| Removal proof | `QemuMediaAttach`/`QemuMediaDetach` broker operations are removed after hotplug is implemented through Volume fd acquisition plus QMP Endpoint dispatch |

---

### ADR046-qemu-media-012 Network attachment routing

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-002, ADR046-network-005, and Provider config `networkProviderRef`; owner: runtime-qemu-media network dependency integration |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/network.rs |
| Detailed design | Network attachment routing: project each Guest network attachment as an opaque `Network/<name>` ref and condition only. The network-local controller declares the opaque semantic effect; its Core-owned NetworkEffectPort adapter maps it to `CreatePersistentTap`, then `SetBridgePortFlags`, and transfers the already-authorized connected `OwnedFd` directly to ProviderSupervisor for the QEMU Process LaunchTicket. The adapter and supervisor keep `FD_CLOEXEC` set on parent copies; only the declared child slot is made inheritable immediately before exec. On cancellation, ticket rejection, or spawn failure, all fd copies close before generation-fenced `DeletePersistentTap`, and the opaque realization is retained until deletion confirmation. The qemu Provider/controller receives no broker operation, fd, bridge name, or interface name, and the fd is never serialized through ResourceAPI, ComponentSession, or d2b-bus. Primary reuse disposition: `create`. |
| Integration | Guest `networkAttachments` drive opaque dependency watches; ADR046-network-005 owns the NetworkEffectPort effect chain and ProviderSupervisor owns fd handoff. Process LaunchTicket carries the fd directly to QEMU; Guest conditions report authorization/resolution failures without exposing the fd or broker operation. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `tests/tap_launch_routing.rs` proves `CreatePersistentTap → SetBridgePortFlags → ProviderSupervisor LaunchTicket` ordering; `tests/tap_fd_lifetime.rs` proves CLOEXEC, single child ownership, and close-before-`DeletePersistentTap`; `tests/tap_fd_no_bus_serialization.rs` rejects fd/broker DTOs at the qemu controller boundary; `tests/tap_fd_unavailable.rs` covers authorization and resolution failure |
| Removal proof | None — Core-routed Network attachment resolution is a new v3 dependency path |

---

### ADR046-qemu-media-013 Reconcile loop and finalize

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-005 through ADR046-qemu-media-012; owner: runtime-qemu-media controller |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/reconcile.rs |
| Detailed design | Reconcile loop and finalize: implement the full async reconcile loop from §11.3 and finalize sequence from §11.4, including dependency gating, providerPhase transitions, condition management, runner exit handling, and WaylandSession cleanup. Primary reuse disposition: `create`. Preserved source-plan detail: net-new reconcile/finalize implementation using the v3 Resource API. |
| Integration | Resource watches feed the controller; the controller creates/updates/deletes Volume, WaylandSession, Endpoint, and Process resources; Guest status and finalizers expose lifecycle outcomes to core and CLI. |
| Data migration | Full d2b 3.0 reset; lifecycle state is re-derived from Resource specs/status and Operation ledger rather than imported from v2 daemon state |
| Validation | `tests/reconcile_dependency_gating.rs`; `tests/reconcile_runner_exit_handling.rs`; `tests/finalize_sequence.rs`; `tests/finalize_wayland_session_cleanup.rs` |
| Removal proof | Legacy daemon-owned qemu-media lifecycle paths can be removed once reconcile/finalize owns all Guest lifecycle transitions |

---

### ADR046-qemu-media-014 Status, conditions, and error reporting

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-013; owner: runtime-qemu-media status/error implementation |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/controller/status.rs |
| Detailed design | Status, conditions, and error reporting: implement all phase transitions from §16.1, providerPhase values from §16.2, condition types from §16.3, error codes from §16.4, and bounds enforcement on `providerPhase`. Primary reuse disposition: `create`. Preserved source-plan detail: net-new status/error projection for the v3 Guest ResourceType. |
| Integration | Controller reconcile writes Guest status; ResourceAPI stores bounded status; CLI/support tooling reads status without paths, argv, fds, socket paths, VM names as labels, or secret material. |
| Data migration | None — status schema is new v3 observation state; no v2 status import |
| Validation | `tests/status_phase_transitions.rs`; `tests/condition_reason_codes.rs` |
| Removal proof | None — this item adds v3 status projection and does not by itself remove a prior owner |

---

### ADR046-qemu-media-015 Audit event emission

| Field | Value |
| --- | --- |
| Dependency/owner | P2; depends on ADR046-qemu-media-013 and ADR046-qemu-media-014; owner: runtime-qemu-media audit integration |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/audit.rs |
| Detailed design | Audit event emission: emit all audit events in §17 and verify that no sensitive fields such as paths, argv, fds, or socket paths appear in any payload. Primary reuse disposition: `create`. Preserved source-plan detail: net-new audit emission for the Provider events in §17. |
| Integration | Controller lifecycle and QMP/hotplug operations call audit helpers; the audit subsystem records bounded event kinds and outcomes; support tooling consumes redacted payloads. |
| Data migration | None — audit-only work; no runtime state import |
| Validation | `tests/audit_event_shapes.rs`; `tests/audit_no_sensitive_fields.rs` |
| Removal proof | None — audit helpers are new for this Provider; no prior owner to remove |

---

### ADR046-qemu-media-016 Metrics and OTEL spans

| Field | Value |
| --- | --- |
| Dependency/owner | P2; depends on ADR046-qemu-media-013 and ADR046-qemu-media-014; owner: runtime-qemu-media telemetry integration |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/src/telemetry.rs |
| Detailed design | Metrics and OTEL spans: implement all metrics from §18 and OTEL trace spans with structural closed-label enforcement and no Zone/VM/resource name, user identity, path, or other sensitive value in any metric label. Span attributes use only the exact fixed semantic fields and `outcome` listed in §18; no resource name, UID, shortened UID, digest, ref, or derived identity is admitted. Retain identity only in allow-listed OTEL Resource attributes and permitted bounded audit fields. Primary reuse disposition: `create`. Preserved source-plan detail: net-new telemetry emission for the Provider metrics and spans in §18. |
| Integration | Controller, QMP, hotplug, and dependency-watch paths call telemetry helpers; OTEL/metrics exporters consume only closed, bounded labels for support dashboards. |
| Data migration | None — telemetry-only work; no runtime state import |
| Validation | `tests/metrics_label_cardinality.rs` asserts exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, and resource-name-derived keys plus Guest/Zone-name canary absence; `tests/otel_span_attributes.rs` asserts the exact per-span semantic allowlist, preserves allowed OTEL Resource identity attributes, and rejects Zone/Guest/Process/Provider-resource names, refs, UIDs, shortened UIDs, digests, and identity canary values in span attributes |
| Removal proof | None — telemetry helpers are new for this Provider; no prior owner to remove |

---

### ADR046-qemu-media-017 Nix module and assertions

| Field | Value |
| --- | --- |
| Dependency/owner | P1; depends on ADR046-qemu-media-002 and ADR046-qemu-media-003; owner: Nix resource compiler and runtime-qemu-media options |
| Current source | `nixos-modules/components/qemu-media.nix` option names only; `nixos-modules/assertions.nix` assertion framework |
| Reuse action | adapt |
| Destination | nixos-modules/options-guest-qemu-media.nix; nixos-modules/assertions.nix |
| Detailed design | Nix module and assertions: implement the Guest resource declaration from §19 and eval-time assertions from §19.8. Rewrite qemu-media options as v3 spec fields and reject raw path options. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt option names into v3 Guest/Provider spec fields; remove raw path options; extend existing assertion predicates. |
| Integration | Nix authoring emits Provider, Guest, Volume, and Device resource JSON; assertions fail invalid configs before build; emitted resources feed ResourceAPI admission and controller reconcile. |
| Data migration | Full d2b 3.0 reset; users reauthor qemu-media configuration as v3 resources and raw path options are not imported |
| Validation | `tests/unit/nix/cases/guest-qemu-media-spec.nix`; `tests/assertions-eval.sh` new assertion cases |
| Removal proof | `nixos-modules/components/qemu-media.nix` raw path option surface is superseded once v3 Guest resource emission and assertions cover the configuration |

---

### ADR046-qemu-media-018 d2b-provider-toolkit conformance

| Field | Value |
| --- | --- |
| Dependency/owner | P2; depends on ADR046-qemu-media-013 through ADR046-qemu-media-016; owner: runtime-qemu-media conformance tests |
| Current source | d2b-provider-toolkit conformance kit |
| Reuse action | adapt |
| Destination | packages/d2b-provider-runtime-qemu-media/tests/conformance_guest.rs |
| Detailed design | d2b-provider-toolkit conformance: pass the Provider conformance kit for the Guest ResourceType axis, including reconcile/finalize contract, phase machine, condition typing, audit shape, and telemetry cardinality. Primary reuse disposition: `adapt`. Preserved source-plan detail: reuse conformance harness; add runtime-qemu-media Guest ResourceType coverage. |
| Integration | Conformance tests instantiate the Provider against fake ResourceAPI/ComponentSession dependencies and verify the public Provider contract consumed by core CI. |
| Data migration | None — test-only work; no runtime state import |
| Validation | `make test-rust` (runs conformance suite) |
| Removal proof | None — conformance coverage is additive test proof |

---

### ADR046-qemu-media-019 Integration tests

| Field | Value |
| --- | --- |
| Dependency/owner | P2; depends on ADR046-qemu-media-005 through ADR046-qemu-media-018; owner: runtime-qemu-media integration fixtures |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-runtime-qemu-media/integration/ |
| Detailed design | Integration tests: implement container/fake-Host scenarios for full reconcile from Created to Ready with fake dependencies, finalize sequence, hotplug attach/detach, and restart recovery. Primary reuse disposition: `create`. Preserved source-plan detail: net-new integration fixtures. |
| Integration | Integration fixtures launch the Provider with fake or containerized Host/Guest/Volume/Network/Device dependencies; CI `make test-integration` consumes the fixtures as the cross-process proof lane. |
| Data migration | None — test-only work; no runtime state import |
| Validation | `make test-integration` |
| Removal proof | None — integration coverage is additive test proof |

---
## 22 Tests

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-runtime-qemu-media --lib --tests`
completes in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only — no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

### 22.1 Hermetic unit tests (`tests/`)

| Test file | Coverage |
| --- | --- |
| `guest_schema_roundtrip.rs` | GuestSpec/GuestStatus JSON Schema generation, serde, unknown-field denial |
| `guest_provider_settings_bounds.rs` | All field bounds in §5 |
| `config_schema_projection.rs` | ProviderConfig schema; controllerExecutionRef required |
| `state_status_spec.rs` | Component descriptor declares empty `stateNamespaces`; status projection stays within bounds and carries no secret/path/argv/PID/unit content |
| `state_status_restart.rs` | Controller re-derives observed state from store/ledger/external observation without a state Volume |
| `state_mount_exclusivity.rs` | Neither controller nor worker Process spec contains a Provider-state-Volume mount |
| `runtime_volume_spec.rs` | Runtime tmpfs Volume spec golden; all layout entries |
| `volume_cleanup_policy.rs` | `cleanupPolicy: vm-stop-with-proof` correctness |
| `media_volume_watch.rs` | Dependency gating for boot/removable Volume refs |
| `media_attachment_validation.rs` | Missing virtio-blk attachment → condition error |
| `kvm_device_watch.rs` | Device/host-kvm phase machine; Degraded/Failed propagation |
| `wayland_session_create.rs` | `display-wayland.d2bus.org.WaylandSession` resource type; no invented spec fields; no managedBy |
| `wayland_session_attachment_read.rs` | Opaque endpoint attachment consumed from display-wayland status |
| `wayland_session_missing_provider.rs` | displayWindow=true + null displayProviderRef → Failed |
| `process_spec_golden.rs` | Full canonical Process spec against §10.1 YAML |
| `launch_ticket_fd_slots.rs` | Core-sealed LaunchTicket fd table completeness; controller supplies only opaque refs |
| `launch_ticket_tap_fd_lifetime.rs` | Tap `OwnedFd` stays CLOEXEC in adapter/supervisor, becomes inheritable only in declared child slot, and closes on all failure paths |
| `no_controller_fd_or_broker_op.rs` | QEMU Provider/controller boundary rejects tap fds and broker DTOs |
| `no_raw_argv_in_spec.rs` | No executable path in any Process spec field |
| `qmp_capability_negotiation.rs` | QMP greeting exchange |
| `qmp_command_dispatch.rs` | All QMP commands in §13.2; success and error paths |
| `qmp_greeting_timeout.rs` | Timeout → qmp-greeting-timeout error code |
| `qmp_health_check.rs` | query-status; Degraded on consecutive failures |
| `hotplug_attach_sequence.rs` | blockdev-add + device_add via QMP attachment |
| `hotplug_detach_sequence.rs` | device_del + blockdev-del via QMP attachment |
| `hotplug_qmp_failure.rs` | QMP error → Degraded + `hotplug-media-failed` |
| `tap_launch_routing.rs` | `CreatePersistentTap → SetBridgePortFlags →` direct ProviderSupervisor LaunchTicket attachment ordering |
| `tap_fd_lifetime.rs` | Failed launch closes all copies before generation-fenced `DeletePersistentTap`; normal exit waits for QEMU closure |
| `tap_fd_no_bus_serialization.rs` | Tap fd never appears in ResourceAPI, ComponentSession, or d2b-bus payloads |
| `tap_fd_unavailable.rs` | Authorization/resolution failure → `network-tap-unavailable` Degraded |
| `reconcile_dependency_gating.rs` | All dependencies missing/present combinations |
| `reconcile_runner_exit_handling.rs` | Runner exit → Failed / re-create logic |
| `finalize_sequence.rs` | Finalizer drain order (runner → WaylandSession → Volume) |
| `finalize_wayland_session_cleanup.rs` | WaylandSession deleted before Volume |
| `status_phase_transitions.rs` | All phase and providerPhase transitions |
| `condition_reason_codes.rs` | All reason codes in §16.3 |
| `audit_event_shapes.rs` | Golden shape for every event in §17 |
| `audit_no_sensitive_fields.rs` | Property test: no path/argv/fd/socket-path in payload |
| `metrics_label_cardinality.rs` | Structural closed-label policy; exact identity-key absence; no Guest/Zone/resource-name canary or path in values; `d2b.zone` resource attribute retained |
| `otel_span_attributes.rs` | Exact fixed semantic attribute allowlist per span; Zone/Guest/Process/Provider-resource names, refs, UIDs, shortened UIDs, digests, and identity canaries rejected; allow-listed OTEL Resource identity retained |
| `provider_layout.rs` | Workspace layout conformance invocation |
| `conformance_guest.rs` | d2b-provider-toolkit conformance suite |

### 22.2 Integration tests (`integration/`)

| Test | Coverage |
| --- | --- |
| `full_lifecycle` | Create Guest, watch deps become Ready (fake), runner launches, QMP ready, finalize |
| `hotplug_roundtrip` | Attach removable Volume after boot; detach; verify attach status |
| `restart_after_runner_exit` | Runner exits → Failed → operator re-creates → Ready |
| `kvm_device_unavailable` | Device/host-kvm absent → Degraded; becomes Ready after Device becomes Ready |
| `display_session_cleanup` | Guest delete with displayWindow=true; WaylandSession deleted first |
| `media_volume_not_ready` | bootMediaRef Volume stuck in Pending → Guest Pending; unblocks when Ready |
| `paused_at_boot_resume` | pauseAtBoot=true; QMP cont issued after operator verb; providerPhase=running |

### 22.3 Nix eval tests

| Test | Coverage |
| --- | --- |
| `tests/unit/nix/cases/guest-qemu-media-spec.nix` | Minimal Guest + Volume + Device config; assert emitted spec shape |
| Additions to `tests/assertions-eval.sh` | All assertions in §19.8; positive and negative cases |

---

## 23 Removal proofs and migration map

### 23.1 Broker operations removed

The following broker op variants from `packages/d2b-contracts/src/broker_wire.rs`
are removed after `runtime-qemu-media` reaches `Ready` in all active Zones:

| Op | Baseline source | Removal gate |
| --- | --- | --- |
| `QemuMediaBoot` | `broker_wire.rs:BrokerOp::QemuMediaBoot` | When runtime-qemu-media provides all Guest startup |
| `QemuMediaAttach` | `broker_wire.rs:BrokerOp::QemuMediaAttach` | When hotplug fully Volume-mediated |
| `QemuMediaDetach` | `broker_wire.rs:BrokerOp::QemuMediaDetach` | When hotplug fully Volume-mediated |
| `QemuMediaStop` | `broker_wire.rs:BrokerOp::QemuMediaStop` | When Process desiredLifecycle controls shutdown |
| `QemuMediaStatus` | `broker_wire.rs:BrokerOp::QemuMediaStatus` | When Process.status conditions replace polling |
| `QemuMediaQueryBlock` | `broker_wire.rs:BrokerOp::QemuMediaQueryBlock` | When Volume attachment status replaces QMP direct poll |
| `QemuMediaResume` | `broker_wire.rs:BrokerOp::QemuMediaResume` | When operator verb triggers QMP cont via controller |
| `QemuMediaOpenDev` | `broker_wire.rs:BrokerOp::QemuMediaOpenDev` | When device-kvm Provider delivers fd via LaunchTicket |

**Gate condition:** `tests/unit/gates/qemu-media-broker-op-removal.sh` — asserts
no callers of the above ops exist in the daemon, CLI, or Nix emitters at
the removal commit.

### 23.2 ProcessRole::QemuMediaRunner removal

`packages/d2b-core/src/processes.rs` `ProcessRole::QemuMediaRunner` is
retired once all QemuMedia process launch flows are handled by the
`qemu-media-runner` Process resource. The removal commit must also delete
the corresponding Nix emitter section in
`nixos-modules/processes-json.nix`.

**Gate condition:** `tests/unit/gates/process-role-removal.sh` — asserts no
reference to `QemuMediaRunner` in any non-migration source file.

### 23.3 Nix option removals

| Current option | Removal trigger |
| --- | --- |
| `d2b.vms.<vm>.qemuMedia.*` (raw path/credential options) | Replaced by Volume + Device + Guest spec; removed in migration commit |
| `nixos-modules/components/qemu-media.nix` raw path options | Replaced by `nixos-modules/options-guest-qemu-media.nix` |

The migration guide `docs/how-to/migrate-qemu-media-v2-to-v3.md` (authored
in ADR046-qemu-media-017) explains the option mapping for operator configurations.

### 23.4 `docs/adr/0036-qemu-media-runtime.md` supersession

ADR 0036 is marked superseded by a `status: Superseded` header edit in
the same commit that lands the first production-ready release of
`Provider/runtime-qemu-media`. The removal gate (`tests/static.sh` drift
check) verifies the superseded field is present.

### 23.5 CHANGELOG entry

The CHANGELOG.md `## [Unreleased]` block receives an `Added` entry at the
ADR046-qemu-media-001 commit and a `Changed` entry noting the removal of `QemuMedia*` broker
ops at the removal-gate commit. No process markers appear in the CHANGELOG
text.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
