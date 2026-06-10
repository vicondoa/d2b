# `nixling status` output

Schema: [`status.schema.json`](./status.schema.json)

`nixling status <vm> --json` emits one object per VM. The JSON form is
intentionally narrower than the human form: it does **not** inline the
bridge-health table.

## Fields

| Field | Type | Semantics | Stability |
| --- | --- | --- | --- |
| `name` | string | Stable VM name. | Stable wire contract. |
| `services.nixling` | string | `nixlingd.service` unit state. | Stable wire contract. |
| `services.microvm` | string | Cloud Hypervisor runner state from the daemon pidfd table (`ch-runner`). | Stable wire contract. |
| `services.virtiofsd` | string | Aggregate virtiofsd runner state from daemon pidfd roles prefixed `virtiofsd`. | Stable wire contract. |
| `services.gpu` | string or `null` | GPU runner state from the daemon pidfd table. Present and `null` when graphics is disabled. | Stable wire contract. |
| `services.video` | string or `null` | Video runner state from daemon pidfd table when the trusted bundle declares the `video` role. `null` means the video sidecar is not declared. | Stable wire contract. |
| `services.snd` | string or `null` | Audio runner state from the daemon pidfd table. Present and `null` when audio is disabled. | Stable wire contract. |
| `services.swtpm` | string or `null` | TPM runner state from the daemon pidfd table. Present and `null` when TPM is disabled. | Stable wire contract. |
| `current` | string or `null` | Target of `/var/lib/nixling/vms/<vm>/current`. | Stable wire contract. |
| `booted` | string or `null` | Target of `/var/lib/nixling/vms/<vm>/booted`. | Stable wire contract. |
| `pendingRestart` | boolean | True when the VM is running and `booted != current`. | Stable wire contract. |
| `declaredRoles` | array of strings | Process-DAG roles declared for the VM in the trusted bundle. Video-enabled VMs include `video`; graphics VMs without `graphics.videoSidecar` omit it. | Stable wire contract. |
| `readiness` | array of strings | Readiness predicates rendered as strings. Video-enabled VMs include `unix-socket-listening:/run/nixling-video/<vm>/video.sock`; graphics VMs with video disabled omit video readiness because the video sidecar is a default-off capability. | Stable wire contract. |
| `runtime` | string | Daemon runtime state label. | Stable wire contract. |
| `livePoolIntegrity` | object or omitted | Host-side integrity state for the ADR 0027 `store-view/live` pool: `status` is `ok`, `suspect`, or `unknown`; `unknownReason`, `auditRef`, `repairAttempted`, and `remediation` provide operator guidance when present. | Stable additive field. |

## Ordering and null handling

- The emitter writes top-level keys in the order shown in the example,
  but consumers should key by name rather than rely on object-order.
- `gpu`, `snd`, `swtpm`, `current`, and `booted` are present and may be
  `null`.
- Disabled optional components are omitted or rendered as `null`; they are not
  readiness failures.

## Stability promise

The top-level keys and service-subkeys are frozen. New runtime
observability belongs in later negotiated protocol/schema revisions, not
as ad hoc extra keys on this object.

## Human example

```text
$ nixling status corp-vm
=== corp-vm ===
env: work
runtime: unknown
nixling@corp-vm: active
microvm@corp-vm (backend): running
virtiofsd: running
interactive: stopped
ssh: declared
pending-restart: no
current: (missing)
booted: (missing)
declared roles: host-reconcile, store-virtiofs-preflight, gpu
```

## JSON example

```json
{
  "name": "corp-vm",
  "env": "work",
  "services": {
    "nixling": "active",
    "microvm": "running",
    "virtiofsd": "running",
    "gpu": "stopped",
    "video": null,
    "snd": null,
    "swtpm": null
  },
  "current": null,
  "booted": null,
  "pendingRestart": false,
  "runtime": "unknown",
  "declaredRoles": [
    "host-reconcile",
    "store-virtiofs-preflight",
    "gpu"
  ],
  "readiness": [],
  "livePoolIntegrity": {
    "status": "ok",
    "repairAttempted": false
  }
}
```
