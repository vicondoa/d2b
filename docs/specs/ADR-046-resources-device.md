# ADR 0046 Device resource and Provider dossiers

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resources-device` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-device-*`, Device controller contracts, Nix device emitters |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox` |
| Supersedes | Current ProcessRole device sidecars (Swtpm, SwtpmPreStartFlush, Usbip, SecurityKeyFrontend, Gpu, GpuRenderNode, Video) and their Nix components |

## Purpose

Device is the single ResourceType for physical and emulated hardware device
inventory, arbitration, claim, and attachment in a Zone. It is the resource
contract for the four frozen Device Provider families: `device-tpm`,
`device-usbip`, `device-security-key`, and `device-gpu` (combined GPU/video).

Audio (`device-audio`) is not part of this spec. The `audio-pipewire` Provider
is in the interaction Provider catalog and is independently specified.

## Device ResourceType spec

### Envelope example

```yaml
apiVersion: resources.d2b.io/v3
type: Device
metadata:
  name: corp-vm-tpm
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: Guest/corp-vm
  finalizers: [device-tpm.d2b.io/state-preserved]
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:00:00Z
spec:
  providerRef: Provider/device-tpm
  deviceClass: emulated
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector: {}             # emulated devices carry no physical selector
  settings: {}               # Provider-specific settings
status:
  observedGeneration: 1
  phase: Ready
  conditions: []
  lastReconciledAt: 2026-07-22T00:00:01Z
  device:
    present: true
    health: healthy
    holderRefs: [Guest/corp-vm]
    claims: []
    provisionedAt: 2026-07-22T00:00:01Z
```

### Device spec fields

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | — | must resolve to an installed Provider | Selects the device Provider |
| `deviceClass` | enum | yes | — | `physical` \| `emulated` | Physical devices exist in sysfs/udev; emulated devices are created by the Provider |
| `arbitration` | enum | yes | — | `exclusive` \| `shared` | Whether the device may be simultaneously claimed by more than one holder |
| `maxConcurrentClaims` | uint | no | 1 | 1–16 | Maximum simultaneous claimants; must equal 1 when `arbitration=exclusive` |
| `inventory` | object | yes | — | see below | Physical or emulated device selector |
| `settings` | object | no | `{}` | Provider-specific schema | Provider-validated device configuration |

### Inventory selector

An inventory selector is a discriminated union keyed on `busClass`. The
`busClass` field is the variant discriminant. Unknown fields for a given
`busClass` variant and unknown `busClass` values are rejected at admission
(strict unknown-field denial). No raw device path appears in the spec.

```yaml
inventory:
  # Emulated device — selector must be absent or {}
  selector: {}

  # USB device
  selector:
    busClass: usb
    label: yubikey-work       # required; stable human label, max 63 chars
    vendorId: "1050"          # 4-hex lower-cased; optional
    productId: "0407"         # 4-hex lower-cased; optional
    serial: null              # max 128 chars; optional

  # HID/hidraw device
  selector:
    busClass: hidraw
    label: yubikey-primary
    vendorId: "1050"
    productId: "0407"
    serial: null

  # DRM/GPU device
  selector:
    busClass: drm
    label: host-gpu
    pciSlot: null             # optional PCI slot filter; e.g. "0000:01:00.0"; max 31 chars

  # PCI device (non-GPU; reserved for future physical-passthrough Providers)
  selector:
    busClass: pci
    label: host-pci-dev
    slot: null                # PCI slot; e.g. "0000:02:00.0"; max 31 chars

  # Physical TPM kernel device (rare; emulated TPM uses {}))
  selector:
    busClass: tpm
    label: host-tpm
    index: 0                  # /dev/tpm<index>; default 0
```

Closed `busClass` values: `usb`, `hidraw`, `drm`, `pci`, `tpm`. Any other value
is rejected. For `deviceClass=emulated` the `selector` must be `{}` or absent.
For `deviceClass=physical` the `selector.label` field is required on every
variant and serves as the stable operator-defined identifier; the Provider
resolves the physical node using the label plus any provided filter fields.
Vendor and product IDs are stored lower-cased and must be exactly four ASCII hex
digits. A spec containing extra fields beyond a variant's declared field set is
rejected without admission.

## Claims and attachments in Host/Guest/Process

Device claims are declared inline on the execution context that holds them.

### Host/Guest devices list

`spec.devices` on a Host or Guest resource:

```yaml
devices:
  - deviceRef: Device/corp-vm-tpm
    claim: exclusive
    passthrough: tpm-socket   # Provider-specific passthrough kind
```

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `deviceRef` | ResourceRef | yes | — | Same-Zone Device resource |
| `claim` | enum | yes | — | `exclusive` \| `read-shared` \| `provider-managed` |
| `passthrough` | string | no | null | Provider-specific passthrough kind (e.g., `tpm-socket`, `usbip-export`, `virtiofs-hidraw`, `gpu-virtio`, `gpu-render-node`) |
| `settings` | object | no | `{}` | Provider-specific per-attachment settings |

A Device with `arbitration=exclusive` rejects more than one claimed Host/Guest
attachment at a time. The controller writes a conflict condition and sets phase
Degraded/Pending on all but the first successful claimant.

### Process devices list

`spec.devices` on a Process or EphemeralProcess resource:

```yaml
devices:
  - deviceRef: Device/corp-vm-gpu
    usage: gpu-socket
    settings: {}
```

Process device entries express a usage dependency. The Process controller
verifies the Device is Ready and claimed by its owning Guest/Host before launch.
A Process that needs a device fd directly (e.g., a GPU worker Process that
receives a socket path) declares its dependency here; the fd handoff is
Provider-specific and is not in the resource spec.

## Device status

### Three-layer status shape (D088)

D088 freezes `Device` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Device`-specific status fields documented in this
section constitute the ResourceType-common `status.resource` (Layer 2) object
and never restate the universal base. Optional implementation-only observation
belongs in `status.provider` (Layer 3) with exactly `providerRef`, qualified
immutable `schemaId`, semver `MAJOR.MINOR` `schemaVersion`, numeric
`observedProviderGeneration`, and strict, bounded, redacted,
unknown-field-denied `details`. Generic API, CLI, and controllers MUST consume
only the base-only projection (`status` base plus `status.resource`).
Controllers MUST write all present layers atomically in one status mutation with
one expected revision. D087/D088 mapping is: shared observations go to
`status.resource`; implementation-specific bounded non-secret observations go to
`status.provider.details`; secret, large, or private observations go to an
optional Volume. D088 bounds apply: total status <= 64 KiB, `status.resource` <=
32 KiB, `status.provider.details` <= 32 KiB, 32 conditions, 64-entry lists/maps,
and 4 KiB strings; violations use `status-oversize`,
`status-provider-schema-invalid`, or `status-provider-overlap`.

