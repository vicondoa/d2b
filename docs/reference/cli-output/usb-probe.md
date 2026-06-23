# `nixling usb probe` output

Schema: [`usb-probe.schema.json`](./usb-probe.schema.json)

`nixling usb probe --json` emits one object with `command = "usb probe"`
and an `entries[]` array. The command is diagnostic: it asks `nixlingd`
to report declared USBIP session claims and qemu-media USB slots using redacted
physical-identity state labels. It may trigger bounded status
reconciliation (USBIP backend-ACL validation and qemu-media redacted
registry refresh), but it does not bind, unbind, import, detach, expose,
or release a USBIP device.

## Fields

| Field | Type | Semantics | Stability |
| --- | --- | --- | --- |
| `command` | string | Always `usb probe`. | Stable wire contract. |
| `entries[]` | array | One row per declared USBIP busid session claim, plus qemu-media physical-USB slot rows when declared. | Stable wire contract. |
| `entries[].kind` | string, omitted for USBIP | `usbip` for passthrough claims; `qemu-media-slot` for qemu-media physical USB rows. The default `usbip` kind may be omitted. | Stable additive field. |
| `entries[].vm` | string | VM that owns or can use the row. | Stable wire contract. |
| `entries[].env` | string | Environment for the VM, or `-` when no env applies. | Stable wire contract. |
| `entries[].busId` | string | Host USB busid for USBIP rows; `-` when no runtime busid is selected. | Stable wire contract. |
| `entries[].lockPath` | string | Broker-owned session claim path for USBIP rows. It is under `/run/nixling/locks/usbip`, so it is durable for the current host boot/session only. Empty for qemu-media rows. Treat this as diagnostics, not mutation authority. | Stable wire contract. |
| `entries[].status` | string | Overall row status: `bound`, `unbound`, `degraded`, `enrollable`, `enrolled`, `stale`, or `direct-config`. | Stable wire contract. |
| `entries[].ownerVm` | string or omitted | VM named by the session claim when observed. | Stable additive field. |
| `entries[].durableClaim.state` | string | Wire-compatible field name for the USBIP session claim state: `missing`, `held-by-desired-owner`, `held-by-other-owner`, `stale-owner`, `corrupt`, or `not-applicable`. The claim survives VM stop/restart and daemon restart, not host reboot. | Stable additive field. |
| `entries[].durableClaim.ownerVm` | string or omitted | Owner recorded inside the session claim. | Stable additive field. |
| `entries[].host.bind` | string | Host driver bind state: `unbound`, `bound-to-usbip-host`, `bound-to-unexpected-driver`, `device-missing`, `unknown`, or `not-applicable`. | Stable additive field. |
| `entries[].host.carrier` | string | Active carrier state for the `usbip-host` module, per-env backend/export readiness, and selected device presence: `absent`, `unavailable`, `withheld-for-owner`, `ready`, `departed-during-probe`, `unknown`, or `not-applicable`. | Stable additive field. |
| `entries[].host.proxy` | string | Per-env proxy listener state: `not-declared`, `stopped`, `starting`, `listening`, `stale`, `failed`, `unknown`, or `not-applicable`. | Stable additive field. |
| `entries[].guest.import` | string | Guest import observation through authenticated guest-control: `detached`, `imported`, `unavailable`, `unknown`, or `not-applicable`. | Stable additive field. |
| `entries[].topologyPolicy.topology` | string | Redacted physical topology match state: `match`, `mismatch`, `incomplete`, `not-observed`, `not-applicable`, or `unknown`. | Stable additive field. |
| `entries[].topologyPolicy.policy` | string | Bundle policy state: `allowed`, `denied`, `missing`, `not-applicable`, or `unknown`. | Stable additive field. |
| `entries[].degradedReasons[]` | array | Closed degraded reason objects with `code`, `summary`, and `remediation`. Raw sysfs paths, serials, command output, and stderr are not included. | Stable additive field. |
| `entries[].remediationCommands[]` | array of strings | Copy-pasteable lifecycle commands for this row, when a safe command exists. | Stable additive field. |
| `entries[].slot`, `mediaRef`, `sourceKind`, `candidateBusIds`, `followUpCommand` | optional | qemu-media physical USB slot metadata and next-step guidance. USBIP rows omit these fields. | Stable additive fields. |

## Status and degradation rules

- `bound` means the session claim is held by the desired VM and no degraded
  reason was observed.
