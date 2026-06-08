# `nixling status` output

Schema: [`status.schema.json`](./status.schema.json)

`nixling status <vm> --json` emits one object per VM. The JSON form is
intentionally narrower than the human form: it does **not** inline the
bridge-health table.

## Fields

| Field | Type | Semantics | Stability |
| --- | --- | --- | --- |
| `name` | string | Stable VM name. | Stable wire contract. |
| `services.nixling` | string | (pre-P6 historical) `systemctl is-active` state for `nixling@<vm>.service`. In v1.0 the lifecycle wrapper unit was retired; the daemon synthesises this field from the broker pidfd table runner state (`running` / `stopped` / `failed`). Operators reading the JSON should expect the same string values; the underlying source-of-truth changed in P6 (per ADR 0015). | Stable wire contract. |
| `services.microvm` | string | `systemctl is-active` state for `microvm@<vm>.service` (the upstream microvm.nix template, still emitted for direct-debug bypass in v1.0). | Stable wire contract. |
| `services.virtiofsd` | string | `systemctl is-active` state for `microvm-virtiofsd@<vm>.service`. | Stable wire contract. |
| `services.gpu` | string or `null` | GPU runner state. In v1.0 sourced from the broker pidfd table (pre-P6 it was the `nixling-<vm>-gpu.service` systemd unit, retired in P6). Present and `null` when `graphics = false`. | Stable wire contract. |
| `services.snd` | string or `null` | Audio runner state (in v1.0 from the broker pidfd table; pre-P6 systemd template retired in P6). Present and `null` when audio is disabled. | Stable wire contract. |
| `services.swtpm` | string or `null` | TPM runner state (in v1.0 from the broker pidfd table; pre-P6 systemd template retired in P6). Present and `null` when TPM is disabled. | Stable wire contract. |
| `current` | string or `null` | Target of `/var/lib/nixling/vms/<vm>/current`. | Stable wire contract. |
| `booted` | string or `null` | Target of `/var/lib/nixling/vms/<vm>/booted`. | Stable wire contract. |
| `pendingRestart` | boolean | True when the VM is running and `booted != current`. | Stable wire contract. |

## Ordering and null handling

- The emitter writes top-level keys in the order shown in the example,
  but consumers should key by name rather than rely on object-order.
- `gpu`, `snd`, `swtpm`, `current`, and `booted` are present and may be
  `null`.
- No other fields are omitted in W2.

## Stability promise

The top-level keys and service-subkeys are frozen for W2. New runtime
observability belongs in later negotiated protocol/schema revisions, not
as ad hoc extra keys on this object.

## Human example

```text
$ nixling status corp-vm
=== corp-vm ===
nixling@corp-vm: inactive
microvm@corp-vm (backend): inactive
virtiofsd: inactive
interactive: stopped
sshd@10.20.0.10: unreachable
pending-restart: no
```

## JSON example

```json
{
  "name": "corp-vm",
  "services": {
    "nixling": "inactive",
    "microvm": "inactive",
    "virtiofsd": "inactive",
    "gpu": null,
    "snd": null,
    "swtpm": null
  },
  "current": null,
  "booted": null,
  "pendingRestart": false
}
```
