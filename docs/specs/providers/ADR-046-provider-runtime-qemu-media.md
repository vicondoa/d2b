# ADR 0046 Provider: runtime-qemu-media

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-runtime-qemu-media` |
| Parent | ADR 0046 |
| Status | Proposed |
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
5. [Guest providerSettings schema](#5-guest-providersettings-schema)
6. [Volume resources](#6-volume-resources)
7. [Network dependency](#7-network-dependency)
8. [Device dependencies](#8-device-dependencies)
9. [WaylandSession dependency (display-wayland)](#9-waylandsession-dependency)
10. [Process templates](#10-process-templates)
11. [Controller reconcile and finalize loop](#11-controller-reconcile-and-finalize-loop)
12. [Guest boot sequence](#12-guest-boot-sequence)
13. [QMP protocol via Process endpoint](#13-qmp-protocol-via-process-endpoint)
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
typed `Guest.status.providerPhase = "paused-at-boot"` state.

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
apiVersion: resources.d2b.io/v3
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
| `pausedAtBootDefault` | bool | no | `true` | — | Default `pauseAtBoot` if not set in Guest providerSettings |
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
apiVersion: resources.d2b.io/v3
type: Guest
metadata:
  name: corp-iso-boot
  zone: corp
  ownerRef: null
  finalizers:
    - runtime-qemu-media.d2b.io/guest-cleanup
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
  providerSettings:                         # see §5
    bootMediaRef: Volume/corp-iso-boot-media
    bootMediaView: guest-attach
    removableVolumeRefs:
      - volumeRef:  Volume/corp-usb-stick
        view:       guest-attach
    vcpu: 4
    memoryMib: 8192
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

## 5 Guest providerSettings schema

`providerSettings` is a bounded map validated against the Provider's signed
JSON Schema. It contains Guest-level settings for the VMM; no raw paths,
executable paths, argv fragments, or credential bytes appear here.

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `bootMediaRef` | ResourceRef? | no | `null` | `Volume/<n>` | Primary boot Volume; nil = direct kernel boot if kernelArtifactId set (not yet supported) |
| `bootMediaView` | string | no | `"guest-attach"` | `^[a-z][a-z0-9-]*$` | View within the boot Volume from which the controller derives the virtio-blk attachment |
| `removableVolumeRefs` | list | no | `[]` | max 4 entries | Runtime-hotpluggable media Volumes |
| `removableVolumeRefs[].volumeRef` | ResourceRef | yes | — | `Volume/<n>` | Removable media Volume |
| `removableVolumeRefs[].view` | string | yes | — | `^[a-z][a-z0-9-]*$` | View within the Volume for guest access |
| `vcpu` | u16 | no | `2` | 1–128 | vCPU count |
| `memoryMib` | u32 | no | `2048` | 128–524288 | RAM in MiB |
| `cpuModel` | string | no | `"host"` | `host\|max\|qemu64` | CPU model string; sealed set |
| `machineType` | string | no | `"q35"` | `q35\|pc` | QEMU machine type |
| `bios` | string | no | `"ovmf"` | `ovmf\|seabios` | Firmware type |
| `pauseAtBoot` | bool | no | `true` | — | If true, start QEMU in `\-S` mode (paused); operator issues QMP `cont` to release |
| `displayWindow` | bool | no | `false` | — | If true, controller creates a `WaylandSession` resource for `Provider/display-wayland` |
| `serialConsole` | bool | no | `true` | — | Expose serial console via Process endpoint |
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
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: <guest-uid-short>-runtime
  zone: corp
  ownerRef: Guest/corp-iso-boot     # controller sets ownerRef to the owning Guest
  finalizers:
    - runtime-qemu-media.d2b.io/runtime-volume
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
apiVersion: resources.d2b.io/v3
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

`ProviderStateSet(zone, "runtime-qemu-media")` is the query-time set of all
Volume resources in the Zone whose `metadata.ownerRef` equals
`Provider/runtime-qemu-media`. It is not a ResourceType and is not stored;
it is derived by querying the Volume owner index.

The controller declares one `stateNamespace` in its component descriptor
(initial release: empty payload schema, `kind: state`,
`persistenceClass: persistent`, `quota.maxBytes: 1048576`,
`quotaBytes: 1048576`). Core `ProviderDeployment` creates the corresponding
state Volume **before** the controller Process is started. The controller
cannot create its own prerequisite. The canonical Volume ResourceSpec is:

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: runtime-qemu-media--controller--state--host-system
  zone: system
  ownerRef: Provider/runtime-qemu-media
spec:
  providerRef: Provider/volume-local
  kind: state
  persistenceClass: persistent           # durable: survives restart, upgrade, reset
  sensitivityClass: private
  stateSchema:
    schemaId: io.d2b.runtime-qemu-media/controller/state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>            # signed into component descriptor
    migrationPolicy: none                 # empty payload schema; no migration worker
  quota:
    maxBytes: 1048576                     # base Volume quota; nonzero even for empty payload
    maxInodes: 1024
  quotaBytes: 1048576                     # provider-state extension quota (mirrors base)
  sealingCredentialRef: null
  source:
    executionRef: Host/host-system        # resolved from config.controllerExecutionRef
    settings:
      kind: local-path
      sourcePolicyId: runtime-qemu-media-controller-state  # opaque; volume-local resolves backing path
  layout:
    - path: state
      type: directory
      ownerRef: User/runtime-qemu-media-system    # Nix-preprovisioned principal
      groupRef: User/runtime-qemu-media-system
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
  views:
    main:
      path: state
      rights: [read, write, create, delete, traverse]
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
  snapshotPolicy: null
  retentionPolicy: null
```