- `unbound` means the declared USBIP row has no session owner and can be
  attached with `nixling usb attach <vm> <busid> --apply`.
- `degraded` means at least one session claim, active carrier, host bind,
  proxy, guest import, topology, or policy check did not converge.
- A session lock alone is not healthy. A row whose lock is
  `held-by-desired-owner` remains `degraded` until host bind/carrier/proxy
  and guest import state converge.
- VM restart reconciliation preserves same-VM session claims. During restart
  the daemon detaches guest imports and only runs host unbind after firewall
  withdrawal plus targeted stream cleanup is proven, then replays
  bind/proxy/import after guest-control readiness. Runtime absence or
  guest/proxy unavailability is reported as degraded USB state; required
  policy/topology failures fail before device exposure.

## Degraded reasons

| Code | Meaning | Default remediation |
| --- | --- | --- |
| `policy-failed` | USB policy does not allow this claim. | Fix the USBIP declaration or caller authorization, rebuild, and retry the lifecycle verb. |
| `device-departed-before-claim` | The device was absent before claiming. | Reconnect the physical device, wait for host observation, then run `nixling usb probe` or retry the lifecycle verb. |
| `device-departed-after-lock` | The device disappeared after broker claim acquisition. | Reconnect the device, then retry the probe or lifecycle verb. |
| `device-departed-during-mutation` | The device disappeared while host or guest state changed. | Reconnect the device, then retry the probe or lifecycle verb. |
| `device-reappeared-with-different-topology` | A different device appeared at the expected location. | Verify the physical device identity, update the declaration if intentional, rebuild, and retry after the probe is stable. |
| `lock-held-by-other-owner` | Another VM or env holds the session claim. | Run `nixling usb detach <owner> <busid> --apply` before attaching this VM. |
| `invalid-persisted-lock-claim` | The broker-mediated claim is stale or corrupt. | Confirm no active owner uses the device, then run the USB detach/reconcile path; remove only broker-owned stale claim state. |
| `carrier-unavailable` | The `usbip-host` module, selected device, or per-env backend/export carrier is unavailable. | Reconnect the device and run `nixling usb attach <vm> <busid> --apply`. |
| `host-bind-unavailable` | The device is not bound to `usbip-host`. | Run `nixling usb attach <vm> <busid> --apply` so the broker can bind it for export. |
| `proxy-unavailable` | The per-env proxy is missing, stale, failed, or not listening. | Reconcile the proxy by running `nixling usb attach <vm> <busid> --apply`; inspect broker audit if it repeats. |
| `guest-import-unavailable` | The guest has not imported the claimed device or guest-control is unavailable. | Start the VM if needed, then run `nixling usb attach <vm> <busid> --apply`. |
| `stale-host-state` | Host export/proxy state remains after the claim was removed. | Run `nixling usb detach <vm> <busid> --apply` to drain host state. |
| `stale-guest-state` | Guest import state remains after the claim was removed. | Run `nixling usb detach <vm> <busid> --apply` so guestd removes the import. |
| `probe-incomplete` | Probe did not produce a reconciliation-safe identity. | Retry `nixling usb probe`; if it repeats, verify the declaration has a stable physical selector. |

## JSON example

```json
{
  "command": "usb probe",
  "entries": [
    {
      "vm": "corp-vm",
      "env": "work",
      "busId": "1-2",
      "lockPath": "/run/nixling/locks/usbip/1-2",
      "status": "degraded",
      "ownerVm": "corp-vm",
      "durableClaim": {
        "state": "held-by-desired-owner",
        "ownerVm": "corp-vm"
      },
      "host": {
        "bind": "unknown",
        "carrier": "unknown",
        "proxy": "unknown"
      },
      "guest": {
        "import": "detached"
      },
      "topologyPolicy": {
        "topology": "unknown",
        "policy": "allowed"
      },
      "degradedReasons": [
        {
          "code": "guest-import-unavailable",
          "summary": "the guest USBIP import has not converged",
          "remediation": "Run `nixling usb attach corp-vm 1-2 --apply` after the VM is running."
        }
      ],
      "remediationCommands": [
        "nixling usb attach corp-vm 1-2 --apply"
      ]
    }
  ]
}
```

## Null and omission handling

Optional fields are omitted when unknown or not applicable. Consumers should
key by field name and tolerate added optional fields. Empty
`degradedReasons`, `remediationCommands`, and `candidateBusIds` arrays may be
omitted.
