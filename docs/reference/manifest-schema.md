# nixling JSON manifest schema

**Status:** current public manifest version is `manifestVersion = 5`.
**Source of truth:** [`manifest-schema.json`](./manifest-schema.json)
(JSON Schema Draft 2020-12). When this prose and the JSON Schema
disagree, the JSON Schema wins.

## What this is

`nixling` evaluates a typed per-VM manifest at Nix-evaluation time and
ships it as:

```text
/run/current-system/sw/share/nixling/vms.json
```

The Rust CLI, `nixlingd`, and `nixling-priv-broker` consume this public
inventory. Private bundle artifacts live beside it and are documented in
[`manifest-bundle.md`](./manifest-bundle.md).

## Top-level shape

```jsonc
{
  "_manifest": { "manifestVersion": 5 },
  "_observability": {
    "enabled": true,
    "vmName": "sys-obs",
    "obsVsockCid": 1000,
    "obsVsockHostSocket": "/var/lib/nixling/vms/sys-obs/vsock.sock",
    "signozUrl": "http://10.40.0.10:8080",
    "signozOtlpGrpcPort": 4317,
    "signozOtlpHttpPort": 4318
  },

  "<vm-name>": {
    "name": "work",
    "graphics": false,
    "tpm": false,
    "usbipYubikey": false,
    "audio": false,
    "tap": "nl-work",
    "bridge": "nl-work" | null,
    "env": "work" | null,
    "isNetVm": false,
    "netVm": "sys-work-net" | null,
    "usbipdHostIp": "10.50.0.1" | null,
    "stateDir": "/var/lib/nixling/vms/work",
    "apiSocket": "/var/lib/nixling/vms/work/work.sock",
    "gpuSocket": "/var/lib/nixling/vms/work/work-gpu.sock",
    "tpmSocket": "/run/nixling/vms/work/tpm.sock",
    "audioStateFile": "/var/lib/nixling/vms/work/state/audio-state.json",
    "audioService": "nixling-work-snd.service",
    "observability": {
      "enabled": true,
      "vsockCid": 110,
      "vsockHostSocket": "/var/lib/nixling/vms/work/vsock.sock",
      "agentSocket": "/run/nixling/otlp.sock"
    },
    "staticIp": "10.50.0.10" | null,
    "sshUser": "alice" | null
  }
}
```

Every top-level key is either a reserved key starting with `_` or a VM
name matching the VM-name assertion (`^[a-z][a-z0-9-]*$`). The leading
underscore prevents reserved keys from colliding with valid VM names.

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
| `vsockCid` | unsigned integer | yes | Deterministic base Cloud Hypervisor vsock CID for this VM, shared by observability and guest control. Env-backed VMs use `100 + envIndex * 1000 + slot` (slot 1 is reserved for the env net VM; workload VMs use their `nixling.vms.<vm>.index`). |
| `vsockHostSocket` | string | yes | Host-side Cloud Hypervisor vsock socket for this VM. |
| `agentSocket` | string | yes | Guest-local OTLP socket path used by the guest collector. |

The per-VM block is emitted for every VM so clients do not need to infer
transport paths from naming conventions.

## Compatibility policy

Consumers must reject manifests with a newer `manifestVersion` than they
support. The daemon and broker fail closed on mismatched bundle/manifest
versions rather than guessing compatibility.

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