`ProviderDeployment` creates this Volume, waits for `Provider/volume-local`
to report `phase: Ready`, then starts the controller Process with the Volume
pre-mounted. The controller Process mounts the `main` view as a dirfd at
`/state` and consumes it; it does not watch, create, delete, or otherwise
reconcile the Volume. `Provider/volume-local` is the sole Volume reconciler.
Volume is not in this Provider's exported ResourceTypes. `ProviderDeployment`
sets `metadata.deletionRequestedAt` on this Volume only after the controller
Process drain completes; the controller does not set it.

Controller-created Guest runtime Volumes carry `ownerRef: Guest/<name>` and
do not appear in the ProviderStateSet. Operator-authored media Volumes carry
`ownerRef: null` and also do not appear.

---

## 7 Network dependency

The controller declares a dependency alias `network → Provider/network-local`
(bound via `config.networkProviderRef`). For each Guest with a non-empty
`spec.networkAttachments`, the controller watches the referenced Network
resources for `Ready` status.

When a Network resource is `Ready`, `Provider/network-local` has allocated a
tap fd for the Guest's MAC address and bridge assignment. The controller
requests the tap fd delivery through the network-local ComponentSession
service. The tap fd is then included in the runner Process LaunchTicket's
inherited fd table. QEMU receives the tap interface via a sealed fd slot; no
bridge name, interface name, or host network path crosses the public surface.

If `spec.networkAttachments` is empty, the runner starts with no network
interface (isolated).

---

## 8 Device dependencies

### 8.1 Device/host-kvm

KVM acceleration is an explicit `Device` resource dependency, not an
implicit Host capability. The operator must declare `Device/host-kvm` in the
Zone and the Guest must list `Device/host-kvm` in `spec.deviceAttachments`.

```yaml
apiVersion: resources.d2b.io/v3
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
Wayland socket. When `providerSettings.displayWindow = true`, the controller
creates a `display-wayland.d2b.io.WaylandSession` resource in the same Zone,
using the exact ResourceSpec defined by `Provider/display-wayland`'s dossier
(including its required `guestRef`, `hostRef`, `userRef`, `policy`, `identity`,
and `device` fields as applicable). The `runtime-qemu-media` controller is the
resource owner (`ownerRef: Guest/<name>`) and is responsible for creating,
updating, and deleting it; it does not invent additional spec fields.

```yaml
apiVersion: resources.d2b.io/v3
type: display-wayland.d2b.io.WaylandSession
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

The controller watches `display-wayland.d2b.io.WaylandSession/<guest-uid-short>-display`
for `phase: Ready`. When `display-wayland` sets the session `Ready`, it
writes a typed endpoint attachment to the session's status. The
`runtime-qemu-media` controller reads this opaque attachment (whose exact
status field names are defined by the `display-wayland` dossier) and
includes the corresponding display fd in the runner LaunchTicket.

The `display-wayland` Provider owns all proxy Process instances internally.
`runtime-qemu-media` only:

1. creates/updates/deletes the `display-wayland.d2b.io.WaylandSession`
   resource as an owner, and
2. consumes the opaque endpoint attachment from that session's `Ready` status.

If `providerSettings.displayWindow = false`, the controller does not create a
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
apiVersion: resources.d2b.io/v3
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
  # The tap fd is pre-connected and delivered via the LaunchTicket inherited fd
  # table (acquired from the network-local ComponentSession before runner
  # creation). The runner process holds no live network stack; networkUsage is null.
  networkUsage: null

  # --- Device ---
  deviceUsage:
    - deviceRef: Device/host-kvm
      access: shared
      purpose: kvm-acceleration

  # --- Endpoints ---
  endpoints:
    - name: qmp
      transport: unix
      purpose: qmp-control            # QMP socket; Process Provider creates it; no public path
    - name: serial
      transport: unix
      purpose: serial-console         # serial console socket; private; no public path

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
    class: provider-defined            # ready when QMP greeting received via Process endpoint

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
- No raw principals, argv strings, host paths, executable paths, or
  credential bytes appear in this spec.

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
apiVersion: resources.d2b.io/v3
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
  mounts:
    - volumeRef: Volume/runtime-qemu-media--controller--state--host-system
      view: main
      mountPath: /state
      access: read-write
      required: true
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
  endpoints: []
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
| `display-wayland.d2b.io.WaylandSession` | Guest ownerRef | Display compositor readiness |
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
      set Guest.providerPhase = "waiting-dependencies"
      return (requeue on dep watch event)
  ensure runtime tmpfs Volume exists and is Ready
  ensure runner Process spec is current (create or UpdateSpec)
  IF runner Process.phase = Ready:
    set Guest.phase = Ready
    set Guest.providerPhase = "paused-at-boot" (if pauseAtBoot) or "running"
  ELIF runner Process.phase = Failed:
    set Guest.phase = Failed
    set Guest.providerPhase = "runner-failed"
  ELIF runner Process.phase = Pending | Degraded:
    set Guest.phase = Degraded
    set Guest.providerPhase = "runner-starting"
