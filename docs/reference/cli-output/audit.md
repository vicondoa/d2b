# `d2b audit` output

Schema: [`audit.schema.json`](./audit.schema.json)

`d2b audit --json` emits one object containing the read-only
security/posture report. Top-level key order is deterministic, but
consumers should key by name rather than rely on object-order.

## Top-level fields

| Field | Type | Meaning | Stability |
| --- | --- | --- | --- |
| `kvm_dev_mode` | string | File mode of `/dev/kvm` as observed by the audit. | Stable wire contract. |
| `wayland_user_in_kvm` | boolean | Whether the configured Wayland user is in the `kvm` group. | Stable wire contract. |
| `store_delivery` | object | Per-VM store-delivery mode (`virtiofs`, `erofs`, or `UNKNOWN`). | Stable top-level key; nested VM map may grow with declared VMs. |
| `virtiofsd` | object | Runtime virtiofsd facts keyed by VM. | Stable top-level key; nested per-VM detail may grow additively. |
| `ssh` | object | Host and per-VM SSH posture facts. | Stable top-level key; nested detail may grow additively. |
| `bridge_isolation` | object | Runtime bridge/tap isolation facts keyed by VM. | Stable top-level key; nested detail may grow additively. |
| `autoUpgrade_commits_lock` | boolean | Whether the host flake appears to use `--commit-lock-file`. | Stable wire contract. |
| `ch_version` | string | Cloud Hypervisor version baked into the evaluated host. | Stable wire contract. |
| `crosvm_rev` | string | Crosvm revision identifier. | Stable wire contract. |
| `seccomp_rev` | string | Seccomp policy revision identifier. | Stable wire contract. |
| `ch_crosvm_pair_ok` | boolean | Whether the pinned CH/crosvm pair matches the audited pairing. | Stable wire contract. |
| `fail2ban_active` | boolean | Whether `fail2ban` is active on the host. | Stable wire contract. |
| `sidecars_per_vm` | object | Runtime sidecar facts keyed by VM. | Stable top-level key; nested detail may grow additively. |
| `usbipd_per_env_isolation` | object | Per-env USBIPd isolation facts. | Stable top-level key; nested detail may grow additively. |

## Null-vs-omitted behavior

- The top-level object always contains every field above.
- Nested per-VM/per-env maps may be empty when nothing is running.
- Later minor releases may add nested properties inside existing maps,
  but they should not remove or rename the top-level keys.

## Stability promise

The **top-level key set** is stable. Nested objects are also
contracted, but additive nested fields are the preferred compatibility
path for later minor releases.

## Human example

```text
$ d2b audit --human

=== d2b security audit ===

  kvm_dev_mode:                            660 ✓
  wayland_user_in_kvm:                     false ✓

  store_delivery:
    corp-vm: virtiofs
    sys-work-net: erofs
```

## JSON example

```json
{
  "kvm_dev_mode": "660",
  "wayland_user_in_kvm": false,
  "store_delivery": {
    "corp-vm": "virtiofs",
    "sys-work-net": "erofs"
  },
  "virtiofsd": {},
  "ssh": {
    "host": {
      "PasswordAuthentication": false
    },
    "corp-vm": {
      "PasswordAuthentication": false
    }
  },
  "bridge_isolation": {},
  "autoUpgrade_commits_lock": false,
  "ch_version": "52.0",
  "crosvm_rev": "deadbeefdead",
  "seccomp_rev": "feedfacefeed",
  "ch_crosvm_pair_ok": true,
  "fail2ban_active": true,
  "sidecars_per_vm": {},
  "usbipd_per_env_isolation": {}
}
```