Mapping convention: within this spec a reference to `status.<field>` denotes the ResourceType-common `status.resource.<field>` unless `<field>` is a universal base field (`observedGeneration`, `phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, `outcome`).

The existing `status.device` sub-object is carried within `status.resource` as
`status.resource.device` by the mapping convention. `Device` has multiple
implementations (`device-tpm`, `device-usbip`, `device-gpu`, and
`device-security-key`). Claim/arbitration base fields, presence/provisioning,
health, holder refs, and bounded claim entries are frozen in `status.resource`
and MUST be identical across all implementations. Implementation-specific
observation belongs only in that implementation's `status.provider.details`;
shared fields MUST NOT be duplicated there.

Status fields beyond the standard common envelope:

```yaml
status:
  phase: Ready | Pending | Degraded | Failed | Unknown
  conditions:
    - type: DevicePresent
      status: "True"
      reason: device-probed-present
    - type: DeviceClaimed
      status: "True"
      reason: exclusive-claim-held
  device:
    present: true | false | null
    health: healthy | degraded | failed | unknown
    holderRefs: []           # ordered list of claimant ResourceRefs (bounded)
    claims: []               # per-claimant claim entry
    provisionedAt: null      # RFC 3339; set when emulated device is created
    lastProbedAt: null       # RFC 3339; set on last physical probe
    providerDiagnostic: null # bounded redacted one-line string; never paths/secrets
```

### Device claim entry

```yaml
claims:
  - holderRef: Guest/corp-vm
    claim: exclusive
    passthrough: tpm-socket
    claimedAt: 2026-07-22T00:00:01Z
    health: healthy | degraded | failed | unknown
```

### Phase semantics

| Phase | Meaning |
| --- | --- |
| `Pending` | Device spec committed but not yet provisioned/claimed |
| `Ready` | Device is present/provisioned, claim(s) held, healthy |
| `Degraded` | Usable but one or more conditions impaired (e.g., probe uncertain) |
| `Failed` | Current spec generation cannot complete under retry policy |
| `Unknown` | Controller or Host/Guest cannot currently prove device state |

`Succeeded` and `Deleted` follow the common phase contract.

### Condition types

| Type | Meaning |
| --- | --- |
| `DevicePresent` | Physical device is sysfs/udev-visible; always True for emulated; Unknown after first probe failure; False after three consecutive failures |
| `DeviceProvisioned` | Emulated device (swtpm, virtual HID) has been created |
| `DeviceClaimed` | At least one active claim is held |
| `DeviceHealthy` | Physical or emulated device is responsive |
| `ClaimConflict` | Exclusive device received a second concurrent claim request |
| `StateIntegrity` | Emulated device tamper-marker/state check passed |
| `BrokerAccessible` | Broker can open/pass the physical device fd |

## Ownership and finalizer contract

The Device resource's `ownerRef` is typically the Guest or Host that provisioned
it. The owning Provider installs one or more typed finalizers before starting any
external effect.

### Finalizer IDs

| Provider | Finalizer ID |
| --- | --- |
| `device-tpm` | `device-tpm.d2b.io/state-preserved` |
| `device-usbip` | `device-usbip.d2b.io/attachment-released` |
| `device-security-key` | `device-security-key.d2b.io/lease-released` |
| `device-gpu` | `device-gpu.d2b.io/worker-stopped` |

Deletion sequence:
1. `deletionRequestedAt` is set.
2. The Device Provider finalizer handler terminates owned worker Processes,
   releases OS resources (socket, udev rules, swtpm state if policy allows),
   and clears the finalizer.
3. If a tamper-sensitive Volume exists (device-tpm only), the finalizer
   completes but does NOT delete the persistent Volume; that Volume has its
   own lifecycle and owner.
4. Core removes the Device resource after all finalizers clear.

Persistent TPM state (swtpm NVRAM, EK seed) is never deleted by the Device
finalizer. The Volume resource that owns TPM state has an independent lifecycle
and its own `volume-local.d2b.io/tpm-state` finalizer.

## Hotplug

### Hotplug observation

Device Providers that manage physical devices declare a bounded observe interval
in their controller descriptor. Core schedules a `scheduled-observe` trigger at
that interval. The controller probes the current device state and writes status.

Hotplug state changes flow exclusively through Device status and the standard
resource/owner/dependency hint mechanism. The Device controller does not signal
the Host/Guest supervisor directly; it writes updated Device status, and
dependent Host/Guest controllers receive a `dependency-changed` trigger through
the normal resource watch path and react accordingly. There is no direct
supervisor signal bypass.

### Physical probe failure semantics

| Consecutive probe failures | Status transition |
| --- | --- |
| 1 (first failure) | phase → `Unknown`; condition `DevicePresent` status=`Unknown`, reason=`device-probe-failed` |
| 2 | condition `DevicePresent` remains `Unknown`; phase remains `Unknown` |
| 3 | phase → `Degraded`; condition `DevicePresent` status=`False`, reason=`device-consecutive-probe-failures-exceeded` |
| Device returns | phase → `Ready`; condition `DevicePresent` status=`True`, reason=`device-probed-present` |

A single probe failure does not set `DevicePresent=False` or phase `Degraded`.
After three consecutive failures the full consequence chain fires:

1. Condition `DevicePresent=False` and phase `Degraded`.
2. Claimant Host/Guest/Process controllers receive `dependency-changed` triggers
   through the normal resource watch path.
3. If the device is essential (e.g., an exclusive TPM claim), the Guest
   controller may set phase `Degraded` or stop the Guest.
4. When the device returns, the Device controller sets `DevicePresent=True` and
   phase `Ready`, re-triggering claimants.

### Hotplug limits

- Maximum observe interval: 60 s (configurable per Provider, default 30 s).
- A probe failure must transition phase to `Unknown` within one observe period.
- Three consecutive probe failures are required before `DevicePresent=False`.

## Security model

### Broker effect limits

The Device Provider is a Process under a Host. It interacts with the Zone
privileged broker for operations that require root:

| Provider | Broker operation | Effect | Audit |
| --- | --- | --- | --- |
| `device-tpm` | `PrepareStateDir` (via `PrepareRuntimeDir`/`PrepareSwtpmDir` broker hook) | Provision/harden swtpm state dir, verify tamper marker | Yes |
| `device-tpm` | `SpawnRunner` (swtpm role) | Spawn swtpm Process in user namespace | Yes |
| `device-usbip` | `UsbipBindFirewallRule` | Add per-env/bus nftables rule | Yes |
| `device-usbip` | `SpawnRunner` (usbip role) | Spawn usbipd/bind Process | Yes |
| `device-security-key` | `SecurityKeyOpenDevice` | Open exact FIDO hidraw node; return fd via SCM_RIGHTS; never a path | Yes |
| `device-security-key` | `SecurityKeyApplyUdevRules` | Write udev rules for configured FIDO hidraw nodes | Yes |
| `device-gpu` | `SpawnRunner` (gpu/gpu-render-node/video role) | Spawn crosvm GPU or video-decoder Process | Yes |
| `device-gpu` | `OpenDevice` (kvm/dri/udmabuf/nvidia*) | Open GPU device fds in user namespace pre-spawn | Yes |

No Device Provider receives a blanket device-path grant, raw socket address, or
ambient host capability. Broker operations are point-specific and audit-logged.

### Security key specific invariants

From D046 and the v3 baseline:

1. The Zone privileged broker is the only entity that opens the physical hidraw
   node. It never accepts a caller-supplied path; it derives the physical node
   from the trusted bundle device table using only a stable label or session ID.
2. **Current implementation:** the relay is a daemon-internal async accept loop
   inside d2bd (`packages/d2bd/src/lib.rs:start_sk_accept_loop`,
   `packages/d2bd/src/security_key.rs`): the daemon calls the broker for the
   hidraw fd via SCM_RIGHTS, binds a vsock-proxy Unix socket, and spawns an
   async accept loop — there is no separate relay process. **v3 target:** this
   relay logic is extracted into a dedicated unprivileged relay Process under
   the device-security-key Provider. The relay Process receives the hidraw fd
   from the broker over SCM_RIGHTS and proxies CTAP HID traffic to the Guest
   frontend over AF_VSOCK. It never runs as root and has no further broker access.
3. **Current implementation:** the guest-side UHID virtual HID binary is
   `packages/d2b-sk-frontend/src/` (static, implemented-and-reachable), running
   inside the Guest VM as a guest systemd service (`d2b-sk-frontend.service`)
   declared in `nixos-modules/components/security-key-guest.nix`. It is NOT
   a current ProcessRole or Zone Process — the ProcessRole name
   `SecurityKeyFrontend` refers to the HOST accept loop, not this guest binary.
   **v3 target:** the `d2b-sk-frontend` binary becomes a Zone Process resource
   (name `device-<uid-short>-sk-frontend`, `executionRef: Guest/<vm>`) owned by
   the device-security-key Provider controller, giving the controller lifecycle
   visibility and replacing the untracked guest systemd unit.
4. At most one Guest may hold the exclusive hidraw lease at a time. A second
   claim is rejected; the requesting Guest receives `ClaimConflict`.
5. Security-key proxy and USBIP YubiKey passthrough are mutually exclusive for
   the same VM. This invariant is enforced at Nix eval time and at runtime by
   the Device controller.

### TPM state integrity

The swtpm state directory (`<stateDir>/vms/<vm>/swtpm`) is identity-bound:

- Mode 0700, owner `d2b-<vm>-swtpm`.
- Per-VM sticky 3770 root prevents non-owner rename/replace.
- Root-owned tamper-guard marker at `/var/lib/d2b/swtpm-markers/<vm>` records
  `st_dev`/`st_ino` + first-provision stamp.
- A missing or mismatched marker fails VM start closed
  (`previously-provisioned-swtpm-state-missing`).
- Wiping state is treated as device tampering by IdPs (Entra ID, Intune);
  the Device finalizer never deletes swtpm NVRAM.

These invariants are preserved exactly in the device-tpm Provider.

### GPU security

- Full GPU (virtio-gpu, card-node access, VFIO passthrough): always
  `arbitration: exclusive`. Only one Guest may hold a full GPU claim at a time.
- Render-node-only mode (`settings.renderNodeOnly=true`) may use
  `arbitration: shared` when the Device spec explicitly sets
  `arbitration: shared`. A Device spec with `arbitration: shared` and
  `settings.renderNodeOnly=false` is rejected at admission. The default
  arbitration for GPU devices is `exclusive` regardless of mode; operators must
  explicitly opt into shared render-node mode.
- Gpu and GpuRenderNode claim `kvm`, `dri`, `nvidia-ctl`, `nvidia-uvm`,
  `nvidia-render`, and `udmabuf` broker device tokens. VFIO/SR-IOV is not
  included in the standard GPU device claim.
- The GPU worker Process runs in a user namespace (ADR 0021 broker-pre-NS).
  The broker opens GPU device fds before clone and passes them; the Process
  has zero ambient host capabilities.
- Render-node-only mode passes only the DRM render node fd without a virtio-gpu
  bind-mount.
- Video decode (crosvm video-decoder) runs as the per-Device Process principal,
  needs `/dev/dri` and optionally NVIDIA devices. It is a separate Process from
  the GPU sidecar but shares the GPU Device claim.

### USBIP security

- USBIP per-env/per-busid firewall rules use the ownership-marker pattern:
  `comment "d2b managed: usbip:env:<env>:bus:<bus_id>"`.
- Broker operation `UsbipBindFirewallRule` is audit-logged and destructive.
- The bus ID validation is governed by `packages/d2b-contracts/src/usbip.rs`:
  max 31 chars (`SYSFS_BUS_ID_MAX`), ASCII digits and separators only, no
  shell metacharacters, no leading zeros on segments.
- Explicit-attach (`d2b usb attach <vm> <busid> --apply`) uses a separate
  `UsbipExplicitBind` broker op path distinct from bundle-declared claims.

## Async reconciliation

Each Device Provider controller implements the standard async reconcile loop
from `ADR-046-resource-reconciliation`:

1. **spec-generation-changed:** create/update inventory selector, provision
   emulated device (swtpm, UHID virtual HID), apply udev rules.
2. **deletion-requested:** stop owned worker Processes, release OS resources,
   uninstall udev rules, clear finalizer. Never delete persistent TPM state.
3. **dependency-changed / execution-status-changed:** if the owning Guest
   stops, release active claims and set phase Degraded.
4. **scheduled-observe:** probe physical device presence; update status.
5. **owned-resource-changed:** if a worker Process fails, set DeviceHealthy=False
   and optionally restart.

### Fast-path process launch

Worker Processes (swtpm, crosvm GPU/video, usbipd) follow the standard
Process fast-path (commit-to-handler ≤5 ms p95, launch-attempt ≤20 ms p95).
The Device controller creates the Process resource; the Process controller
manages the launch.

### Pre-start EphemeralProcess pattern

`SwtpmPreStartFlush` maps to an EphemeralProcess owned by the device-tpm
controller. The controller creates it before creating the long-lived swtpm
Process, waits for `Succeeded`, then creates the swtpm Process:

```text
Device/corp-vm-tpm
  └─ EphemeralProcess/device-<uid-short>-flush  (pre-start flush; successfulTtl=15m)
  └─ Process/device-<uid-short>-swtpm           (long-lived swtpm socket)
  └─ Volume/corp-vm-tpm-state                   (persistent TPM NVRAM; separate owner)
```

If the flush EphemeralProcess fails, the Device controller sets phase Failed
and does not launch the swtpm Process.

## Provider: device-tpm

### Identity

```text
Provider/device-tpm
```

Crate: `packages/d2b-provider-device-tpm/`
Dossier: `docs/specs/ADR-046-provider-device-tpm.md` (separate, not yet authored)

### Implements

Device (emulated, exclusive, TPM class).

### Root config schema

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `logLevel` | uint | 20 | 1–20 | swtpm `--log level` value |
| `startupClear` | bool | true | — | Emit `--flags startup-clear`; requires pre-start flush |
| `stateDirPath` | null | provider-derived | — | Overrides are rejected; path is always derived from Volume/state owner |

swtpm and swtpm_ioctl binaries are dependencies bundled inside the
`d2b-provider-device-tpm` package closure. Their executable paths are resolved
from the signed component descriptor inside that closure; they are not
configurable fields in the Device spec.

Config must not carry a raw state directory path; the Provider derives it from
the owned Volume resource.

### Device spec

```yaml
spec:
  providerRef: Provider/device-tpm
  deviceClass: emulated
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector: {}
  settings:
    logLevel: 20
    startupClear: true
```

### Worker processes

| Process name | Role | Executable | Domain | Execution | Notes |
| --- | --- | --- | --- | --- | --- |
| `device-<uid-short>-flush` | EphemeralProcess | `swtpm_ioctl -i --unix <ctrl.sock>` | system | Host | Pre-start flush; once per start cycle |
| `device-<uid-short>-swtpm` | Process | `swtpm socket ...` | system | Host | Long-lived; supervised |

Process resource names are derived deterministically from the owner Device
resource UID (`uid-short` = first 12 hex chars of the Device UID) combined with
a component template from the signed Provider dossier. The name never contains a
VM or Guest human name. The Device UID is stable across restarts. Foundation
canonical process identity format applies.

### State/Volume

The swtpm NVRAM Volume is a separate `volume-local` resource with independent
ownership:

```yaml
type: Volume
metadata:
  name: corp-vm-tpm-state
spec:
  providerRef: Provider/volume-local
  layout:
    - path: swtpm
      type: directory
      ownerRef: User/corp-vm-swtpm-system
      mode: "0700"
      noFollow: true
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: retain-forever
  views:
    swtpm-process:
      path: swtpm
      rights: [read, write, create]
```

The Device resource owns the swtpm Process via ownerRef. The Volume is the
TPM persistent state root; the Device does NOT own the Volume to prevent
accidental deletion cascade.

### Broker operations consumed

- `PrepareSwtpmDir` (hardening/tamper-marker) — once per start cycle.
- `SpawnRunner` (swtpm role) — for each long-lived swtpm Process.

### Nix options (v3 successors)

Current: `d2b.vms.<vm>.tpm.enable = true` → guest `nixos-modules/components/tpm.nix`

v3 successor:
```nix
d2b.zones.<zone>.resources."<vm>-tpm" = {
  type = "Device";
  metadata.ownerRef = "Guest/<vm>";
  spec = {
    providerRef  = "Provider/device-tpm";
    deviceClass  = "emulated";
    arbitration  = "exclusive";
    settings.startupClear = true;
  };
};
```

The swtpm and swtpm_ioctl binaries are dependencies inside the
`d2b-provider-device-tpm` package closure (referenced by the Provider resource's
`spec.artifactId`); they require no configuration in the Device spec.

The guest NixOS module wiring (`--tpm socket=...`, `tpm_crb` kernel module,
`tpm2-tools`, session-flush service) remains in the Guest Provider's
`runtime-cloud-hypervisor` Nix module.

### Tests

- `packages/d2b-contract-tests/tests/minijail_swtpm_video.rs`: swtpm profile
  shape, user namespace propagation, zero host caps.
- `packages/d2b-contract-tests/tests/policy_swtpm_readiness.rs`: readiness contract.
- `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` unit tests: tamper marker,
  fresh/existing dir, symlink/mismatch fail-closed.
- `packages/d2b-host/src/swtpm_argv.rs` unit tests: argv golden vectors.
- New: Device reconcile state-machine test (flush → swtpm → Ready).
- New: Tamper-marker failure closes VM start (does not recreate empty TPM).
- New: Delete finalizer does not remove Volume/state.

## Provider: device-usbip

### Identity

```text
Provider/device-usbip
```

Crate: `packages/d2b-provider-device-usbip/`
Dossier: `docs/specs/ADR-046-provider-device-usbip.md` (separate, not yet authored)

### Implements

Device (physical, exclusive per Guest attachment).

### Root config schema

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `usbipHostKernelModule` | string | `usbip-host` | fixed | Host kernel module |
| `vhciHcdKernelModule` | string | `vhci_hcd` | fixed | Guest kernel module |
| `backendPort` | uint16 | provider-derived | 1–65535 | Per-env deterministic USBIP backend port |

The usbip host and guest binaries are dependencies bundled inside the
`d2b-provider-device-usbip` package closure. Their executable paths are resolved
from the signed component descriptor inside that closure; they are not
configurable fields in the Device spec.

### Device spec

```yaml
spec:
  providerRef: Provider/device-usbip
  deviceClass: physical
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector:
      busClass: usb
      label: yubikey-work
      vendorId: "1050"
      productId: "0407"
      serial: null
  settings:
    env: work
```

### Worker processes

| Process name | Role | Executable | Domain | Execution | Notes |
| --- | --- | --- | --- | --- | --- |
| `device-<uid-short>-bind` | EphemeralProcess | `usbip bind --busid <id>` | system | Host | Bind bus ID at attach |
| `device-<uid-short>-unbind` | EphemeralProcess | `usbip unbind --busid <id>` | system | Host | Release bus ID at detach |
| `device-<uid-short>-daemon` | Process | `usbipd --tcp-port <port>` | system | Host | Long-lived per-Device USBIP backend |

The per-Device usbipd daemon and proxy Processes are owned by
`Provider/device-usbip`. The Network provider supplies only the
dependency/status/firewall interface: bridge membership, port allocation, and
the `UsbipBindFirewallRule` broker op surface. Network does not own or supervise
USBIP Processes.

Process resource names follow the `device-<uid-short>-<component>` template
(`uid-short` = first 12 hex chars of the Device UID; component from the signed
Provider dossier). VM or Guest human names never appear in Process resource
names.

### Bring-up sequence (from usbip_state_machine.rs)

The canonical per-busid USBIP bring-up order is captured in
`packages/d2bd/src/usbip_state_machine.rs` (implemented-and-reachable):

```
modprobe → lock → withhold → firewall → backend → bind → proxy
```

This sequence is preserved in the device-usbip Provider reconcile loop. Each
step maps to a broker operation or host-side action:

| Step | Action | Broker op |
| --- | --- | --- |
| `modprobe` | Load `usbip-host` kernel module | EphemeralProcess (modprobe) |
| `lock` | Acquire per-busid OFD lock | — |
| `withhold` | Prevent OS auto-claim of device | sysfs write via broker |
| `firewall` | Add nftables rule | `UsbipBindFirewallRule` |
| `backend` | Start per-env usbipd daemon | `SpawnRunner` (usbip role) |
| `bind` | Bind bus ID to usbip-host | EphemeralProcess (usbip bind) |
| `proxy` | Start TCP proxy on env-host IP | `SpawnRunner` (usbip proxy role) |

### Broker operations consumed

- `UsbipBindFirewallRule`: add nftables rule per env/bus; audited/destructive.
- `SpawnRunner` (usbip backend role, `usbip-host` device token): spawn per-env usbipd.
- `SpawnRunner` (usbip proxy role): spawn TCP proxy.

### Validation

Bus ID validation follows `packages/d2b-contracts/src/usbip.rs`:
- Max `SYSFS_BUS_ID_MAX` = 31 chars.
- Accepted form: `B`, `B-P`, `B-P.S[.S...]`.
- ASCII digits only. No leading zeros on segments. No metacharacters.

Vendor/product ID: exactly 4 ASCII hex digits, lower-cased at storage.

### Guest-side config (Nix)

Current: `d2b.vms.<vm>.usbip.yubikey = true` → `nixos-modules/components/usbip.nix`

v3 guest module wires:
- `vhci_hcd` kernel module.
- `usbip` CLI tools.
- `d2b.guestControl.usbipPath` for guest-side import.

The guest-side kernel module and tools remain in the Guest's `runtime-cloud-hypervisor`
Nix module under v3.

### Tests

- `packages/d2b-contract-tests/tests/usbip_json_contract.rs`: USBIP DTO
  serde/schema round-trips, unknown field denial.
- `packages/d2b-contract-tests/tests/usb_network_scoping.rs`: per-env isolation.
- New: Device arbitration conflict test (two Guests claim same bus ID).
- New: Firewall rule ownership-marker preservation.
- New: Bus ID validation corpus (31-char max, metachar rejection, leading-zero segments).
- New: Explicit-attach (EphemeralProcess bind) vs declared (Process bind) path split.

## Provider: device-security-key

### Identity

```text
Provider/device-security-key
```

Crate: `packages/d2b-provider-device-security-key/`
Dossier: `docs/specs/ADR-046-provider-device-security-key.md` (separate, not yet authored)

### Implements

Device (physical, exclusive per-session lease).

### Key invariants (from D046)

- The Provider owns unprivileged Host relay Processes and Guest frontend
  Processes plus ceremony/CID/lease.
- The Zone privileged broker only opens/passes the hidraw fd; no path crosses
  the public surface.

### Root config schema

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `devices` | list | [] | max 16 | Per-device selector entries |
| `devices[].label` | string | — | `^[a-z][a-z0-9-]{0,62}$` | Stable selector label |
| `devices[].vendorId` | uint16 | — | — | USB vendor ID |
| `devices[].productId` | uint16 | — | — | USB product ID |
| `devices[].serial` | string \| null | null | max 128 chars | Optional serial filter |
| `vsockPort` | uint16 | 14320 | 1–65535 | AF_VSOCK port for host↔guest relay |
| `sessionRingSize` | uint | 32 | 8–256 | Bounded recent-session ring size |
| `leaseTimeoutSecs` | uint | 300 | 30–3600 | Session-level lease timeout |

The `vsockPort` default 14320 is stable and matches `security-key-guest.nix`
option `d2b.securityKey.vsockPort` default.

### Device spec

```yaml
spec:
  providerRef: Provider/device-security-key
  deviceClass: physical
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector:
      busClass: hidraw
      label: yubikey-primary
      vendorId: "1050"
      productId: "0407"
      serial: null
  settings:
    vsockPort: 14320
```

### Process model

The security-key Provider owns two process classes:

**Host relay Process** (one per Device resource):

```yaml
type: Process
metadata:
  name: device-<uid-short>-sk-relay
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: worker
  # Package/template from device-security-key Provider dossier
```

The relay process:
- Runs unprivileged; receives the hidraw fd from the broker via SCM_RIGHTS.
- Proxies raw CTAP HID packets to the Guest frontend over AF_VSOCK.
- Has no further broker access.
- Restarts on disconnect; clears its lease on clean exit.

**Current implementation note:** In the v3 baseline the relay is NOT a separate
spawned process — it is a daemon-internal async accept loop in d2bd
(`packages/d2bd/src/lib.rs:10456` `start_sk_accept_loop` and
`packages/d2bd/src/security_key.rs`). ProcessRole::SecurityKeyFrontend is
handled as a ReadinessOnly node that triggers this daemon coroutine rather than
spawning a runner through the broker. The v3 Provider target extracts this relay
logic into the `d2b-provider-device-security-key` crate running as a proper
unprivileged Process.

**Guest frontend Process** (per opted-in Guest):

The guest-side UHID virtual device binary is `packages/d2b-sk-frontend/`
(static binary; implemented-and-reachable). In v3 it becomes a Zone Process
resource owned by the device-security-key Provider controller:

```yaml
type: Process
metadata:
  name: device-<uid-short>-sk-frontend
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/<vm>
  domain: user
  processClass: worker
  # Package/template from device-security-key Provider dossier
```

The `d2b-sk-frontend.service` guest systemd unit is removed from the Nix module
when the Process resource is live; the Process controller manages the lifecycle.
The guest-side Nix module (`nixos-modules/components/security-key-guest.nix`)
continues to wire:
- `uhid` kernel module.
- `plugdev` group + udev rule for `KERNEL=="hidraw*" … KERNELS=="0003:1050:0407.*"`.
- The `d2b-sk-frontend` static binary in the Guest store.

### Ceremony, CID, and lease

The security-key lease lifecycle:

1. Guest firmware initiates a CTAP HID operation; the UHID frontend receives it.
2. The frontend sends an acquire-lease request to the host relay over AF_VSOCK.
3. The relay acquires the per-device OFD lock under `/run/d2b/locks/security-key/`.
4. The relay opens the physical hidraw fd via the broker (`SecurityKeyOpenDevice`).
5. The relay creates a session record (`SecurityKeySession`) with a new opaque
   `SecurityKeySessionId` and updates Device status to reflect the active claim.
6. CTAP HID traffic is proxied through the relay until the operation ends or
   times out.
7. On completion, the relay releases the OFD lock and updates session status.
8. The relay retains the session in a bounded ring (max `sessionRingSize`).

The broker's `SecurityKeyOpenDevice` operation:
- Accepts only `device_label` and `session_id` (audit correlation); no path.
- Derives the physical hidraw node from the trusted bundle device table.
- Returns the fd over SCM_RIGHTS; never a path in the response body.
- Audit record: subject, device_label digest, session_id, outcome.

### Status

Device status `device.claims` carries at most one entry for the active session:

```yaml
claims:
  - holderRef: Guest/corp-vm
    claim: exclusive
    passthrough: hidraw-relay
    claimedAt: 2026-07-22T00:05:00Z
    sessionId: sk-corp-vm-42   # opaque; NOT the physical device descriptor
    health: healthy
```

The `SecurityKeyStatusResponse` and `SecurityKeySessionsResponse` public wire
DTOs are reachable in `packages/d2b-contracts/src/security_key.rs` and are
preserved in v3 with adapted Zone/ResourceRef identifiers.

### Mutual exclusion

Security-key proxy and USBIP passthrough of the same physical USB device are
mutually exclusive. This is enforced:
- At Nix eval time in `assertions.nix`.
- At Device controller startup: if a Device/`<label>` is active under
  `device-usbip` and the same selector is requested by `device-security-key`,
  the second Device resource transitions to Failed with a `ClaimConflict`
  condition.

### Broker operations consumed

- `SecurityKeyOpenDevice`: open exact FIDO hidraw node; fd via SCM_RIGHTS.
- `SecurityKeyApplyUdevRules`: write udev rules; called once per activation.

### Tests

- `packages/d2b-contract-tests/tests/usb_sk_contract.rs`: DTO serde,
  unknown-field denial, broker capability set.
- `packages/d2b-core/src/privileges_w3.rs` unit tests: W3BrokerOperation flags.
- New: Lease acquire/release state machine (Idle → AwaitingLease → Active → Completed).
- New: Second-claim conflict rejection.
- New: Session ring bounded overflow (oldest session evicted).
- New: Broker `SecurityKeyOpenDevice` without inline path.
- New: USBIP mutual-exclusion eval check and runtime rejection.
- New: Guest frontend module udev rule format (`0003:1050:0407.*` pattern).

## Provider: device-gpu

### Identity

```text
Provider/device-gpu
```

Crate: `packages/d2b-provider-device-gpu/`
Dossier: `docs/specs/ADR-046-provider-device-gpu.md` (separate, not yet authored)

This Provider is the combined GPU/video Provider (D046). It manages GPU
graphics acceleration and hardware video decode under one crate and one Device
resource per Guest.

### Implements

Device (physical, exclusive per Guest, GPU/video combined).

### Root config schema

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `renderNodeOnly` | bool | false | — | If true, use render-node-only mode (no full virtio-gpu bind-mount) |
| `videoSidecar` | bool | false | — | If true, spawn crosvm video-decoder alongside GPU worker |
| `videoNvidiaDecode` | bool | false | — | If true, expose `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm` to video worker |
| `contextTypes` | list | [virgl, virgl2, cross-domain] | closed set | GPU context types: `virgl`, `virgl2`, `cross-domain` |
| `displays` | list | [{hidden: true}] | max 8 | Virtual display config |
| `egl` | bool | true | — | EGL rendering |
| `vulkan` | bool | true | — | Vulkan rendering |
| `crossDomainTrusted` | bool | false | — | Enable trusted cross-domain context (Wayland proxy path) |
| `virglVideo` | bool | false | — | Experimental virglrenderer video forwarding; separate from videoSidecar |

### Device spec

```yaml
spec:
  providerRef: Provider/device-gpu
  deviceClass: physical
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector:
      busClass: drm
      label: host-gpu
  settings:
    renderNodeOnly: false
    videoSidecar: true
    videoNvidiaDecode: false
    contextTypes: [virgl, virgl2, cross-domain]
    displays: [{hidden: true}]
    egl: true
    vulkan: true
    crossDomainTrusted: false
```

### Worker processes

| Process name | Role | Executable | Domain | Execution | Notes |
| --- | --- | --- | --- | --- | --- |
| `device-<uid-short>-gpu` | Process | `crosvm device gpu` | system | Host | Full GPU virtio sidecar; exclusive arbitration only |
| `device-<uid-short>-render-node` | Process | `crosvm device gpu` (render-node mode) | system | Host | Render-node-only; exclusive or shared per Device spec |
| `device-<uid-short>-video` | Process | `crosvm device video-decoder --backend vaapi` | system | Host | Video decode; separate Process, shares Device claim |

Only one of `device-<uid-short>-gpu` and `device-<uid-short>-render-node` is
active at a time, selected by `settings.renderNodeOnly`. The video Process is
active only when `settings.videoSidecar=true`.

Process resource names are derived deterministically from the owner Device UID
(`uid-short` = first 12 hex chars) and a component template from the signed
Provider dossier. Current v3 OS-level process principal names (`d2b-{vm}-gpu`,
`d2b-{vm}-gpu-render-node`) from `bundle_resolver.rs` ProcessRole exec_arg0 are
the process identity at the OS layer; they are distinct from the Process resource
name. VM or Guest human names never appear in Process resource names.

### Device claims and broker tokens

The GPU Device broker token set (from `bundle_resolver.rs` ProcessRole::Gpu
exact comment):

- `kvm`, `dri`, `nvidia-ctl`, `nvidia-uvm`, `nvidia-render`, `udmabuf`.
- `nvidia-ctl`, `nvidia-uvm`, `nvidia-render` are included only when
  `videoNvidiaDecode=true` or when the guest is a NVIDIA-graphics VM.

Device default allowlist in the privileged broker for the GPU worker:
- `/dev/dri/renderD128` always.
- `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm` only when `videoNvidiaDecode=true`.

### Video wire contract pins

The following constants from `packages/d2b-host/src/video_argv.rs` are frozen
wire contract values and must be preserved by the device-gpu Provider:

| Constant | Value | Source |
| --- | --- | --- |
| `VIRTIO_ID_MEDIA` | 48 | CH patch `0003-vhost-user-media-device.patch` |
| `VHOST_USER_MEDIA_NUM_QUEUES` | 2 | CH patch `NUM_QUEUES` |
| `VHOST_USER_MEDIA_QUEUE_SIZE` | 256 | CH patch `QUEUE_SIZES` |
| `VHOST_USER_MEDIA_SHM_REGION_BYTES` | 268435456 (256 MiB) | CH patch SHM region |
| `VHOST_USER_MEDIA_VRING_BASE` | 0 | CH patch `activate()` override |
| `VHOST_USER_MEDIA_PROTOCOL_FLAGS` | `BACKEND_REQ\|REPLY_ACK\|SHMEM_MAP_CROSVM` | CH patch `acked_protocol_features` |

These constants are part of the vhost-user-media wire contract between the
patched Cloud Hypervisor and the crosvm video-decoder sidecar. Changing them
requires an updated CH patch.

### Broker operations consumed

- `SpawnRunner` (gpu/gpu-render-node/video role): spawn crosvm worker Process.
- `OpenDevice` (kvm/dri/udmabuf/nvidia*): open GPU device fds before clone.

### Nix options (v3 successors)

Current:
- `d2b.vms.<vm>.graphics.enable = true` → `nixos-modules/components/graphics.nix` (host)
- `d2b.vms.<vm>.graphics.videoSidecar = true` → `nixos-modules/components/video/guest.nix`
- `d2b.graphics.*` in guest NixOS config

v3 Device resource:
```nix
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
    settings = {
      videoSidecar      = true;
      contextTypes      = ["cross-domain" "virgl" "virgl2"];
      crossDomainTrusted = false;
    };
  };
};
```

Guest-side `virtio_media` module, kernel packages, and `--vhost-user-media`
CH arg remain in the Guest `runtime-cloud-hypervisor` Nix module.

### Tests

- `packages/d2b-contract-tests/tests/minijail_gpu.rs`: GPU minijail profile.
- `packages/d2b-contract-tests/tests/minijail_swtpm_video.rs`: video minijail profile.
- `packages/d2b-contract-tests/tests/video_binary_contract.rs`: video binary contract.
- `packages/d2b-host/src/gpu_argv.rs` unit tests: GPU argv golden.
- `packages/d2b-host/src/video_argv.rs` unit tests: video argv golden + wire-contract snapshot.
- New: GPU+video combined Device reconcile state-machine test.
- New: `renderNodeOnly` vs full-GPU Process selection test.
- New: Video sidecar Process dependency on GPU Process readiness.
- New: Wire-contract constant snapshot byte-stability test.

## RBAC

Device resources use standard resource verbs. The following roles are required:

| Role | Verbs | Scope | Subjects |
| --- | --- | --- | --- |
| `device-manager` | get, list, watch, create, update-spec, delete | Zone | Provider controllers |
| `device-status-owner` | update-status | Zone | Owning Device Provider controller only |
| `device-finalizer-owner` | update-finalizers | Zone | Owning Device Provider controller only |
| `device-reader` | get, list, watch | Zone | Guest/Host runtime Providers, CLI |
| `device-claimant` | get, watch | Zone | Guest/Host runtime Providers that hold claims |

Device attachments in Host/Guest/Process specs are desired state, not imperative
operations. Adding or removing a `devices[]` entry on a Host or Guest resource
is a normal spec update governed by `update-spec` RBAC. The Device Provider
controller detects the change via its reconcile loop, performs arbitration, and
reflects the result in Device status. There is no separate `claim-device` or
`release-device` verb.

No Role grants wildcard `*` over all Device resources. Provider controllers
have bounded RoleBinding scopes that cover only the Device types and names
matching their provisioned resource set.

## Broker effect limits

All Device Provider broker operations are subject to:

- Audit-logged (no broker op without an audit record).
- Point-specific: no device Provider receives a blanket device-path grant.
- One broker connection per Provider process; FDs returned over SCM_RIGHTS.
- Broker validates all inputs against the trusted bundle before any effect.
- No inline path, raw device node string, or capability byte crosses the
  public or broker wire.

Additional per-operation limits (confirmed conservative defaults; a conformance
test for each limit is required in `packages/d2b-contract-tests/`):

| Broker operation | Rate limit | FD quota | Notes |
| --- | --- | --- | --- |
| `SecurityKeyOpenDevice` | 1 concurrent per device label | 1 | One active hidraw session per Device at a time |
| `SecurityKeyApplyUdevRules` | Activation-only | — | One batch per Provider activation; not a hot path |
| `UsbipBindFirewallRule` | One bounded batch per activation | — | Ownership-marker check prevents duplicate rules |
| `SpawnRunner` (swtpm) | 1 per Device per Guest start cycle | — | Idempotent; broker verifies tamper marker |
| `SpawnRunner` (gpu/video) | 1 per Device (one GPU worker set per Guest) | — | One GPU worker set per Device |
| `OpenDevice` (gpu) | Per-spawn call only | ≤8 per Process launch | Opened before clone; counted per-spawn |

## Audit and OTEL

### Audit records

Each Device broker operation emits a path-free audit record containing:

| Field | Value |
| --- | --- |
| `subject` | Provider Process identity digest |
| `zone` | Zone name |
| `op` | Broker operation tag (e.g., `SecurityKeyOpenDevice`) |
| `resource_type` | `Device` |
| `resource_name_digest` | Bounded stable hash of device label/name; never raw path |
| `outcome` | `success` \| `failure` \| `denied` |
| `error_class` | closed-set error slug |
| `correlation_id` | operation/trace ID |
| `timestamp` | RFC 3339 UTC |

The audit record excludes: raw device paths, hidraw node names, sysfs bus IDs,
vendor/product strings, CTAP session contents, TPM NVRAM contents, GPU sockets,
and any credential material.

### OTEL spans

Device reconcile telemetry attribute placement — including span vs resource
attribute classification, `d2b.device.zone` cardinality, `d2b.device.provider`
label level, and full label set boundaries — is specified in
`ADR-046-telemetry-audit-and-support`. This spec does not define competing
telemetry decisions. Device Provider implementations must cross-reference that
spec for all OTEL label and span attribute constraints. No device path, busid,
serial, vendor/product string, session content, or process PID may appear in
any OTEL attribute or metric label.

## Errors

Stable Device-specific error classes (in addition to common resource errors):

| Error | Meaning |
| --- | --- |
| `device-not-found` | Physical device absent from sysfs/udev at probe time |
| `device-claim-conflict` | Exclusive device already claimed by another holder |
| `device-claim-max-exceeded` | `maxConcurrentClaims` reached |
| `device-arbitration-violation` | Exclusive Device received a shared claim or vice versa |
| `device-provision-failed` | Emulated device creation failed |
| `device-broker-inaccessible` | Broker cannot open/pass the device fd |
| `device-state-integrity-failure` | Tamper marker mismatch (TPM only) |
| `device-session-timeout` | CTAP session exceeded lease timeout |
| `device-session-cancelled` | CTAP session cancelled by operator |
| `device-mutual-exclusion-violation` | security-key and USBIP for same physical device |
| `device-worker-failed` | Owned worker Process in Failed/Unknown phase |
| `device-wire-contract-mismatch` | Video or GPU wire constant mismatch |

All error messages are bounded, UTF-8 validated, and must not contain device
paths, sysfs bus IDs, raw hidraw node names, TPM NVRAM content, CTAP session
bytes, GPU socket paths, or credential material.

## Nix configuration

### Nix authoring shape

All resources in a Zone are declared under `d2b.zones.<zone>.resources`. The
shape mirrors the canonical resource envelope directly: `type` selects the
ResourceType, `metadata` carries author-settable metadata fields (only
`ownerRef` and optional presentation `labels`/`annotations`), and `spec`
carries the exact ResourceType spec fields without renaming or re-nesting.

```nix
d2b.zones.<zone>.resources.<name> = {
  type = "Device";              # ResourceType discriminant

  # Author-settable metadata only.
  # metadata.name, metadata.zone, and apiVersion are derived (see table below).
  # metadata.managedBy, metadata.configurationGeneration, uid, generation,
  # revision, timestamps, finalizers, and managedBy are Core-managed (omitted).
  # status is read-only (omitted).
  metadata.ownerRef = "Guest/<vm>";          # ResourceRef; required for Device
  # metadata.labels.<key>  = "<value>";      # optional presentation labels
  # metadata.annotations.<key> = "<value>";  # optional annotations

  spec = {
    # Required
    providerRef   = "Provider/device-tpm";  # must resolve to installed Provider
    deviceClass   = "emulated";             # "emulated" | "physical"
    arbitration   = "exclusive";            # "exclusive" | "shared"

    # Optional — defaults shown
    maxConcurrentClaims = 1;               # 1–16; must be 1 when arbitration=exclusive

    inventory.selector = {};               # emulated: {} or absent
    # inventory.selector.busClass = "usb" | "hidraw" | "drm" | "pci" | "tpm";
    # inventory.selector.label    = "<stable-label>";  # required for physical
    # inventory.selector.<field>  = ...;               # variant-specific fields only

    settings = {};   # Provider-specific; see Provider settings schema
  };
};
```

**Derived and read-only fields (not author-specified):**

| Field | Derived from |
| --- | --- |
| `metadata.name` | Resource attribute key `<name>` |
| `metadata.zone` | Zone attribute key `<zone>` |
| `apiVersion` | Constant `"resources.d2b.io/v3"` |
| `metadata.uid` | Assigned by Core on first creation |
| `metadata.generation` | Incremented by Core on each spec change |
| `metadata.revision` | Opaque; set by Core |
| `metadata.createdAt` / `updatedAt` | Set by Core |
| `metadata.finalizers` | Written by Provider controllers |
| `metadata.deletionRequestedAt` | Set by Core on Delete |
| `metadata.managedBy` | `"configuration"` — set by Core when activating the Nix bundle; closed enum: `configuration \| controller \| api`; Nix input omits it |
| `metadata.configurationGeneration` | NixOS system generation number — set by Core at activation |
| `status` | Entirely read-only; managed by Provider controller |

**Nix option types, defaults, and documentation** are generated from the
committed ResourceTypeSchema (`docs/reference/schemas/v3/device.schema.json`) and
the per-Provider settings schemas. There is no second bespoke Nix vocabulary:
`spec` field names, types, bounds, and defaults in Nix are identical to those in
the schema. A `spec` field absent from the schema fails eval. A `settings` field
absent from the Provider schema fails eval.

### Artifact catalog

Derivation-valued inputs (packages, toolchains, NixOS systems) are configured in
a separate named artifact catalog, never as inline fields inside a ResourceSpec:

```nix
d2b.artifacts.<id> = {
  package = <derivation>;   # Nix derivation; store path is private catalog data
  type    = "provider"      # closed set: "provider" | "nixos-system" | …
          | "nixos-system"
          | ...;
};
```

**Key invariants:**

- ResourceSpec fields that reference catalog entries use a plain bounded ID
  (`artifactId` or `systemArtifactId` — **not** `*Ref`, because `Artifact` is
  not a ResourceType). For example, a Provider resource's `spec.artifactId`
  references its own `d2b-provider-device-*` package; a Guest resource's
  `spec.systemArtifactId` references its NixOS system derivation.
- The Nix build hashes the derivation, validates catalog `type`/`id`/duplicate
  invariants and trust, and emits a private integrity-pinned artifact catalog
  mapping each `id` to `type`, content digest, and closure metadata.
- Store paths are private catalog implementation data. They never appear in
  ResourceSpec fields, resource status, audit records, error messages, or
  telemetry.
- A missing `artifactId` value fails the NixOS build with `artifact-id-not-found`.
- A catalog entry whose `type` does not match the field's required type fails
  with `artifact-type-mismatch`.

**Device Provider binary resolution** does not use `artifactId` fields in the
Device `spec.settings`. The swtpm, swtpm_ioctl, and usbip binaries are
dependencies bundled inside the `d2b-provider-device-tpm` and
`d2b-provider-device-usbip` package closures respectively. Their exact
executable paths and content digests are embedded in the **signed component
descriptor** shipped inside each Provider's closure; the Provider process
resolves them at startup without any Device-spec field. No Device Provider
settings field carries a store path or an `artifactId`.

The four frozen Device Providers are therefore all closed with respect to
artifact catalog references from the Device resource's spec: the Device spec
carries only configuration values (logLevel, arbitration, selector, etc.). The
Provider resources themselves (defined by the Provider ResourceType, not this
spec) use `spec.artifactId` to reference their packages.

### Eval-time validation

The Nix emitter validates the following rules at `nixos-rebuild` / `nix flake
check` time against the committed ResourceTypeSchema:

| Rule | Eval error |
| --- | --- |
| `metadata.ownerRef` is `<Type>/<name>` format | `invalid-owner-ref` |
| `spec.providerRef` is `Provider/<name>` format | `invalid-provider-ref` |
| `spec.providerRef` resolves to an installed Provider in the Zone | `unresolved-provider-ref` |
| `spec.deviceClass` ∈ `{"emulated","physical"}` | `invalid-device-class` |
| `spec.deviceClass=emulated` ⟹ `spec.inventory.selector = {}` or absent | `emulated-with-nonempty-selector` |
| `spec.deviceClass=physical` ⟹ `spec.inventory.selector.label` present | `physical-missing-selector-label` |
| `spec.inventory.selector.busClass` ∈ closed set `{usb,hidraw,drm,pci,tpm}` | `unknown-bus-class` |
| Selector contains only fields declared for its `busClass` variant | `selector-unknown-field` |
| `spec.arbitration=exclusive` ⟹ `spec.maxConcurrentClaims = 1` | `exclusive-max-claims-conflict` |
| `spec.arbitration=shared` ⟹ `spec.deviceClass=physical` | `emulated-shared-arbitration` |
| `spec.arbitration=shared` + GPU Provider ⟹ `spec.settings.renderNodeOnly=true` | `shared-arbitration-requires-render-node-only` |
| `spec.maxConcurrentClaims` ∈ 1–16 | `max-claims-out-of-bounds` |
| No two Device resources in the same Zone share the same `spec.inventory.selector.label` | `duplicate-device-label` |
| USBIP and security-key Provider both referencing same selector label | `usbip-sk-mutual-exclusion` |
| `spec.settings` validates against the Provider's signed JSON Schema | `invalid-provider-settings` |
| No inline secret strings in `spec.settings` (must use `credentialRef`) | `inline-secret-in-settings` |

### Provider settings schema validation

Each Device Provider registers a JSON Schema for its `settings` sub-object as
part of its signed Provider descriptor. The Nix emitter imports this schema from
the Provider's store path and validates `spec.settings` against it at eval time.
The schema fingerprint is committed to
`docs/reference/schemas/v3/providers/device-<name>.settings.json`; the drift
gate (`make test-drift` / `cargo xtask gen-schemas`) fails the build on any
mismatch.

Settings fields that accept sensitive values use a `credentialRef` entry pointing
to a `Credential/<name>` resource in the same Zone. Inline strings in sensitive
settings fields fail eval with `inline-secret-in-settings`. The four frozen
Device Providers have no `artifactId` field in their `spec.settings`; binary
paths are resolved from Provider package closures (see "Artifact catalog").

```nix
# Correct — sensitive value via Credential ref
spec.settings.exampleSecret = { credentialRef = "Credential/device-example-key"; };

