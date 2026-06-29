# d2b JSON manifest schema

**Status:** current public manifest version is `manifestVersion = 7`.
**Source of truth:** [`manifest-schema.json`](./manifest-schema.json)
(JSON Schema Draft 2020-12). When this prose and the JSON Schema
disagree, the JSON Schema wins.

## What this is

`d2b` evaluates a typed per-VM manifest at Nix-evaluation time and
ships it as:

```text
/run/current-system/sw/share/d2b/vms.json
```

The Rust CLI, `d2bd`, and `d2b-priv-broker` consume this public
inventory. Private bundle artifacts live beside it and are documented in
[`manifest-bundle.md`](./manifest-bundle.md).

## Top-level shape

```jsonc
{
  "_manifest": { "manifestVersion": 7 },
  "_observability": {
    "enabled": true,
    "vmName": "sys-obs",
    "obsVsockCid": 1000,
    "obsVsockHostSocket": "/var/lib/d2b/vms/sys-obs/vsock.sock",
    "signozUrl": "http://10.40.0.10:8080",
    "signozOtlpGrpcPort": 4317,
    "signozOtlpHttpPort": 4318
  },

  "<vm-name>": {
    "name": "work",
    "runtime": {
      "kind": "nixos",
      "provider": {
        "id": "local-cloud-hypervisor",
        "type": "local",
        "driver": "cloud-hypervisor"
      },
      "capabilities": {
        "lifecycle": true,
        "display": true,
        "usbHotplug": true,
        "guestControl": true,
        "exec": true,
        "configSync": true,
        "ssh": true,
        "storeSync": true,
        "keys": true,
        "inGuestObservability": true
      },
      "operationCapabilities": {
         "lifecycle": {
           "start": true,
           "stop": true,
           "restart": true,
           "switch": true,
           "hostPrepare": true
         },
         "media": {
           "usbHotplug": true,
           "removableMedia": false,
           "qemuMedia": false
         },
         "display": {
           "display": true,
           "graphics": true,
           "video": false,
           "waylandProxy": true
         },
         "guest": {
           "guestControl": true,
           "exec": true,
           "shell": true,
           "configSync": true,
           "ssh": true,
           "keys": true,
           "inGuestObservability": true
         },
         "storage": {
           "storeSync": true,
           "virtiofs": true,
           "volumes": false
         }
      },
      "autostartPolicy": "manual-only",
      "services": [
         { "id": "cloud-hypervisor", "role": "hypervisor", "optional": false }
      ]
    },
    "lifecycle": {
     "gracefulShutdown": {
       "enable": true,
       "timeoutSeconds": null
     },
     "liveActivation": {
       "timeoutSeconds": null
     }
    },
    "graphics": false,
    "tpm": false,
    "usbipYubikey": false,
    "audio": false,
    "tap": "d2b-work",
    "bridge": "d2b-work" | null,
    "env": "work" | null,
    "isNetVm": false,
    "netVm": "sys-work-net" | null,
    "usbipdHostIp": "10.50.0.1" | null,
    "stateDir": "/var/lib/d2b/vms/work",
    "apiSocket": "/var/lib/d2b/vms/work/work.sock",
    "gpuSocket": "/var/lib/d2b/vms/work/work-gpu.sock",
    "tpmSocket": "/run/d2b/vms/work/tpm.sock",
    "audioStateFile": "/var/lib/d2b/vms/work/state/audio-state.json",
    "audioService": null,
    "observability": {
      "enabled": true,
      "vsockCid": 110,
      "vsockHostSocket": "/var/lib/d2b/vms/work/vsock.sock",
      "agentSocket": "/run/d2b/otlp.sock"
    },
    "shell": {
      "enabled": true,
      "defaultName": "default",
      "maxSessions": 8,
      "maxAttached": 1
    },
    "staticIp": "10.50.0.10" | null,
    "sshUser": "alice" | null
  }
}
```

Every top-level key is either a reserved key starting with `_` or a VM
name matching the VM-name assertion (`^[a-z][a-z0-9-]*$`). The leading
underscore prevents reserved keys from colliding with valid VM names.

## Per-VM entry

