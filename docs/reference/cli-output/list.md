# `nixling list` output

Schema: [`list.schema.json`](./list.schema.json)

`nixling list --json` emits one JSON array ordered lexicographically by
VM name. Each array element is a declared VM plus its current dispatch
status.

## Fields

| Field | Type | Semantics | Stability |
| --- | --- | --- | --- |
| `name` | string | Stable VM name. Unique within the manifest. | Stable wire contract. |
| `env` | string or `null` | Environment name. Present and `null` only when the VM has no environment binding. | Stable wire contract. |
| `graphics` | boolean | Whether the VM is a graphics VM. | Stable wire contract. |
| `tpm` | boolean | Whether the VM declares TPM support. | Stable wire contract. |
| `usbip` | boolean | Whether the VM declares USBIP/YubiKey support. | Stable wire contract. |
| `staticIp` | string or `null` | Declared static IPv4 address. Present and `null` for DHCP-backed shapes. | Stable wire contract. |
| `status` | string enum | One of `stopped`, `running`, `pending-restart`, `failed`, or `unknown`. | Stable wire contract. |
| `isNetVm` | boolean | True only for auto-declared per-env net VMs. | Stable wire contract. |

## Ordering and null handling

- The top-level array is ordered by `name`.
- No fields are omitted.
- `env` and `staticIp` are the only nullable fields.

## Stability promise

The field set and the five `status` enum values are part of the
compatibility contract. Human table spacing may change; the JSON shape
may not change without an intentional schema update.

## Human example

```text
$ nixling list
NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
corp-vm            work      false     false false   10.20.0.10      stopped
sys-work-net       work      false     false false   192.0.2.1       stopped (net-vm)
```

## JSON example

```json
[
  {
    "name": "corp-vm",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "usbip": false,
    "staticIp": "10.20.0.10",
    "status": "stopped",
    "isNetVm": false
  },
  {
    "name": "sys-work-net",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "usbip": false,
    "staticIp": "192.0.2.1",
    "status": "stopped",
    "isNetVm": true
  }
]
```