```

The controller does not hold a queue slot during dependency waits; it
returns immediately and is re-triggered when the dependency watch event fires.

### 11.4 Finalize logic

```
OBSERVE: Guest.finalizers contains runtime-qemu-media.d2b.io/guest-cleanup
  set runner Process.desiredLifecycle = stopped (or delete Process resource)
  wait for Process.phase = Succeeded | Failed (via Process watch event)
  IF displayWindow: delete WaylandSession resource; wait for Deleted
  set runtime Volume.desiredLifecycle = deleted (via Volume finalizer drain)
  wait for Volume.phase = Deleted (via Volume watch event)
  remove runtime-qemu-media.d2b.io/guest-cleanup from Guest.finalizers
  → core removes Guest row; emits ResourceDeleted audit event
```

The finalizer never forcefully unlinks socket paths or sends signals to
processes outside the provider-owned resource graph. The runner's runtime
Volume finalizer (`runtime-qemu-media.d2b.io/runtime-volume`) ensures the
tmpfs is unmounted only after the runner Process pidfd signals exit.

---

## 12 Guest boot sequence

1. **Dependencies check (async watch):** Controller observes via watch events
   that `Device/host-kvm.phase = Ready`, all `Volume/<media>.phase = Ready`
   (for each ref in providerSettings), and (if displayWindow)
   `WaylandSession.phase = Ready`. No blocking loop; controller re-queues on
   watch events.

2. **Runtime Volume:** Controller creates `<guest-uid-short>-runtime` Volume
   if absent. Waits for `Volume.phase = Ready`.

3. **Tap fd acquisition:** Controller calls `network-local` ComponentSession
   service to obtain the tap fd for the Guest's Network attachment. The fd is
   added to the LaunchTicket.

4. **Media fd acquisition:** Volume controller (provider/volume-local) makes
   the boot media fd available via the Volume's virtio-blk attachment. The
   fd is included in the runner LaunchTicket as a sealed slot.

5. **KVM fd acquisition:** `device-kvm` Provider delivers the `/dev/kvm` fd
   via the LaunchTicket device fd table.

6. **Display handle (if displayWindow):** Controller reads the opaque
   endpoint attachment from
   `display-wayland.d2b.io.WaylandSession/<guest-uid-short>-display`
   status (field names owned by the `display-wayland` dossier). Includes
   the corresponding display fd in the LaunchTicket for QEMU's display
   backend.

7. **Runner Process creation:** Controller creates (or UpdateSpec) the
   `<guest-uid-short>-qemu-runner` Process resource with a fully-sealed
   LaunchTicket and all fd slots resolved.

8. **ProviderSupervisor launch:** `system-minijail` ProviderSupervisor
   verifies the LaunchTicket, compiles the minijail sandbox plan, and spawns
   QEMU via `clone3(CLONE_PIDFD)` into the correct cgroup leaf. QEMU receives
   all required fds via its inherited fd table; no paths cross the supervisor
   boundary.

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

## 13 QMP protocol via Process endpoint

The QMP socket is a private `Process` endpoint declared in the runner spec
(`endpoints[name=qmp, transport=unix, purpose=qmp-control]`). It is not a
broker operation target and no public path is exposed.

### 13.1 Connection attachment delivery

When the runner Process reaches `Ready`, the `system-minijail` Process
Provider delivers a validated local connection attachment for each declared
endpoint to the controller via the ProviderSupervisor ComponentSession
channel. The attachment for the `qmp` endpoint is a sealed connection handle
(an owned fd to the QMP socket). The attachment for the `serial` endpoint
is delivered similarly.

The controller uses the `qmp` attachment fd to negotiate the QMP capability
exchange and then issue commands. Neither the fd number nor any socket path
is written to the resource store, status fields, audit events, or OTEL spans.

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
`spec.providerSettings.removableVolumeRefs` list. The controller observes
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
| `QemuMediaAttach` (opens image/usb fd, QMP blockdev-add) | Replaced by Volume attachment update + QMP command via Process endpoint connection |
| `QemuMediaDetach` (QMP device_del, closes fd) | Replaced by Volume attachment delete + QMP command via Process endpoint connection |
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
| `TapFdDeliver` (network-local family) | Deliver tap fd for Guest MAC/bridge assignment | Issued by `network-local` Provider; consumed by `runtime-qemu-media` controller as a fd slot |

`runtime-qemu-media` does not issue any broker operations directly. It
communicates with `network-local` and `device-kvm` via their ComponentSession
service contracts (d2b-bus), not via broker wire ops.

---

## 15 RBAC and permission claims

### 15.1 Declared permission claims (in Provider manifest)

| Claim | Target type | Verbs | Purpose |
| --- | --- | --- | --- |
| `guest-reconcile` | Guest | get, list, watch, create, update, delete | Own ResourceType |
| `process-manage` | Process | get, list, watch, create, update, delete | Runner process lifecycle |
| `volume-watch-media` | Volume | get, list, watch | Watch media Volume status |
| `volume-create-runtime` | Volume | get, list, watch, create, update, delete | Create/delete runtime tmpfs Volume |
| `network-watch` | Network | get, list, watch | Watch Network readiness |
| `device-kvm-watch` | Device | get, list, watch | Watch Device/host-kvm status |
| `waylandsession-manage` | display-wayland.d2b.io.WaylandSession | get, list, watch, create, update, delete | Create/delete WaylandSession for display |
| `user-watch` | User | get, list, watch | Resolve Guest userRef |

### 15.2 Operator RoleBindings required

The operator must grant the controller's identity (auto-created Service
Account for `Provider/runtime-qemu-media`) at least the following:

```yaml
# Minimum operator RoleBinding for the controller
rules:
  - resources: [Guest]
    verbs: [get, list, watch, create, update, delete]
  - resources: [Process, EphemeralProcess]
    verbs: [get, list, watch, create, update, delete]
  - resources: [Volume]
    verbs: [get, list, watch, create, update, delete]
  - resources: [Network]
    verbs: [get, list, watch]
  - resources: [Device]
    verbs: [get, list, watch]
  - resources: [display-wayland.d2b.io.WaylandSession]
    verbs: [get, list, watch, create, update, delete]
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