Every non-reserved top-level key is a VM name mapping to the per-VM entry
described below. The JSON Schema (`manifest-schema.json`, `$defs.vmEntry`)
is the canonical type spec; this table is its human-readable companion.
Fields are listed in `nixos-modules/manifest.nix` declaration order.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | yes | VM name; matches the enclosing top-level key. Pattern `^[a-z][a-z0-9-]*$` (enforced by `nixos-modules/assertions.nix`). |
| `runtime` | object | yes | Runtime/provider metadata and provider support matrix. Shape: `{ kind, provider: { id, type, driver }, capabilities, operationCapabilities, autostartPolicy, services }`. `operationCapabilities` groups positive operation support by lifecycle/media/display/guest/storage axis; `operationCapabilities.guest.shell` records provider support for the staged persistent-shell operation. `services[]` contains bounded provider-neutral service summaries. `qemu-media` uses provider `local-qemu-media`/driver `qemu`; its supported capabilities are lifecycle/display/USB hotplug, while guest-control, exec, shell, config-sync, SSH, store-sync, keys, and in-guest observability are unsupported. |
| `lifecycle` | object | yes | Per-VM lifecycle policy. Shape: `{ gracefulShutdown: { enable, timeoutSeconds }, liveActivation: { timeoutSeconds } }`. `gracefulShutdown` controls provider-aware guest shutdown before forced VMM cleanup; its timeout is a nullable 1–600 second per-VM override. `liveActivation.timeoutSeconds` is a nullable 1–3600 second per-VM override for in-guest `switch`/`test`/`rollback`; `null` means the daemon default from `/etc/d2b/daemon-config.json` applies. |
| `graphics` | boolean | yes | Mirror of `d2b.vms.<name>.graphics.enable`. The CLI uses it to pick the launch path. |
| `tpm` | boolean | yes | Mirror of `d2b.vms.<name>.tpm.enable`. |
| `usbipYubikey` | boolean | yes | Mirror of `d2b.vms.<name>.usbip.yubikey`. `d2b usb attach\|detach\|probe` refuses to run when false. |
| `audio` | boolean | yes | Mirror of `d2b.vms.<name>.audio.enable` (the capability bit). Live grant state lives in `audioStateFile`. |
| `tap` | string | yes | Host-side tap-device name. Derived: `<env>-l<index>` (workload), `<env>-u2` (net VM), or `vm-<name>` (legacy). |
| `bridge` | string \| null | yes | Linux bridge the tap attaches to. Workload: `br-<env>-lan`. Net VM: `br-<env>-up`. Legacy hand-rolled VM: `null`. |
| `env` | string \| null | yes | Env this VM belongs to (workload) or serves (net VM). Null for legacy hand-rolled VMs. |
| `mtu` | integer | no | Effective MTU for env-backed VMs when emitted. Omitted for legacy/env-less VMs. |
| `mssClamp` | integer | no | Effective TCP MSS clamp for env-backed VMs when emitted. Omitted when no clamp is configured. |
| `lan` | object | no | LAN east-west policy metadata for env-backed VMs when emitted. Shape: `{ allowEastWest, effectiveEastWest }`. |
| `isNetVm` | boolean | yes | True iff this VM is the auto-generated `sys-<env>-net`. Used for bring-up ordering. |
| `netVm` | string \| null | yes | For workload VMs: name of the net VM serving this VM's env. Null for net VMs and legacy VMs. |
| `usbipdHostIp` | string \| null | yes | Host IP of the per-env usbipd proxy, passed to `usbip attach -r` via the broker. Null for net VMs and legacy. |
| `stateDir` | string | yes | Per-VM state dir. Currently `/var/lib/d2b/vms/<name>`. |
| `apiSocket` | string \| null | yes | Cloud Hypervisor runner API socket path (`<stateDir>/<name>.sock`). Null for providers without a CH API socket. |
| `gpuSocket` | string \| null | yes | GPU sidecar control socket (`<stateDir>/<name>-gpu.sock`). Null for providers without the d2b GPU sidecar socket. |
| `tpmSocket` | string \| null | yes | swtpm vTPM socket (`/run/d2b/vms/<name>/tpm.sock`). Null for providers without d2b-managed TPM state. |
| `audioStateFile` | string \| null | yes | Live audio-grant state file (`<stateDir>/state/audio-state.json`). Null for providers without the d2b audio sidecar. |
| `audioService` | string \| null | yes | Retired field. Always null; the audio sidecar is broker-spawned through the daemon DAG. |
| `observability` | object | yes | Per-VM observability transport metadata (`enabled`, base `vsockCid`/`vsockHostSocket`, guest `agentSocket`). See [Per-VM observability block](#per-vm-observability-block). |
| `shell` | object \| null | yes | Persistent guest shell policy metadata for providers that support the authenticated guest-control terminal substrate. Null for providers without d2b guest-control. Shape: `{ enabled, defaultName, maxSessions, maxAttached }`; `defaultName` matches `^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$`, `maxSessions` is 1–256, and `maxAttached` is 1–64. This is policy/capability metadata only; runtime helper sockets, shpool state, terminal handles, and session names beyond the configured default are never included in the world-readable manifest. |
| `staticIp` | string \| null | yes | The VM's static LAN IP. Derived for env-attached VMs; null when no IP source applies. |
| `sshUser` | string \| null | yes | Username for `d2b`-driven SSH. Mirrors `d2b.vms.<name>.ssh.user`. Null for headless net VMs. |

## Reserved keys

### `_manifest`

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `manifestVersion` | unsigned integer | yes | Schema version. Bumped on every breaking shape or semantic change. |

Version history:

- v1: initial VM inventory.
- v2: observability metadata.
- v3: daemon-only clean break.
- v4: native SigNoz observability metadata. Replaces the old
  Grafana/Cloud-Hypervisor-exporter metadata with SigNoz UI and OTLP
  collector metadata while preserving vsock transport fields.
- v5: combines the v4 SigNoz observability metadata with base Cloud
  Hypervisor vsock semantics — the per-VM `observability.vsockCid` /
  `observability.vsockHostSocket` fields define the host-owned base
  Cloud Hypervisor vsock device shared by observability and guest
  control, not only the observability relay. These two changes each
  landed as a `4` on separate branches and are unified at `5`.
- v6: adds per-VM runtime/provider metadata and provider capability
  summaries. Provider-specific socket/vsock fields are now nullable so
  `qemu-media` entries do not fabricate Cloud Hypervisor, guest-control,
  SSH, store-sync, key, or in-guest-observability artifacts.
- v6 additive: adds per-VM nullable `shell` policy metadata and
  `runtime.operationCapabilities.guest.shell`. This is additive and does not bump
  `manifestVersion`.
- v7: adds required per-VM `lifecycle.gracefulShutdown` metadata for
  provider-aware guest shutdown policy. The daemon accepts v6 manifests during
  the rollout by treating missing lifecycle metadata as `{ enable = true,
  timeoutSeconds = null }` for supported local providers.

### `_observability`

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | yes | Whether framework observability is enabled. |
| `vmName` | string | yes | Auto-declared observability VM name. Default: `sys-obs`. |
| `obsVsockCid` | unsigned integer | yes | CID assigned to the observability VM. |
| `obsVsockHostSocket` | string | yes | Host-side Cloud Hypervisor vsock socket for the obs VM. |
| `signozUrl` | string | yes | Resolved SigNoz UI URL. |
| `signozOtlpGrpcPort` | unsigned integer | yes | Loopback OTLP gRPC port inside `sys-obs`. |
| `signozOtlpHttpPort` | unsigned integer | yes | Loopback OTLP HTTP port inside `sys-obs`. |

The manifest intentionally keeps the transport metadata
(`obsVsockCid`, `obsVsockHostSocket`) separate from the UI/collector
metadata so daemon readiness and broker transport can continue to reason
about the vsock path without knowing SigNoz internals.

## Per-VM observability block

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | yes | Whether telemetry collection is enabled for this VM. |
| `vsockCid` | unsigned integer \| null | yes | Deterministic base Cloud Hypervisor vsock CID for nixos/Cloud Hypervisor VMs. Null for providers without d2b guest-control or in-guest observability. |
| `vsockHostSocket` | string \| null | yes | Host-side Cloud Hypervisor vsock socket for this VM. Null for providers without d2b guest-control or in-guest observability. |
| `agentSocket` | string \| null | yes | Guest-local OTLP socket path used by the guest collector. Null for providers without in-guest observability. |

The per-VM block is emitted for every VM so clients do not need to infer
transport paths from naming conventions.

## Compatibility policy

Consumers must reject manifests with a newer `manifestVersion` than they
support. The daemon and broker normally fail closed on mismatched
bundle/manifest versions rather than guessing compatibility.

The v6→v7 transition has one explicit compatibility window: new daemons may
load a v6 manifest that lacks `lifecycle.gracefulShutdown` and synthesize the
same default policy the v7 Nix emitter writes (`enable = true` for supported
local Cloud Hypervisor and qemu-media providers, `timeoutSeconds = null`).
This is only for running systems that have switched the daemon before the host
manifest is refreshed; newly rendered manifests use v7.

Adding optional fields without changing semantics can remain within the
same manifest version only when all consumers tolerate the field.
Removing fields, renaming fields, changing requiredness, or changing the
meaning of an existing field requires a manifest version bump and updated
fixtures, generated schemas, docs, and CHANGELOG entries in the same
change.

## Regeneration

When the Rust DTOs change, regenerate schema artifacts with:

```bash
cd packages
cargo run -p xtask -- gen-schemas
```

Then run the manifest parity and bundle drift gates so the Nix emitter,
Rust DTOs, schemas, and compact JSON fixtures stay byte-aligned.