# Rejected at eval time — inline string in a credentialRef-constrained field
spec.settings.exampleSecret = "raw-secret-value";
```

The four frozen Device Providers have no settings fields that accept secrets.
The `credentialRef` constraint is stated here for future Device Providers.

### Canonical ResourceSpec JSON

The build emits one JSON object per Device resource, representing the full
resource envelope. All keys are sorted lexicographically at every nesting level.
The emitted JSON contains `apiVersion`, `type`, `metadata`, and
`spec` only. `status` is omitted; Core-managed fields (`uid`, `generation`,
`revision`, `createdAt`, `updatedAt`, `finalizers`, `deletionRequestedAt`) are
not emitted by the Nix build and are filled by the runtime after first apply.

**device-tpm:**

```json
{
  "apiVersion": "resources.d2b.io/v3",
  "type": "Device",
  "metadata": {
    "name": "corp-vm-tpm",
    "ownerRef": "Guest/corp-vm",
    "zone": "dev"
  },
  "spec": {
    "arbitration": "exclusive",
    "deviceClass": "emulated",
    "inventory": { "selector": {} },
    "maxConcurrentClaims": 1,
    "providerRef": "Provider/device-tpm",
    "settings": {
      "logLevel": 20,
      "startupClear": true
    }
  }
}
```

**device-usbip:**

```json
{
  "apiVersion": "resources.d2b.io/v3",
  "type": "Device",
  "metadata": {
    "name": "corp-vm-usb",
    "ownerRef": "Guest/corp-vm",
    "zone": "dev"
  },
  "spec": {
    "arbitration": "exclusive",
    "deviceClass": "physical",
    "inventory": {
      "selector": {
        "busClass": "usb",
        "label": "yubikey-work",
        "productId": "0407",
        "serial": null,
        "vendorId": "1050"
      }
    },
    "maxConcurrentClaims": 1,
    "providerRef": "Provider/device-usbip",
    "settings": { "env": "work" }
  }
}
```

**device-security-key:**

```json
{
  "apiVersion": "resources.d2b.io/v3",
  "type": "Device",
  "metadata": {
    "name": "corp-vm-security-key",
    "ownerRef": "Guest/corp-vm",
    "zone": "dev"
  },
  "spec": {
    "arbitration": "exclusive",
    "deviceClass": "physical",
    "inventory": {
      "selector": {
        "busClass": "hidraw",
        "label": "yubikey-primary",
        "productId": "0407",
        "serial": null,
        "vendorId": "1050"
      }
    },
    "maxConcurrentClaims": 1,
    "providerRef": "Provider/device-security-key",
    "settings": {
      "leaseTimeoutSecs": 300,
      "sessionRingSize": 32,
      "vsockPort": 14320
    }
  }
}
```

**device-gpu (full GPU, exclusive):**

```json
{
  "apiVersion": "resources.d2b.io/v3",
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
      "displays": [{ "hidden": true }],
      "egl": true,
      "renderNodeOnly": false,
      "videoNvidiaDecode": false,
      "videoSidecar": false,
      "virglVideo": false,
      "vulkan": true
    }
  }
}
```

**device-gpu (render-node only, shared):**

```json
{
  "apiVersion": "resources.d2b.io/v3",
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

Array elements are sorted: primitive arrays lexicographically, object arrays by
first key. `null`-default optional selector fields are included explicitly; absent
settings fields receive their schema default.

### Zone resource bundle/generation

The Nix build produces a Zone resource generation bundle at
`/etc/d2b/zones/<zone>/resources.json`:

```json
{
  "schemaVersion": 1,
  "zone": "dev",
  "configGeneration": 42,
  "generatedAt": "2026-07-22T00:00:00Z",
  "contentDigest": "sha256:<hex>",
  "resources": [
    { "apiVersion": "resources.d2b.io/v3", "type": "Device", "metadata": { ..., "name": "corp-vm-gpu" },          "spec": { ... } },
    { "apiVersion": "resources.d2b.io/v3", "type": "Device", "metadata": { ..., "name": "corp-vm-security-key" }, "spec": { ... } },
    { "apiVersion": "resources.d2b.io/v3", "type": "Device", "metadata": { ..., "name": "corp-vm-tpm" },          "spec": { ... } },
    { "apiVersion": "resources.d2b.io/v3", "type": "Device", "metadata": { ..., "name": "corp-vm-usb" },          "spec": { ... } }
  ]
}
```

Bundle properties:

- `resources` sorted lexicographically by `(type, metadata.name)`.
- `contentDigest` = SHA-256 of the canonical serialization of the `resources`
  array alone (sorted keys, no trailing whitespace, no BOM).
- `configGeneration` is the NixOS system generation number.
- Two bundles with identical `contentDigest` produce no resource changes at the
  Zone runtime regardless of `configGeneration`.
- The Nix build fails if any `(type, metadata.name)` pair is duplicated, if the
  computed digest does not match, if any spec fails ResourceTypeSchema
  validation, or if any settings fails Provider schema validation.
- The bundle file is `root:d2bd` `0640`; the Zone runtime reads it at startup and
  on generation-change signal.

### Assertions at eval time

New assertions added to `nixos-modules/assertions.nix`:

1. A Guest with `tpm.enable=true` must declare exactly one Device resource with
   `spec.providerRef = "Provider/device-tpm"` and `metadata.ownerRef` pointing to
   that Guest.
2. Security-key proxy and USBIP passthrough of the same physical device label are
   mutually exclusive: no two Device resources in the same Zone may have the same
   `spec.inventory.selector.label` if one uses `Provider/device-security-key` and
   the other uses `Provider/device-usbip`.
3. `spec.settings.videoSidecar=true` requires a Device resource for the same
   Guest with `spec.settings.renderNodeOnly=false` (full GPU).
4. `spec.settings.virglVideo=true` and `spec.settings.videoSidecar=true` are
   mutually exclusive on the same Guest.
5. A Device with `spec.arbitration=shared` requires `spec.deviceClass=physical`
   and, for `Provider/device-gpu`, `spec.settings.renderNodeOnly=true`.
6. No two Device resources in the same Zone have the same
   `spec.inventory.selector.label` value.

## Zone generation and cleanup

### Generation model

A NixOS build produces a Zone resource generation: the complete set of Device
(and other resource-type) specs declared in the Nix config for that Zone. The
generation has a monotonic `configGeneration`, a `contentDigest`, and the full
set of `managedBy=configuration` resource descriptors. Applying a generation with
the same `contentDigest` as the current active generation is a no-op.

### Configuration-owned vs controller-created resources

| Metadata field | Value | Set by | Meaning |
| --- | --- | --- | --- |
| `metadata.managedBy` | `"configuration"` | Core at bundle activation | Subject to generation-based lifecycle |
| `metadata.managedBy` | `"controller"` | Controller when creating child resources | Never subject to generation deletion |
| `metadata.managedBy` | `"api"` | Core when resource is created through the API | Persists until explicit Delete; never subject to generation deletion |

`metadata.managedBy` is a closed enum (`configuration | controller | api`). Nix
input never sets it; Core sets it at activation or creation. Core uses it as the
definitive ownership marker. It:

- Scopes generation-based deletion strictly to `managedBy=configuration`.
- Never deletes a resource with `managedBy=controller` or `managedBy=api` based
  on generation absence.
- Sets `managedBy=configuration` on every resource reconciled from the Nix bundle;
  never sets it on resources created by Provider controllers or through the API.

Provider controllers create child resources (Processes, EphemeralProcesses,
Volumes) with `managedBy=controller`. The Device Provider controller manages them
entirely through its reconcile loop and finalizer sequence; generation cleanup
never touches them.

### Cleanup contract

When the Zone runtime activates generation N+1:

1. **Create/update:** `managedBy=configuration` resources present in N+1 but not
   N are Created; those present in both with changed spec receive a
   `spec-generation-changed` reconcile trigger.
2. **Mark stale for deletion:** `managedBy=configuration` resources present in N
   but absent from N+1 receive `deletionRequestedAt` (normal Delete request). The
   Zone transitions to `Degraded/pending-cleanup`.
3. **Non-blocking:** generation N+1 is fully active and operational while stale
   resource deletion proceeds asynchronously. New workloads start immediately.
4. **Standard sequence per deleted resource:** Provider finalizer handler runs →
   terminates owned Processes → releases OS resources → clears finalizer → Core
   removes resource. Duration is bounded per-Provider by the existing finalizer
   timeout contract.
5. **Controller-created children:** child Processes/Volumes of a deleted Device
   carry `managedBy=controller` and are owned by the Device Provider's finalizer,
   not by generation cleanup. Core does not issue Delete to them; the Device
   finalizer handler deletes them as part of its normal sequence.
6. **Orphan-sweep prohibition:** Core never sweeps `managedBy=controller`
   resources for deletion merely because they are absent from the Nix bundle.
7. **Owner-controller reconcile:** if a `managedBy=configuration` resource's spec
   changes (e.g., Device settings updated), its child Processes receive
   `dependency-changed` triggers and the owner controller reconciles them.

### Zone cleanup status

```yaml
# Zone status while cleanup is pending (generation N → N+1)
status:
  phase: Degraded
  conditions:
    - type: GenerationCleanPending
      status: "True"
      reason: config-resources-pending-deletion
      message: "2 Device resource(s) pending deletion from prior generation"
  activeGeneration: 43
  priorGeneration: 42
  pendingDeletion:
    - type: Device
      name: old-corp-vm-tpm
      deletionRequestedAt: "2026-07-22T02:00:00Z"
    - type: Device
      name: old-corp-vm-usb
      deletionRequestedAt: "2026-07-22T02:00:00Z"

# Zone status after cleanup completes
status:
  phase: Ready
  conditions:
    - type: GenerationCleanPending
      status: "False"
      reason: generation-cleanup-complete
  activeGeneration: 43
  priorGeneration: null
```

The `Degraded/pending-cleanup` state does not affect scheduling of new workloads
under generation N+1. Resources added in N+1 are reconciled immediately.

### Prior generation retention

The Zone runtime retains prior generation bundles for diagnostics and rollback
reference. Retention is count-based:

- `d2b.zones.<zone>.priorGenerationRetentionCount` — number of prior generations
  to retain; default `3`; valid range `1..16`.
- When the count of retained prior generations would exceed the configured value,
  the oldest fully-cleaned generation is pruned first.
- A generation is eligible for pruning only after all `managedBy=configuration`
  resources it declared that are absent from the successor generation have reached
  phase `Deleted` (or been removed from the store).
- Operator explicit prune via `d2b zone gc <zone>` — prunes all fully-cleaned
  generations beyond the retention count immediately.

Re-applying a prior bundle requires explicit operator action.

### Cleanup audit records

Each `managedBy=configuration` resource deletion triggered by a generation transition emits:

| Field | Value |
| --- | --- |
| `event` | `config-resource-deletion-requested` |
| `zone` | Zone name |
| `type` | Resource type (e.g., `Device`) |
| `resource_name_digest` | SHA-256 of the resource name; never raw name |
| `prior_generation` | N |
| `active_generation` | N+1 |
| `reason` | `absent-from-new-generation` |
| `timestamp` | RFC 3339 UTC |

Per-op finalizer/broker audit records continue to be emitted by the Device
Provider controller's existing per-op audit path.

### Cleanup error classes

| Error | Meaning |
| --- | --- |
| `cleanup-finalizer-stuck` | Provider finalizer not cleared within bounded timeout; Zone stays `Degraded/pending-cleanup`; operator must inspect |
| `cleanup-child-deletion-failed` | Child Process/Volume deletion failed during Device finalizer; retried with exponential backoff |
| `cleanup-config-ownership-mismatch` | Generation deletion attempted on a resource with `managedBy=controller` or `managedBy=api`; fails closed and audited |
| `cleanup-controller-resource-protected` | Generation cleanup attempted to delete a controller-created resource; rejected, audited, and reported as invariant violation |

## Tests

### Layer-1 unit tests (Nix eval)

- `tests/unit/nix/cases/device-tpm-eval.nix`: Device spec round-trip for TPM; emitted JSON golden vector.
- `tests/unit/nix/cases/device-usbip-eval.nix`: USBIP discriminated-union selector validation; unknown-field rejection.
- `tests/unit/nix/cases/device-security-key-eval.nix`: security-key mutual-exclusion assertion; Credential-ref requirement.
- `tests/unit/nix/cases/device-gpu-eval.nix`: GPU + video settings validation; shared-arbitration render-node-only enforcement.
- `tests/unit/nix/cases/device-schema-validation.nix`: eval-time rule corpus — one test per validation row in the eval-time validation table; each must reject with the documented error slug.
- `tests/unit/nix/cases/device-gen-cleanup-eval.nix`: generation diff — resource removed from Nix config appears in `pendingDeletion`; resource absent from prior generation does not appear; bundle `contentDigest` changes.
- `tests/unit/nix/cases/device-bundle-canonical.nix`: bundle JSON is canonical (sorted keys, sorted resources, stable contentDigest); two identical config subtrees produce identical digests.
- `tests/unit/nix/cases/device-inline-secret-rejected.nix`: inline string in settings field with `credentialRef` constraint fails eval with `inline-secret-in-settings`.
- `tests/unit/nix/cases/device-artifact-catalog.nix`: store path absent from all Device ResourceSpec JSON outputs; `spec.settings` for TPM/USBIP carries no `artifactId` field; private catalog structure (type/digest/closure) is not present in the emitted resource bundle.

### Layer-1 Rust (contract tests)

| File | Scope |
| --- | --- |
| `packages/d2b-contract-tests/tests/minijail_swtpm_video.rs` | TPM/video minijail profile (reused) |
| `packages/d2b-contract-tests/tests/minijail_gpu.rs` | GPU minijail profile (reused) |
| `packages/d2b-contract-tests/tests/usbip_json_contract.rs` | USBIP DTO serde (reused) |
| `packages/d2b-contract-tests/tests/usb_sk_contract.rs` | Security-key DTO/broker capability (reused) |
| `packages/d2b-contract-tests/tests/video_binary_contract.rs` | Video wire-contract snapshot (reused) |
| New: `packages/d2b-contract-tests/tests/device_resource_schema.rs` | Device ResourceTypeSchema golden vectors; unknown-field denial; discriminated-union busClass rejection corpus; no `artifactId` or store-path field in any Device Provider settings schema |
| New: `packages/d2b-contract-tests/tests/device_provider_dossiers.rs` | Provider dossier completeness/schema conformance; settings schema fingerprint matches committed file |
| New: `packages/d2b-contract-tests/tests/device_bundle_canonical.rs` | Bundle JSON canonical form: sorted keys, sorted resources, stable contentDigest, duplicate-(type,name) rejection, digest mismatch fails |
| New: `packages/d2b-contract-tests/tests/device_gen_cleanup.rs` | Generation lifecycle: `managedBy=configuration` set on emitted resources; `managedBy=controller` on controller-created resources; `managedBy=api` on API-created resources; stale `managedBy=configuration` resource receives DeleteRequest; `managedBy=controller` and `managedBy=api` resources never receive generation-Delete |

### Layer-1 Rust (Provider tests — `src/` colocated unit tests)

Colocated `#[cfg(test)]` modules within each Provider crate's `src/`:

- `packages/d2b-provider-device-tpm/src/` — swtpm argv golden vectors, state-dir
  hardening, tamper-marker detection, flush → start sequencing, finalizer
  non-deletion invariant.
- `packages/d2b-provider-device-usbip/src/` — bus ID corpus (31-char max, metachar
  rejection, leading-zero segments), firewall rule ownership-marker format,
  bind/unbind EphemeralProcess creation.
- `packages/d2b-provider-device-security-key/src/` — lease acquire/release
  transitions, session ring eviction at capacity, broker op path-free invariant,
  CID translation round-trip.
- `packages/d2b-provider-device-gpu/src/` — GPU/video process role selection,
  wire-constant snapshot stability, render-node vs full-GPU path branching.

### Layer-1 Rust (Provider tests — `tests/` hermetic Cargo integration)

Each Provider crate's `tests/` directory; run with `cargo test -p d2b-provider-device-<name>`:

| Crate | Tests |
| --- | --- |
| `d2b-provider-device-tpm/tests/` | `controller_state_machine.rs` — flush→swtpm→Ready cycle with fake broker; `conformance.rs` — spec/status serde vs ResourceTypeSchema; `fault_swtpm_missing.rs` — swtpm absent → phase Degraded |
| `d2b-provider-device-usbip/tests/` | `arbitration_conflict.rs` — second-claim rejects; `conformance.rs` — spec/settings serde; `firewall_marker.rs` — ownership marker preserved in rule; `explicit_attach_split.rs` — EphemeralProcess bind vs declared Process |
| `d2b-provider-device-security-key/tests/` | `lease_state_machine.rs` — full acquire/cancel/expire cycle; `session_ring.rs` — ring wrap and eviction; `mutual_exclusion.rs` — USBIP+SK same label rejected; `conformance.rs` — spec/status serde; `guest_frontend_process.rs` — frontend Process resource fields |
| `d2b-provider-device-gpu/tests/` | `combined_reconcile.rs` — gpu+video combined state machine; `render_node_enforcement.rs` — shared+renderNodeOnly=false rejected; `wire_constant_snapshot.rs` — byte-stable wire-contract constants; `conformance.rs` — spec/settings serde |

### Layer-2 integration tests

- `tests/integration/containers/device-tpm-state.sh`: TPM provision/boot/reboot
  cycle; tamper-marker survives restart.
- `tests/integration/containers/device-security-key-lease.sh`: lease acquire,
  cancel, and session ring.
- `tests/integration/containers/device-usbip-arbitration.sh`: second-claim
  conflict rejection.
- `tests/integration/containers/device-gen-cleanup.sh`: full generation cleanup
  cycle — apply generation N with two Devices, apply generation N+1 removing one
  Device, verify: (a) Zone transitions to `Degraded/pending-cleanup`, (b) removed
  Device enters finalizer sequence, (c) child Processes of removed Device are
  deleted by Device finalizer (not by Core), (d) Zone remains operational for the
  retained Device during cleanup, (e) Zone transitions to `Ready` after cleanup
  completes, (f) `GenerationCleanPending` condition clears.
- `tests/integration/containers/device-controller-resource-protected.sh`: start a
  Device, let Provider create a child Process, then switch to a new generation
  that does not change the Device; verify the child Process is NOT deleted by
  generation cleanup and has `managedBy=controller` (not `managedBy=configuration`).
- `tests/integration/containers/device-gen-cleanup-audit.sh`: each stale-resource
  deletion triggered by generation change emits a `config-resource-deletion-requested`
  audit record with correct `prior_generation`, `active_generation`, and
  `resource_name_digest` (never raw name).

### Layer-2 integration tests (Provider crate `integration/` fixtures)

Each Provider crate's `integration/` directory contains heavier container/Host/Guest
scenarios invoked by `make test-integration`. These are distinct from the
top-level `tests/integration/containers/` scripts and are co-located with the
Provider they test:

| Crate `integration/` path | Scenarios |
| --- | --- |
| `d2b-provider-device-tpm/integration/` | `provision_and_reboot/` — full TPM provision → Guest boot → reboot cycle; `tamper_marker_survives/` — marker present after Provider restart; `finalizer_no_delete/` — Volume not deleted on Device finalizer |
| `d2b-provider-device-usbip/integration/` | `arbitration_conflict/` — second Host claim rejected at runtime; `busid_bind_cycle/` — full modprobe→lock→withhold→firewall→bind→proxy bringup; `network_firewall_coexistence/` — Provider firewall rule does not clobber Network rules |
| `d2b-provider-device-security-key/integration/` | `lease_acquire_cancel/` — full acquire → cancel → re-acquire cycle; `session_ring_capacity/` — ring wraps correctly under real vsock load; `guest_frontend_connect/` — Guest frontend Process connects and authenticates over AF_VSOCK |
| `d2b-provider-device-gpu/integration/` | `gpu_worker_start/` — GPU worker Process obtains broker tokens and becomes Ready; `render_node_shared/` — two Guests share render-node Device simultaneously; `video_dependency/` — video-decoder Process starts only after gpu worker Process is Ready |

### Feasibility proofs

- Device reconcile state machine with fake supervisor and broker.
- Security-key relay process with fake hidraw fd and fake vsock Guest.
- swtpm flush → start EphemeralProcess ordering with fake broker.
- GPU worker process broker token set verification.
- Generation cleanup: fake Zone runtime with two Devices; remove one from
  config; assert only the `managedBy=configuration` stale resource receives DeleteRequest;
  assert controller-created child is not touched; assert Zone stays operational.

## ProcessRole → future path disposition

| Current v3 ProcessRole | Future v3 resource | Provider | Notes |
| --- | --- | --- | --- |
| `SwtpmPreStartFlush` | `EphemeralProcess/<vm>-tpm-flush` | `device-tpm` | Pre-start ioctl flush; `swtpm_ioctl -i --unix <ctrl.sock>` |
| `Swtpm` | `Process/<vm>-tpm-swtpm` | `device-tpm` | Long-lived swtpm socket process; user NS, zero host caps |
| `Usbip` | `Process/<vm>-usbip-daemon` + `EphemeralProcess/<vm>-usbip-bind/unbind` | `device-usbip` | Daemon and per-busid bind/unbind |
| `SecurityKeyFrontend` | **Host relay**: `Process/device-<uid-short>-sk-relay` (extracted from d2bd daemon-internal accept loop in `packages/d2bd/src/lib.rs:10456` and `packages/d2bd/src/security_key.rs`). **Guest frontend**: `Process/device-<uid-short>-sk-frontend` (from `packages/d2b-sk-frontend/`; `executionRef: Guest/<vm>`; replaces `d2b-sk-frontend.service` guest systemd unit) | `device-security-key` | KEY FACT: current `ProcessRole::SecurityKeyFrontend` is a d2bd-internal async accept loop (`start_sk_accept_loop`), NOT a spawned process; in v3 the relay is extracted into a separate unprivileged relay Process; the guest frontend binary becomes a Zone Process resource with Guest execution context. |
| `Gpu` | `Process/device-<uid-short>-gpu` (full GPU; exclusive) | `device-gpu` | crosvm `device gpu`; Wayland socket, virtio-gpu |
| `GpuRenderNode` | `Process/device-<uid-short>-render-node` (render-node mode; exclusive default, shared when explicit) | `device-gpu` | Render-node-only mode; same broker tokens |
| `Video` | `Process/device-<uid-short>-video` | `device-gpu` | crosvm `device video-decoder --backend vaapi` |

No ProcessRole or Nix component is removed until the successor Provider/Process
integration is live and all current tests pass against the new resource model.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | **Process/DAG**: `packages/d2b-core/src/processes.rs` (ProcessRole enum: Swtpm, SwtpmPreStartFlush, Usbip, SecurityKeyFrontend, Gpu, GpuRenderNode, Video; VmProcessDag/VmProcessInvariants structs — old Workload DAG names); `packages/d2b-core/src/bundle_resolver.rs` (process exec names, device token sets, USBIP intents); `packages/d2b-host/src/swtpm_argv.rs`, `gpu_argv.rs`, `video_argv.rs`; **swtpm state**: `packages/d2b-priv-broker/src/ops/swtpm_dir.rs`; **Contracts/broker ops**: `packages/d2b-contracts/src/security_key.rs`, `usbip.rs`, `broker_wire.rs`; `packages/d2b-core/src/privileges_w3.rs` (W3BrokerOperation enum: SecurityKeyOpenDevice, SecurityKeyApplyUdevRules, UsbipBindFirewallRule); **Security-key relay (daemon-internal)**: `packages/d2bd/src/security_key.rs` (CTAPHID relay: CID translation, SO_PEERCRED auth, hidraw async fd, accept loop, lease; lives inside d2bd, NOT a separate spawned process); `packages/d2bd/src/lib.rs:10456` (`start_sk_accept_loop` — ProcessRole::SecurityKeyFrontend is handled as a daemon-internal coroutine: broker fetches hidraw fd, daemon binds vsock-proxy socket, spawns async accept loop); **Guest binary**: `packages/d2b-sk-frontend/src/` (static binary for in-guest UHID virtual HID device; connects over AF_VSOCK to the daemon accept loop; NOT related to the ProcessRole name); **USBIP state machine**: `packages/d2bd/src/usbip_state_machine.rs` (typed per-busid bring-up plan and executor; canonical step order: modprobe→lock→withhold→firewall→backend→bind→proxy); `packages/d2bd/src/usbip_reconcile_state.rs` (restart-safe reconciler state model; internal to daemon, not yet wired to reconciler); `packages/d2bd/src/usbipd_perenv_autostart.rs` (per-env usbipd daemon autostart via broker SpawnRunner, retiring legacy systemd units in `nixos-modules/network.nix`); **Workload/Realm capability surface (old names)**: `packages/d2b-realm-core/src/capability.rs` (old Realm Capability enum: GpuAccel, Usb, Hid, Hotplug — current inter-Realm device capability assertion, target maps to Device ResourceType claims); `packages/d2b-realm-core/src/stream.rs` (StreamKind::DeviceHid → Capability::Hid, StreamKind::DeviceUsb → Capability::Usb); `packages/d2bd/src/realm_access_resolver.rs` (maps old Workload ops: `ops.media.usb_hotplug` → Capability::Usb + Capability::Hotplug, `ops.display.graphics` → Capability::GpuAccel); **Workload manifest (old name)**: `packages/d2b-core/src/manifest_v04.rs` VmEntry fields: `tpm: bool`, `usbip_yubikey: bool`, `security_key: bool`, `graphics: bool`, `gpu_socket: Option<String>` (per-Workload device-enable flags in the v04 manifest; these are the current per-VM device declarations); **Runtime capability surface**: `packages/d2b-core/src/runtime.rs` (RuntimeServiceRole enum maps ProcessRoles to public roles: Tpm←Swtpm/SwtpmPreStartFlush, Display←Gpu/GpuRenderNode, Video←Video, Usb←Usbip+SecurityKeyFrontend; RuntimeMediaCapabilities: `usb_hotplug`; RuntimeDisplayCapabilities: `graphics`/`video`); **Nix options (old Workload namespace)**: `nixos-modules/options-realms-workloads.nix` (`d2b.vms.<vm>.tpm.enable`, `d2b.vms.<vm>.graphics.enable` — current Nix Workload device options; v3 target is `d2b.zones.<zone>.resources.<name> = { type = "Device"; ... }`); `nixos-modules/components/tpm.nix`, `usbip.nix`, `security-key-guest.nix`, `video/guest.nix`, `graphics.nix` |
| Evidence class | ProcessRole enum (ProcessRole, VmProcessDag): **implemented-and-reachable**. swtpm/gpu/video argv generators: **implemented-and-reachable**. swtpm_dir hardening and tamper marker: **implemented-and-reachable**. Security-key broker ops DTOs (`security_key.rs`, `broker_wire.rs` W3BrokerOperation): **implemented-and-reachable** (NOT unwired stubs — the full CTAPHID relay runs in `packages/d2bd/src/security_key.rs` and `packages/d2bd/src/lib.rs:start_sk_accept_loop`). USBIP state machine (`usbip_state_machine.rs`): **implemented-and-reachable**. USBIP reconcile state model (`usbip_reconcile_state.rs`): **implemented-but-unwired** (future restart-safe reconciler, internal state model). USBIP per-env autostart (`usbipd_perenv_autostart.rs`): **implemented-and-reachable**. Realm Capability enum (GpuAccel/Usb/Hid/Hotplug): **implemented-and-reachable** (old Workload/Realm names; in-process capability assertion). StreamKind::DeviceHid/DeviceUsb: **implemented-and-reachable**. realm_access_resolver.rs (Workload ops → Capabilities): **implemented-and-reachable**. manifest_v04.rs VmEntry device fields: **generated-or-eval-contract** (bundle/manifest-driven per-Workload flags). runtime.rs RuntimeServiceRole/RuntimeCapabilities: **implemented-and-reachable** (current public service role and capability surface). d2b-sk-frontend guest binary: **implemented-and-reachable** (guest static binary, not a Zone Process). Nix options-realms-workloads.nix device options: **generated-or-eval-contract**. Device ResourceType schema: **ADR-only**. Provider crates (d2b-provider-device-*): **ADR-only**. |
| Behavior retained | Swtpm user-namespace/zero-host-caps (ADR 0021), tamper-marker/fail-closed, umask=7 socket ACL; GPU broker token set (kvm/dri/udmabuf/nvidia*); video wire-contract constants frozen; USBIP bus ID validation; security-key hidraw-only broker access; eval-time mutual-exclusion assertions |
| Required delta | Device ResourceType schema, four Provider crates, controller reconcile loops, RBAC roles, hot-plug observe interval, Guest frontend Process resolution, consolidated process name templates |
| Reuse path | Extract swtpm_argv.rs, swtpm_dir.rs, gpu_argv.rs, video_argv.rs unmodified into device-tpm/device-gpu crates. Adapt security_key.rs DTOs with Zone ResourceRef identifiers. Adapt usbip.rs with v3 enum changes. Copy ProcessRole disposition table verbatim into Provider dossiers. |
| Replacement/deletion | ProcessRole enum variants are retained until Provider successor crates reach integration parity; Nix components retained until v3 Guest Nix emitters replace them |
| Feasibility proof | State-machine and protocol-conformance tests per Provider; physical-probe absence and hotplug condition transitions |
| Future owner | Work items below and per-Provider dossier specs |

## Current device capability surface (old Workload/Realm terminology)

The following baseline code uses old Workload/Realm terminology that maps to the
Device ResourceType and claims model in v3. Current evidence is cited by old
symbol names; v3 target names are in the "maps to" column.

| Current symbol | File | Class | v3 mapping |
| --- | --- | --- | --- |
| `Capability::GpuAccel` | `packages/d2b-realm-core/src/capability.rs` | implemented-and-reachable | `Device/<vm>-gpu` claim, `device-gpu` Provider |
| `Capability::Usb` | `packages/d2b-realm-core/src/capability.rs` | implemented-and-reachable | `Device/<vm>-usb` claim, `device-usbip` Provider |
| `Capability::Hid` | `packages/d2b-realm-core/src/capability.rs` | implemented-and-reachable | `Device/<vm>-security-key` claim, `device-security-key` Provider |
| `Capability::Hotplug` | `packages/d2b-realm-core/src/capability.rs` | implemented-and-reachable | Device controller `scheduled-observe` trigger / hotplug notification |
| `StreamKind::DeviceHid` | `packages/d2b-realm-core/src/stream.rs` | implemented-and-reachable | `device-security-key` hidraw relay stream |
| `StreamKind::DeviceUsb` | `packages/d2b-realm-core/src/stream.rs` | implemented-and-reachable | `device-usbip` USB export stream |
| `ops.display.graphics` → `Capability::GpuAccel` | `packages/d2bd/src/realm_access_resolver.rs:140` | implemented-and-reachable | GPU Device claim from Workload op flag (old Workload `graphics: bool`) |
| `ops.media.usb_hotplug` → `Capability::Usb + Capability::Hotplug` | `packages/d2bd/src/realm_access_resolver.rs:148` | implemented-and-reachable | USBIP Device claim from Workload op flag (old Workload `usb_hotplug: bool`) |
| `VmEntry::tpm: bool` | `packages/d2b-core/src/manifest_v04.rs` | generated-or-eval-contract | `Device/<vm>-tpm` spec; `device-tpm` Provider |
| `VmEntry::usbip_yubikey: bool` | `packages/d2b-core/src/manifest_v04.rs` | generated-or-eval-contract | `Device/<vm>-usb` spec; `device-usbip` Provider |
| `VmEntry::security_key: bool` | `packages/d2b-core/src/manifest_v04.rs` | generated-or-eval-contract | `Device/<vm>-security-key` spec; `device-security-key` Provider |
| `VmEntry::graphics: bool` | `packages/d2b-core/src/manifest_v04.rs` | generated-or-eval-contract | `Device/<vm>-gpu` spec; `device-gpu` Provider |
| `RuntimeServiceRole::Tpm` | `packages/d2b-core/src/runtime.rs` | implemented-and-reachable | Device `device-tpm` public service role |
| `RuntimeServiceRole::Usb` | `packages/d2b-core/src/runtime.rs` | implemented-and-reachable | Device `device-usbip` + `device-security-key` public service role |
| `RuntimeServiceRole::Display` | `packages/d2b-core/src/runtime.rs` | implemented-and-reachable | Device `device-gpu` (GPU) public service role |
| `RuntimeServiceRole::Video` | `packages/d2b-core/src/runtime.rs` | implemented-and-reachable | Device `device-gpu` (video) public service role |
| `RuntimeMediaCapabilities::usb_hotplug` | `packages/d2b-core/src/runtime.rs` | implemented-and-reachable | USBIP Device capability declaration in Workload runtime metadata |
| `RuntimeDisplayCapabilities::graphics` | `packages/d2b-core/src/runtime.rs` | implemented-and-reachable | GPU Device capability declaration in Workload runtime metadata |
| `d2b.vms.<vm>.tpm.enable` | `nixos-modules/options-realms-workloads.nix` | generated-or-eval-contract | v3: `d2b.zones.<zone>.resources."<vm>-tpm"` |
| `d2b.vms.<vm>.graphics.enable` | `nixos-modules/options-realms-workloads.nix` | generated-or-eval-contract | v3: `d2b.zones.<zone>.resources."<vm>-gpu"` |

The `Capability` enum in `packages/d2b-realm-core/src/capability.rs` is not
preserved as an enum in v3. Its values become Device ResourceType claims
(exclusive claim for Usb/Hid/GpuAccel) and Device controller observe-triggers
(Hotplug). The routing through `d2b-realm-router` uses the old Realm names; the
v3 Device controller replaces this routing.

## Provider crate layout

Every `packages/d2b-provider-<base>-<implementation>/` crate must contain all
four of the following paths. Missing any path fails the workspace/package policy
check (`make test-policy` / `cargo xtask check-provider-layout`):

| Path | Required contents |
| --- | --- |
| `src/` | Implementation: lib modules, binaries, controllers, workers, and services. Colocated unit tests as `#[cfg(test)]` modules within the source files they test. |
| `tests/` | Hermetic Cargo integration tests: ResourceType/spec serde round-trips, controller state-machine tests, conformance corpus against the ResourceTypeSchema and Provider settings schema, fault-injection and error-path tests. All tests run in-process with fake/stub dependencies; no container, privileged socket, or root privilege required. |
| `integration/` | Heavier fixtures and scenarios: container-level, Host/Guest-level, cross-process, and provider-system tests. Invoked by the existing test orchestration (`make test-integration` / Layer-2 container runner). Individual scripts or fixtures may be standalone but must be discoverable by the orchestrator without manual wiring. |
| `README.md` | Markdown document that covers all of the following topics: Provider identity (`Provider/<name>`), config schema and Nix authoring shape, ResourceTypes managed, controllers/services/workers/binaries and their roles, placement (Host/Guest/Zone), Provider dependencies and RBAC, security posture and state ownership, telemetry and audit attributes, build/test/integration commands, and future standalone-repo usage notes. |

The policy check is a `cargo xtask` command invoked by `make test-policy`. It
walks the workspace for crates matching the `d2b-provider-*` naming pattern and
asserts all four paths exist. The check fails closed: any crate matching the
pattern that is missing any required path fails the policy gate with a named
error listing the missing paths. There is no opt-out mechanism.

## Implementation work items

### ADR046-device-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0 shared contract root; `d2b-contracts` |
| Current source | `packages/d2b-contracts/src/security_key.rs` (SecurityKeyStatusResponse, SecurityKeySession, SecurityKeyLeaseState, SecurityKeyVmSessionState DTOs; implemented-and-reachable), `usbip.rs`, `broker_wire.rs`; `packages/d2b-core/src/privileges_w3.rs` (W3BrokerOperation: SecurityKeyOpenDevice, SecurityKeyApplyUdevRules, UsbipBindFirewallRule — implemented-and-reachable); `packages/d2b-core/src/manifest_v04.rs` VmEntry device fields (tpm, usbip_yubikey, security_key, graphics — old Workload manifest, generated-or-eval-contract) |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-contracts/src/v3/device.rs` |
| Detailed design | Device ResourceType schema (spec/status/conditions/claims/inventory); closed-set error codes; Device RBAC verbs; broker operation effect-limit constants |
| Integration | Provider dossiers, resource API/store, CLI status surfaces |
| Data migration | Full reset; no v2 device object import |
| Validation | Schema golden vectors; unknown-field denial; exclusive/shared conflict rejection; arbitration/maxClaims invariant |
| Removal proof | Old ProcessRole/DTO branches retained until Provider integrations are live |

### ADR046-device-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-device-001; device-tpm provider owner |
| Current source | `packages/d2b-host/src/swtpm_argv.rs`; `packages/d2b-priv-broker/src/ops/swtpm_dir.rs`; `nixos-modules/components/tpm.nix` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-device-tpm/src/` (controller, swtpm runner, state-dir logic); `packages/d2b-provider-device-tpm/tests/` (hermetic Cargo integration); `packages/d2b-provider-device-tpm/integration/` (container/Host scenarios); `packages/d2b-provider-device-tpm/README.md` |
| Detailed design | Device spec/status; flush EphemeralProcess → swtpm Process sequencing; state-dir hardening; tamper-marker; finalizer non-deletion of Volume; Nix emitter; all four required crate paths present (see "Provider crate layout") |
| Integration | Zone resource store; Process controller; Volume lifecycle |
| Data migration | State dir and tamper markers preserved across reset |
| Validation | `src/`: swtpm argv golden, state-dir, flush sequencing, finalizer no-delete; `tests/`: `controller_state_machine.rs`, `conformance.rs`, `fault_swtpm_missing.rs`; `integration/`: `provision_and_reboot/`, `tamper_marker_survives/`, `finalizer_no_delete/`; workspace policy check: `make test-policy` passes with all four paths present |
| Removal proof | ProcessRole::Swtpm and SwtpmPreStartFlush removed after parity |

### ADR046-device-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-device-001; device-usbip provider owner |
| Current source | `packages/d2b-contracts/src/usbip.rs` (USBIP DTOs, SYSFS_BUS_ID_MAX, bus-ID validation — implemented-and-reachable); `packages/d2b-core/src/bundle_resolver.rs` USBIP intents; `packages/d2b-core/src/privileges.rs` authz rows; `packages/d2bd/src/usbip_state_machine.rs` (typed per-busid bring-up state machine, step order: modprobe→lock→withhold→firewall→backend→bind→proxy — implemented-and-reachable); `packages/d2bd/src/usbipd_perenv_autostart.rs` (per-env usbipd daemon autostart — implemented-and-reachable); `packages/d2bd/src/usbip_reconcile_state.rs` (restart-safe reconciler state model — implemented-but-unwired); old Workload Nix option: `nixos-modules/options-realms-workloads.nix` `d2b.vms.<vm>.usbip.*` (generated-or-eval-contract); `nixos-modules/components/usbip.nix` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-device-usbip/src/` (controller, daemon Process, bind/unbind EphemeralProcess, firewall); `packages/d2b-provider-device-usbip/tests/` (hermetic Cargo integration); `packages/d2b-provider-device-usbip/integration/` (container/Host scenarios); `packages/d2b-provider-device-usbip/README.md` |
| Detailed design | Device spec/status; bus ID validation; firewall rule ownership-marker; bind/unbind EphemeralProcess; per-Device daemon Process (owned by device-usbip; Network supplies dependency/firewall interface); Nix emitter; all four required crate paths present (see "Provider crate layout") |
| Integration | Zone resource store; broker `UsbipBindFirewallRule`; nftables marker |
| Data migration | None; full reset |
| Validation | `src/`: bus ID corpus, firewall marker format, EphemeralProcess creation; `tests/`: `arbitration_conflict.rs`, `conformance.rs`, `firewall_marker.rs`, `explicit_attach_split.rs`; `integration/`: `arbitration_conflict/`, `busid_bind_cycle/`, `network_firewall_coexistence/`; workspace policy check: `make test-policy` passes with all four paths present |
| Removal proof | ProcessRole::Usbip removed after parity |

### ADR046-device-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-device-001; device-security-key provider owner |
| Current source | `packages/d2b-contracts/src/security_key.rs` (DTOs — implemented-and-reachable); `packages/d2b-core/src/privileges_w3.rs` (W3BrokerOperation — implemented-and-reachable); **KEY: relay is in d2bd** — `packages/d2bd/src/security_key.rs` (CTAPHID relay: CID translation, SO_PEERCRED, hidraw async fd, accept loop — implemented-and-reachable) and `packages/d2bd/src/lib.rs:start_sk_accept_loop` (ProcessRole::SecurityKeyFrontend dispatch — implemented-and-reachable); **guest binary**: `packages/d2b-sk-frontend/src/` (static UHID frontend — implemented-and-reachable); old Workload Nix option: `nixos-modules/options-realms-workloads.nix` `d2b.vms.<vm>.security_key.*`; `nixos-modules/components/security-key-guest.nix` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-device-security-key/src/` (controller, relay Process, guest frontend Process, lease/session ring); `packages/d2b-provider-device-security-key/tests/` (hermetic Cargo integration); `packages/d2b-provider-device-security-key/integration/` (container/Host/Guest scenarios); `packages/d2b-provider-device-security-key/README.md` |
| Detailed design | Device spec/status; unprivileged relay Process (`device-<uid-short>-sk-relay`); guest frontend Process (`device-<uid-short>-sk-frontend`, `executionRef: Guest/<vm>`); ceremony/CID/lease/session ring (max 1 session per Device); broker hidraw-only access; mutual-exclusion enforcement; Nix emitter; all four required crate paths present (see "Provider crate layout") |
| Integration | Zone resource store; broker `SecurityKeyOpenDevice`/`SecurityKeyApplyUdevRules`; Guest frontend module |
| Data migration | None; full reset |
| Validation | `src/`: lease transitions, session ring eviction, broker op path-free, CID round-trip; `tests/`: `lease_state_machine.rs`, `session_ring.rs`, `mutual_exclusion.rs`, `conformance.rs`, `guest_frontend_process.rs`; `integration/`: `lease_acquire_cancel/`, `session_ring_capacity/`, `guest_frontend_connect/`; workspace policy check: `make test-policy` passes with all four paths present |
| Removal proof | ProcessRole::SecurityKeyFrontend removed after parity |

### ADR046-device-005

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-device-001; device-gpu provider owner |
| Current source | `packages/d2b-host/src/gpu_argv.rs`, `video_argv.rs`; `packages/d2b-core/src/bundle_resolver.rs` Gpu/GpuRenderNode/Video; `nixos-modules/components/graphics.nix`, `video/guest.nix` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-device-gpu/src/` (controller, GPU/render-node/video worker Processes, broker token set); `packages/d2b-provider-device-gpu/tests/` (hermetic Cargo integration); `packages/d2b-provider-device-gpu/integration/` (container/Host/Guest scenarios); `packages/d2b-provider-device-gpu/README.md` |
| Detailed design | Combined Device spec/status; GPU worker Process (`device-<uid-short>-gpu`, exclusive); render-node Process (`device-<uid-short>-render-node`, exclusive default, shared when explicit); video-decoder Process (`device-<uid-short>-video`); broker token set; wire-contract constants; shared render-node arbitration enforcement; Nix emitter; all four required crate paths present (see "Provider crate layout") |
| Integration | Zone resource store; broker `SpawnRunner`/`OpenDevice`; Display Provider device consumption |
| Data migration | None; full reset |
| Validation | `src/`: process role selection, wire-constant snapshot, render-node vs full-GPU branching; `tests/`: `combined_reconcile.rs`, `render_node_enforcement.rs`, `wire_constant_snapshot.rs`, `conformance.rs`; `integration/`: `gpu_worker_start/`, `render_node_shared/`, `video_dependency/`; workspace policy check: `make test-policy` passes with all four paths present |
| Removal proof | ProcessRole::Gpu, GpuRenderNode, Video removed after parity |

### ADR046-device-006

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-device-001 through ADR046-device-005; Nix integrator |
| Current source | `nixos-modules/components/tpm.nix`, `usbip.nix`, `security-key-guest.nix`, `video/guest.nix`, `graphics.nix`; `nixos-modules/assertions.nix`; **old Workload Nix namespace**: `nixos-modules/options-realms-workloads.nix` (`d2b.vms.<vm>.tpm.enable`, `d2b.vms.<vm>.graphics.enable`, `d2b.vms.<vm>.usbip.*` — generated-or-eval-contract; v3 replaces `d2b.vms.*` with `d2b.zones.*`) |
| Reuse action | adapt |
| Destination | `nixos-modules/resources-device.nix`; `nixos-modules/bundle-artifacts.nix` (bundle emission for resource store); `nixos-modules/assertions.nix` (six eval-time device assertions) |
| Detailed design | Nix authoring shape `d2b.zones.<zone>.resources.<name> = { type = "Device"; metadata.ownerRef = ...; spec = { ...exact ResourceSpec fields... }; };` as specified in "Nix configuration" section; `metadata.name`/`metadata.zone`/`apiVersion` derived automatically; `status` and Core management fields (`managedBy`, `configurationGeneration`, uid, generation, revision, timestamps, finalizers) omitted from emitted JSON; `spec` field names/types/defaults identical to ResourceTypeSchema with no renaming; per-Provider `spec.settings` validated against signed Provider schema; no `artifactId` or store-path fields in Device `spec.settings` — binary paths are resolved from Provider package closures; Credential-ref enforcement; artifact catalog emitted as a separate private integrity-pinned map (ID→type/digest/closure) by its own emitter; six eval-time validation assertions; canonical sorted-key full resource-envelope JSON emission (`apiVersion`, `type`, `metadata`, `spec` only); Zone resource bundle with `contentDigest` as specified in "Zone resource bundle/generation" section; Core sets `metadata.managedBy=configuration` and `metadata.configurationGeneration` at activation |
| Integration | Resource store Nix emitter; artifact catalog emitted separately by Provider/system resource emitter (not by this emitter); device resource JSON output; Zone generation object including `priorGeneration`, `pendingDeletion`, `cleanupStatus` fields; cleanup contract logic belongs in Zone runtime (ADR-046-zone-lifecycle) consuming the generation from this emitter |
| Data migration | Consumers migrate from per-VM options to Zone Device declarations; data migration guide references "Nix configuration" section migration table |
| Validation | nix-unit: `device-tpm-eval.nix`, `device-usbip-eval.nix`, `device-security-key-eval.nix`, `device-gpu-eval.nix`, `device-schema-validation.nix`, `device-gen-cleanup-eval.nix`, `device-bundle-canonical.nix`, `device-inline-secret-rejected.nix`, `device-artifact-catalog.nix`; contract tests: `device_resource_schema.rs`, `device_bundle_canonical.rs`, `device_gen_cleanup.rs` |
| Removal proof | Nix option `d2b.vms.<vm>.tpm.enable` etc. retained until v3 reset |

### ADR046-device-007

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-device-006; Zone runtime implementer |
| Current source | None (new work; no equivalent in v3 baseline or main a1cc0b2d) |
| Reuse action | new |
| Destination | Zone runtime reconciliation loop (package TBD by ADR-046-zone-lifecycle); `packages/d2b-contract-tests/tests/device_gen_cleanup.rs` |
| Detailed design | Implement the cleanup contract described in "Zone generation and cleanup": (1) on new generation activation, diff `resources` against prior generation's `resources` by (type, name) — resources absent from new generation that have `managedBy=configuration` go into `pendingDeletion`; (2) Zone phase transitions to `Degraded/pending-cleanup` until all items in `pendingDeletion` reach terminal Delete; (3) items in `pendingDeletion` with `managedBy=controller` or `managedBy=api` are rejected with `cleanup-config-ownership-mismatch`; (4) `managedBy=controller` and `managedBy=api` resources are never touched by generation cleanup — `cleanup-controller-resource-protected` emitted if attempted; `managedBy=api` resources persist until explicit Delete; (5) prior generations retained by count: default 3, range 1..16, configured via `d2b.zones.<zone>.priorGenerationRetentionCount`; (6) each deletion is non-blocking; (7) finalizer-stuck timeout emits `cleanup-finalizer-stuck` and leaves Zone in `Degraded/pending-cleanup`; (8) all deletions emit `config-resource-deletion-requested` audit record with digested resource identity |
| Integration | Consumes Zone resource bundle from ADR046-device-006 emitter; drives Device Provider finalizers via normal resource-Delete protocol; feeds Zone status conditions `GenerationCleanPending` and `GenerationCleanError` |
| Validation | Layer-2: `device-gen-cleanup.sh`, `device-controller-resource-protected.sh`, `device-gen-cleanup-audit.sh`; feasibility proof: generation cleanup with fake Zone runtime |
| Removal proof | N/A (new) |

### ADR046-device-008

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-device-002 through ADR046-device-005; workspace/tooling maintainer |
| Current source | `packages/xtask/src/main.rs` (existing `gen-*` and `check-*` commands — implemented-and-reachable); `packages/d2b-contract-tests/tests/workspace_policy.rs` (existing workspace policy tests — evidence class TBD pending inspection) |
| Reuse action | extend |
| Destination | `packages/xtask/src/main.rs` (`check-provider-layout` subcommand); `packages/d2b-contract-tests/tests/workspace_policy.rs` (provider-layout policy assertions) |
| Detailed design | Add `cargo xtask check-provider-layout`: enumerate workspace members matching `d2b-provider-*`; for each, assert `src/`, `tests/`, `integration/`, and `README.md` all exist relative to the crate root; report all missing paths before failing; no opt-out flag. Add companion test in `workspace_policy.rs` that asserts the same invariant against the static crate list in `Cargo.toml`. Wire `cargo xtask check-provider-layout` into `make test-policy` alongside existing workspace naming and sort checks. The check must also run in CI as part of the policy gate. |
| Integration | `make test-policy`; `make check`; GitHub CI policy job |
| Data migration | N/A |
| Validation | Policy gate passes once all four Device Provider crates exist with required paths; gate fails predictably when any path is removed from a Provider crate fixture; test fixture crate with one missing path must produce a named error identifying the exact missing path |
| Removal proof | N/A (new tooling; not removed) |