`Guest.status.providerPhase` is a bounded string carrying backend lifecycle
detail. Transitions are controlled by the controller only:

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
| `network-tap-unavailable` | Degraded | network-local could not deliver tap fd |

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

All metric labels use closed sets. Cardinality is bounded; no label value
may contain VM name, user identity, executable path, or VM memory content.

| Metric | Type | Labels | Notes |
| --- | --- | --- | --- |
| `d2b_guest_reconcile_total` | counter | zone, provider, outcome (success/failure) | Reconcile loop outcomes |
| `d2b_guest_reconcile_duration_seconds` | histogram | zone, provider | Reconcile latency |
| `d2b_guest_phase_transitions_total` | counter | zone, provider, from_phase, to_phase | Phase machine |
| `d2b_guest_runner_launches_total` | counter | zone, provider, outcome | Launch attempts |
| `d2b_guest_qmp_ready_seconds` | histogram | zone, provider | Time from runner spawn to QMP greeting |
| `d2b_guest_qmp_operations_total` | counter | zone, provider, operation, outcome | Per-operation QMP results |
| `d2b_guest_media_hotplug_total` | counter | zone, provider, operation (attach/detach), outcome | |
| `d2b_guest_dependency_wait_seconds` | histogram | zone, provider, dep_type | Time waiting for dependency |
| `d2b_guest_active` | gauge | zone, provider, phase | Active Guest count per phase |
| `d2b_guest_runner_restart_total` | counter | zone, provider | Runner exits (controller re-creates) |

OTEL trace spans:

| Span | Parent | Attributes |
| --- | --- | --- |
| `guest.reconcile` | — | zone, provider, guest_uid_short, phase |
| `guest.runner.launch` | `guest.reconcile` | zone, provider, process_ref_uid_short |
| `guest.qmp.connect` | `guest.runner.launch` | zone, provider |
| `guest.qmp.command` | `guest.reconcile` | zone, provider, command (closed set) |
| `guest.media.hotplug` | `guest.reconcile` | zone, provider, operation |
| `guest.finalize` | `guest.reconcile` | zone, provider |

No span attribute may carry: argv, executable paths, VM memory, host fs
paths, fds, socket paths, raw process output, or user-supplied opaque data.
`guest_uid_short` = first 12 hex chars of Guest UID (not human name).

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

### 19.3B Controller state Volume (ProviderDeployment-created; shown for reference)

Core `ProviderDeployment` creates this Volume before the controller Process
starts. It is **not** authored by the operator. The operator's NixOS module
must provision the layout principal `User/runtime-qemu-media-system`:

```nix
# nixos-modules/provider-users.nix — provisioned automatically by module activation
users.users."runtime-qemu-media-system" = {
  isSystemUser = true;
  group        = "runtime-qemu-media-system";
};
users.groups."runtime-qemu-media-system" = {};
```

The corresponding Zone resource (ProviderDeployment-created at runtime; not authored in Nix):

```yaml
# ProviderDeployment creates this before the controller Process starts; not in Nix
type: Volume
metadata:
  name: runtime-qemu-media--controller--state--host-system
  ownerRef: Provider/runtime-qemu-media
spec:
  providerRef: Provider/volume-local
  kind: state
  persistenceClass: persistent
  sensitivityClass: private
  stateSchema:
    schemaId: io.d2b.runtime-qemu-media/controller/state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: none                 # empty payload schema; no migration worker
  quota:
    maxBytes: 1048576     # base Volume quota; nonzero even for empty payload
    maxInodes: 1024
  quotaBytes: 1048576     # provider-state extension quota
  source:
    executionRef: Host/host-system
    settings:
      kind: local-path
      sourcePolicyId: runtime-qemu-media-controller-state
  layout:
    - path: state
      type: directory
      ownerRef: User/runtime-qemu-media-system
      groupRef: User/runtime-qemu-media-system
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
  views:
    main:
      path: state
      rights: [read, write, create, delete, traverse]
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
```

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
    providerSettings = {
      bootMediaRef  = "Volume/corp-iso-boot-media";
      bootMediaView = "guest-attach";
      vcpu          = 4;
      memoryMib     = 8192;
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
    providerSettings = {
      bootMediaRef  = "Volume/corp-win-installer";
      bootMediaView = "guest-attach";
      removableVolumeRefs = [{
        volumeRef = "Volume/corp-drivers-usb";
        view      = "guest-attach";
      }];
      vcpu          = 2;
      memoryMib     = 4096;
      pauseAtBoot   = false;
      displayWindow = true;     # controller will create WaylandSession
      serialConsole = false;
      tablet        = true;
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

`ProviderStateSet(zone, "runtime-qemu-media")` is the query-time grouping of
all Volume resources in the Zone whose `metadata.ownerRef` resolves to
`Provider/runtime-qemu-media`. It is not a ResourceType, not a stored
artifact, and has no "compartments". The set is derived by querying the Zone
resource store's owner index.

### 20.1 Controller state Volume

Core `ProviderDeployment` creates exactly one state Volume before the
controller Process is started. The controller cannot create its own
prerequisite Volume:

| Field | Value |
| --- | --- |
| Name | `runtime-qemu-media--controller--state--host-system` |
| Creator | `ProviderDeployment` (core); controller watches, does not create |
| `ownerRef` | `Provider/runtime-qemu-media` |
| `providerRef` | `Provider/volume-local` |
| `kind` | `state` |
| `persistenceClass` | `persistent` (durable: survives restart, upgrade, destroy/reset lifecycle) |
| `sensitivityClass` | `private` |
| `stateSchema.schemaId` | `io.d2b.runtime-qemu-media/controller/state` |
| `stateSchema.schemaVersion` | `"1.0"` |
| `stateSchema.migrationPolicy` | `none` (empty payload schema; no migration worker) |
| `quota.maxBytes` | `1048576` (base Volume quota; nonzero even for empty payload) |
| `quota.maxInodes` | `1024` |
| `quotaBytes` | `1048576` (provider-state extension; participates in quota enforcement) |
| `source.settings.sourcePolicyId` | `runtime-qemu-media-controller-state` (opaque; volume-local resolves backing path) |
| Layout principal | `User/runtime-qemu-media-system` (Nix-preprovisioned) |
| View | `main` — path `state`, rights `[read, write, create, delete, traverse]` |
| `identityMarker.class` | `broker-maintained` |

The canonical Volume ResourceSpec is in §6.4.

### 20.2 Controller Process mount

The controller Process mounts this Volume via the `main` view:

```yaml
mounts:
  - volumeRef: Volume/runtime-qemu-media--controller--state--host-system
    view: main
    mountPath: /state
    access: read-write
    required: true
```

The `sensitivityClass: private` constraint enforces that exactly one process
instance mounts this Volume at a time. No worker Process, no other component,
and no operator-authored or Guest-owned Volume shares this Volume or its view.

### 20.3 What is not in the ProviderStateSet

- Runtime tmpfs Volumes for Guest runners carry `ownerRef: Guest/<name>` —
  they are Guest resources, not Provider state.
- Operator-authored boot/removable media Volumes carry `ownerRef: null` —
  they are not Provider state.
- The controller state Volume has an empty payload schema in the initial
  release but is still `kind: state`, `persistenceClass: persistent`, with
  a minimal nonzero quota and an identity marker. It survives component and
  Provider restart and participates in upgrade, destroy, and reset flows.
  A future revision that adds persistent per-provider registry state
  (for example, a source-policy cache) adds a new `stateNamespace` to the
  component descriptor and creates an additional Volume with
  `ownerRef: Provider/runtime-qemu-media`.

### 20.4 Layout principal

`User/runtime-qemu-media-system` is a system user provisioned by the NixOS
module (`nixos-modules/provider-users.nix`) as part of host activation. The
volume-local Provider resolves this `User/<name>` reference at provision time
through the Host's User resource to obtain the numeric uid/gid. No
`ComponentPrincipal` ResourceRef is used; the layout always binds named
`User/<name>` references from the Nix-preprovisioned pool.

### 20.5 Destruction

When `Provider/runtime-qemu-media` receives `metadata.deletionRequestedAt`,
`ProviderDeployment` (core):

1. Signals the controller Process to drain (`desiredLifecycle: stopped`);
   waits for Process finalizer to complete.
2. Sets `metadata.deletionRequestedAt` on the controller state Volume after
   the Process drain is confirmed. The controller does not trigger this step.
3. The volume-local Provider destroys the Volume (layout removal, marker
   removal, finalizer commit).
4. Core removes the Provider row and emits `ResourceDeleted`.

---

## 21 Implementation work items

Each work item includes the source it adapts (baseline `b5ddbed6`) and the
destination in `packages/d2b-provider-runtime-qemu-media/`.

---

### WI-001 Crate scaffold and layout gate

**Priority:** P0 (blocks all others)

**Description:** Create the crate with the four required paths; commit a
`README.md` stub; verify `make test-policy` passes.

**Source:** none (new crate)

**Destination:**
```
packages/d2b-provider-runtime-qemu-media/
  src/lib.rs                  # minimal stub
  tests/provider_layout.rs    # layout conformance invocation
  integration/mod.rs          # placeholder
  README.md                   # min 200 bytes; see §1 requirements
```

**Tests:** `make test-policy` (workspace policy gate)

---

### WI-002 Guest ResourceType schema and serde

**Priority:** P0

**Description:** Define `GuestSpec`, `GuestStatus`, `GuestProviderSettings`
Rust types with serde/JSON Schema. Derive JSON Schema via `schemars`. Fields
must match §4, §5, and §16 exactly.

**Source (ADAPT):**
- `packages/d2b-core/src/host.rs` — `HostQemuMedia`, `QemuMediaSourceIntent`:
  extract field names/types; discard raw path/credential fields.

**Destination:** `src/types/guest.rs`

**Invariants to enforce:**
- `bootMediaRef` is a ResourceRef (`Volume/<n>`), not a path
- `removableVolumeRefs` max 4 entries
- `providerPhase` max 64 chars; closed value set
- No `argv`, path, or credential byte in any serialized type

**Tests:**
- `tests/guest_schema_roundtrip.rs` — JSON Schema generation, serde
  round-trips, unknown field denial
- `tests/guest_provider_settings_bounds.rs` — field bound validation

---

### WI-003 Provider config schema and projection

**Priority:** P0

**Description:** Define `ProviderConfig` Rust type; JSON Schema derivation;
`controllerExecutionRef` required field; project to controller component only.

**Source (ADAPT):**
- `packages/d2b-core/src/runtime.rs` — extract relevant timeout/quota fields

**Destination:** `src/config.rs`

**Tests:** `tests/config_schema_projection.rs`

---

### WI-003B Controller state Volume (ProviderDeployment-created)

**Priority:** P0

**Description:** The controller state Volume is created and deleted by core
`ProviderDeployment`; the semantic controller does not own, watch, create,
or delete it. `Provider/volume-local` is the sole Volume reconciler. Volume
is not in this Provider's exported ResourceTypes. This WI covers:

1. **Component descriptor declaration**: the controller component descriptor
   declares one `stateNamespace` with `schemaId`, `schemaVersion`,
   `schemaDigest`, `migrationPolicy: none` (empty payload; no migration
   worker), `quota.maxBytes`, `quota.maxInodes`, `quotaBytes`, and the
   `main` view; this drives ProviderDeployment's Volume creation.
2. **View consumption only**: the controller Process receives the `main`
   view pre-mounted at `/state` as a dirfd; it reads/writes state through
   the volume-local-enforced view boundary. It holds no ResourceClient
   handle to the Volume and issues no Volume API calls (no create, watch,
   update, or deletionRequestedAt).
3. **No controller-side ownership**: the controller holds no finalizer on
   the state Volume and does not add Volume to its owned resource set.
   ProviderDeployment sets `deletionRequestedAt` after Process drain.

Full Volume spec (§6.4): `ownerRef: Provider/runtime-qemu-media`,
`kind: state`, `persistenceClass: persistent`, `migrationPolicy: none`,
`quota.maxBytes/maxInodes: 1048576/1024`, `quotaBytes: 1048576`,
`source.settings.sourcePolicyId: runtime-qemu-media-controller-state`,
`User/runtime-qemu-media-system` layout principal, `main` view,
`identityMarker: broker-maintained`. Worker Processes receive no mount;
`sensitivityClass: private` is enforced at admission.

**Source:** new (no baseline equivalent)

**Destination:** `src/descriptor.rs` (stateNamespace declaration in
component descriptor); no `state_volume.rs` — the controller has no Volume
management code.

**Tests:**
- `tests/state_volume_spec.rs` — component descriptor's stateNamespace
  fields match §6.4: migrationPolicy none, quota.maxBytes/maxInodes +
  quotaBytes all nonzero, source.settings.sourcePolicyId, layout
  ownerRef/mode, view rights, identityMarker class
- `tests/state_volume_principal.rs` — layout principal resolves to
  `User/runtime-qemu-media-system`; no ComponentPrincipal refs; no
  cross-component shared Volume
- `tests/state_volume_mount_exclusivity.rs` — worker Process spec contains
  no mount referencing the controller state Volume
- `tests/state_volume_no_controller_ops.rs` — controller reconcile handler
  issues no Volume create, watch, update, or deletionRequestedAt for the
  state namespace; Volume is absent from the controller's ResourceClient
  permission set

---



**Priority:** P1

**Description:** Controller creates the per-Guest runtime tmpfs Volume as
specified in §6.1. Spec must exactly match the canonical YAML including all
layout entries, views, and quota.

**Source (ADAPT):**
- `packages/d2b-host/src/qemu_media_argv.rs` — `run_dir` and socket path
  derivation (extract naming pattern; discard raw path construction)

**Destination:** `src/controller/volume.rs`

**Tests:**
- `tests/runtime_volume_spec.rs` — emitted Volume spec golden test; field
  by field validation; layout entry completeness
- `tests/volume_cleanup_policy.rs` — verify `cleanupPolicy: vm-stop-with-proof`

---

### WI-005 Media Volume watch and virtio-blk attachment validation

**Priority:** P1

**Description:** Controller watches `bootMediaRef` and `removableVolumeRefs`
Volumes for `Ready` status and validates that each has a `virtio-blk`
attachment for the owning Guest. No path inspection.

**Source (ADAPT):**
- `packages/d2b-core/src/host.rs` `QemuMediaSourceKind` — media kind
  enumeration; map to Volume source kind assertions.

**Destination:** `src/controller/media_watch.rs`

**Tests:**
- `tests/media_volume_watch.rs` — fake Volume in Pending/Ready/Failed states;
  dependency gating logic
- `tests/media_attachment_validation.rs` — missing attachment → condition error

---

### WI-006 KVM Device watch

**Priority:** P1

**Description:** Controller watches `Device/host-kvm` from
`spec.deviceAttachments` for `Ready` status and gates runner launch on it.

**Destination:** `src/controller/device_watch.rs`

**Tests:**
- `tests/kvm_device_watch.rs` — Device Pending / Ready / Failed state
  transitions; condition propagation to Guest

---

### WI-007 WaylandSession resource management

**Priority:** P1

**Description:** When `providerSettings.displayWindow = true`, controller
creates/updates/deletes a `display-wayland.d2b.io.WaylandSession` resource
(§9) using the exact ResourceSpec defined by the `display-wayland` dossier.
Watches for `Ready` and reads the opaque endpoint attachment from status
(field names defined by `display-wayland` dossier).

**Destination:** `src/controller/display.rs`

**Tests:**
- `tests/wayland_session_create.rs` — emitted resource type is
  `display-wayland.d2b.io.WaylandSession`; no invented spec fields; no
  `managedBy` in metadata
- `tests/wayland_session_attachment_read.rs` — opaque endpoint attachment
  parsed from status without inventing field names
- `tests/wayland_session_missing_provider.rs` — displayProviderRef=null +
  displayWindow=true → Failed + `display-provider-not-configured`

---

### WI-008 Process spec builder and LaunchTicket assembly

**Priority:** P1

**Description:** Build the canonical `qemu-media-runner` Process ResourceSpec
(§10.1). Assemble the LaunchTicket with all sealed fd slots (kvm fd, tap fd,
media fds, display fd if applicable). No raw path, argv, or principal in any
field.

**Source (ADAPT):**
- `packages/d2b-host/src/qemu_media_argv.rs` — extract arg shape for fd
  indices; rewrite as sealed fd table declarations (do not copy raw argv
  strings or path construction)
- `packages/d2b-core/src/processes.rs` `ProcessRole::QemuMediaRunner` —
  extract sandbox/budget baseline

**Destination:** `src/controller/process_builder.rs`

**Tests:**
- `tests/process_spec_golden.rs` — emitted Process spec against §10.1 YAML;
  field-by-field validation
- `tests/launch_ticket_fd_slots.rs` — fd table completeness; no path in slots
- `tests/no_raw_argv_in_spec.rs` — assert no executable path string in any
  Process spec field

---

### WI-009 QMP endpoint attachment handling

**Priority:** P1

**Description:** Consume the `qmp` and `serial` endpoint connection attachments
delivered by the ProviderSupervisor ComponentSession channel (§13.1).
Implement QMP capability negotiation, command dispatch (§13.2), and health
check. Use only the attachment fd delivered by the Process Provider; no
direct socket path access.

**Source (ADAPT):**
- `packages/d2b-host/src/media.rs` — QMP command set; adapt to typed
  attachment; discard all socket path / fd-open code
- `packages/d2b-contracts/src/broker_wire.rs` `QemuMedia*` — command payload
  shapes (DISCARD the broker wire ops themselves; reuse only the QMP command
  payload shapes as internal DTOs)

**Destination:** `src/qmp/`

**Tests:**
- `tests/qmp_capability_negotiation.rs`
- `tests/qmp_command_dispatch.rs` — all commands in §13.2 table
- `tests/qmp_greeting_timeout.rs` — timeout → `qmp-greeting-timeout` error
- `tests/qmp_health_check.rs` — query-status; Degraded on failure

---

### WI-010 Hotplug attach/detach protocol

**Priority:** P2

**Description:** On `removableVolumeRefs` update, request Volume fd from
`volume-local` ComponentSession and issue `blockdev-add`/`device_add` QMP
commands (§13.3). Reverse for detach.

**Source (ADAPT):**
- `packages/d2b-contracts/src/broker_wire.rs` `QemuMediaAttach`,
  `QemuMediaDetach` — extract QMP command bodies; delete broker op wiring

**Destination:** `src/controller/hotplug.rs`

**Tests:**
- `tests/hotplug_attach_sequence.rs`
- `tests/hotplug_detach_sequence.rs`
- `tests/hotplug_qmp_failure.rs` — QMP error → Degraded + `hotplug-media-failed`

---

### WI-011 Network tap fd acquisition

**Priority:** P1

**Description:** Call `network-local` ComponentSession service to request tap
fd for a Guest MAC/bridge assignment. Include fd in LaunchTicket. No bridge
name or interface name in any public field.

**Destination:** `src/controller/network.rs`

**Tests:**
- `tests/tap_fd_acquisition.rs` — fake network-local service; fd delivery
- `tests/tap_fd_unavailable.rs` → `network-tap-unavailable` Degraded

---

### WI-012 Reconcile loop and finalize

**Priority:** P1

**Description:** Full async reconcile loop (§11.3) and finalize sequence
(§11.4). Dependency gating, providerPhase transitions, condition management.

**Destination:** `src/controller/reconcile.rs`

**Tests:**
- `tests/reconcile_dependency_gating.rs`
- `tests/reconcile_runner_exit_handling.rs`
- `tests/finalize_sequence.rs`
- `tests/finalize_wayland_session_cleanup.rs`

---

### WI-013 Status, conditions, and error reporting

**Priority:** P1

**Description:** All phase transitions (§16.1), providerPhase values (§16.2),
condition types (§16.3), and error codes (§16.4). Bounds enforcement on
`providerPhase` string.

**Destination:** `src/controller/status.rs`

**Tests:** `tests/status_phase_transitions.rs`, `tests/condition_reason_codes.rs`

---

### WI-014 Audit event emission

**Priority:** P2

**Description:** Emit all audit events in §17. Verify no sensitive fields
(paths, argv, fds, socket paths) in any payload.

**Destination:** `src/audit.rs`

**Tests:**
- `tests/audit_event_shapes.rs` — golden shapes for each event kind
- `tests/audit_no_sensitive_fields.rs` — property test: no path/argv/fd in payload

---

### WI-015 Metrics and OTEL spans

**Priority:** P2

**Description:** Implement all metrics (§18) and OTEL trace spans. Label
cardinality enforcement; no VM name, user identity, or path in any label.

**Destination:** `src/telemetry.rs`

**Tests:**
- `tests/metrics_label_cardinality.rs`
- `tests/otel_span_attributes.rs` — no sensitive attribute

---

### WI-016 Nix module and assertions

**Priority:** P1

**Description:** Nix module for Guest resource declaration (§19). Eval-time
assertions (§19.8) in `nixos-modules/assertions.nix`.

**Source (ADAPT):**
- `nixos-modules/components/qemu-media.nix` — extract option names; rewrite
  as v3 spec fields; remove raw path options
- `nixos-modules/assertions.nix` — add new assertion predicates

**Destination:**
- `nixos-modules/options-guest-qemu-media.nix` (new)
- `nixos-modules/assertions.nix` (extend)

**Tests:**
- `tests/unit/nix/cases/guest-qemu-media-spec.nix` (nix-unit eval case)
- `tests/assertions-eval.sh` — new assertion cases

---

### WI-017 d2b-provider-toolkit conformance

**Priority:** P2

**Description:** Pass the Provider conformance kit for the `Guest` ResourceType
axis: reconcile/finalize contract, phase machine, condition typing,
audit shape, telemetry cardinality.

**Destination:** `tests/conformance_guest.rs`

**Tests:** `make test-rust` (runs conformance suite)

---

### WI-018 Integration tests

**Priority:** P2

**Description:** Integration scenarios with container/fake-Host fixtures:
full reconcile from Created to Ready (fake dependencies), finalize sequence,
hotplug attach/detach, restart recovery.

**Destination:** `integration/`

**Tests:** `make test-integration`

---

## 22 Tests

### 22.1 Hermetic unit tests (`tests/`)

| Test file | Coverage |
| --- | --- |
| `guest_schema_roundtrip.rs` | GuestSpec/GuestStatus JSON Schema generation, serde, unknown-field denial |
| `guest_provider_settings_bounds.rs` | All field bounds in §5 |
| `config_schema_projection.rs` | ProviderConfig schema; controllerExecutionRef required |
| `state_volume_spec.rs` | Component descriptor stateNamespace: migrationPolicy none, quota.maxBytes/maxInodes + quotaBytes nonzero, source.settings.sourcePolicyId, layout ownerRef/mode, view rights, identityMarker class |
| `state_volume_principal.rs` | Layout principal is `User/runtime-qemu-media-system`; no ComponentPrincipal ref; no cross-component shared Volume |
| `state_volume_mount_exclusivity.rs` | Worker Process spec contains no mount to the controller state Volume |
| `state_volume_no_controller_ops.rs` | Controller reconcile issues no Volume create/watch/update/deletionRequestedAt; Volume absent from controller ResourceClient permissions |
| `runtime_volume_spec.rs` | Runtime tmpfs Volume spec golden; all layout entries |
| `volume_cleanup_policy.rs` | `cleanupPolicy: vm-stop-with-proof` correctness |
| `media_volume_watch.rs` | Dependency gating for boot/removable Volume refs |
| `media_attachment_validation.rs` | Missing virtio-blk attachment → condition error |
| `kvm_device_watch.rs` | Device/host-kvm phase machine; Degraded/Failed propagation |
| `wayland_session_create.rs` | `display-wayland.d2b.io.WaylandSession` resource type; no invented spec fields; no managedBy |
| `wayland_session_attachment_read.rs` | Opaque endpoint attachment consumed from display-wayland status |
| `wayland_session_missing_provider.rs` | displayWindow=true + null displayProviderRef → Failed |
| `process_spec_golden.rs` | Full canonical Process spec against §10.1 YAML |
| `launch_ticket_fd_slots.rs` | LaunchTicket fd table completeness |
| `no_raw_argv_in_spec.rs` | No executable path in any Process spec field |
| `qmp_capability_negotiation.rs` | QMP greeting exchange |
| `qmp_command_dispatch.rs` | All QMP commands in §13.2; success and error paths |
| `qmp_greeting_timeout.rs` | Timeout → qmp-greeting-timeout error code |
| `qmp_health_check.rs` | query-status; Degraded on consecutive failures |
| `hotplug_attach_sequence.rs` | blockdev-add + device_add via QMP attachment |
| `hotplug_detach_sequence.rs` | device_del + blockdev-del via QMP attachment |
| `hotplug_qmp_failure.rs` | QMP error → Degraded + `hotplug-media-failed` |
| `tap_fd_acquisition.rs` | network-local ComponentSession; fd delivery |
| `tap_fd_unavailable.rs` | `network-tap-unavailable` Degraded |
| `reconcile_dependency_gating.rs` | All dependencies missing/present combinations |
| `reconcile_runner_exit_handling.rs` | Runner exit → Failed / re-create logic |
| `finalize_sequence.rs` | Finalizer drain order (runner → WaylandSession → Volume) |
| `finalize_wayland_session_cleanup.rs` | WaylandSession deleted before Volume |
| `status_phase_transitions.rs` | All phase and providerPhase transitions |
| `condition_reason_codes.rs` | All reason codes in §16.3 |
| `audit_event_shapes.rs` | Golden shape for every event in §17 |
| `audit_no_sensitive_fields.rs` | Property test: no path/argv/fd/socket-path in payload |
| `metrics_label_cardinality.rs` | All metric label values; no VM name or path |
| `otel_span_attributes.rs` | No sensitive attribute in any span |
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
in WI-016) explains the option mapping for operator configurations.

### 23.4 `docs/adr/0036-qemu-media-runtime.md` supersession

ADR 0036 is marked superseded by a `status: Superseded` header edit in
the same commit that lands the first production-ready release of
`Provider/runtime-qemu-media`. The removal gate (`tests/static.sh` drift
check) verifies the superseded field is present.

### 23.5 CHANGELOG entry

The CHANGELOG.md `## [Unreleased]` block receives an `Added` entry at the
WI-001 commit and a `Changed` entry noting the removal of `QemuMedia*` broker
ops at the removal-gate commit. No process markers appear in the CHANGELOG
text.
