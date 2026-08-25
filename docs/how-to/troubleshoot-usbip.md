# Troubleshoot USBIP passthrough

Use this runbook when a declared USBIP/YubiKey passthrough does not attach,
when a USB row is degraded, or when a claimed device does not reappear after a
VM restart. For field-level contracts, see
[`components-usbip.md`](../reference/components-usbip.md) and
[`usb-probe.md`](../reference/cli-output/usb-probe.md).

> **Current U10 limitation:** the signed target-local USBIP Process and its
> authenticated ComponentSession effect adapter are not wired into `d2bd`.
> Non-qemu attach, detach, and VM-start USBIP reconciliation therefore fail
> closed before host mutation. Probe remains read-only and reports Guest import
> as unavailable; a host claim alone is not a successful attachment.

## Before you start

Run the commands as a d2b admin: either a user in the `d2b` lifecycle
group or via `sudo`, depending on how the host was configured. Do not edit
`/run/d2b/locks/usbip/<busid>`, sysfs driver links, nftables rules, or
per-env proxy processes directly; the daemon and broker own those surfaces.

## 1. Confirm the declaration

USBIP needs both host and guest configuration.

Host prerequisites:

- `d2b.site.yubikey.enable = true`.
- The target VM is enabled, belongs to an env, and sets
  `d2b.vms.<vm>.usbip.yubikey = true`.
- The VM declares the allowed busids with `d2b.vms.<vm>.usbip.busids`.
- If `d2b.host.usbip.allowlist` is set, the plugged device's VID/PID must
  match an allowlist entry.

Guest prerequisites are provided by `usbip.yubikey = true`: the guest loads
`vhci_hcd` and includes the guest `usbip` tool. The target-local Process and
authenticated ComponentSession USBIP status/import operations remain a future
signed Process contract.

Example host snippet:

```nix
{
  d2b.site.yubikey.enable = true;

  d2b.host.usbip.allowlist = [
    { vendor = "0x1050"; product = "0x0407"; }
  ];

  d2b.vms.corp-vm = {
    env = "work";
    usbip.yubikey = true;
    usbip.busids = [ "1-2" ];
  };
}
```

After changing host configuration, switch the host and restart the daemon so it
reloads the generated bundle:

```bash
sudo nixos-rebuild switch --flake .#desktop
sudo systemctl restart d2bd.service
```

## 2. Inspect USB status

Start with the read-only probe:

```bash
d2b device usb probe
```

For one Guest, `d2b guest status <name>` also includes the USB summary:

```bash
d2b guest status corp-vm
```

Read each USBIP row by layer:

- `SESSION-CLAIM` in `d2b device usb probe` (or `session-claim=` in
  `d2b guest status`) is the broker claim for the current host boot/session.
  `held-by-desired-owner` means the VM owns the busid, but that alone is not
  healthy. It survives VM stop/restart and daemon restart, not host reboot,
  because the backing lock lives under `/run/d2b/locks/usbip`.
- `HOST-BIND`, `CARRIER`, and `PROXY` are active host carrier state.
- `GUEST` is the in-guest import state.
- `POLICY` / topology fields explain whether the physical device still matches
  the declaration.
- `degraded` lines explain why the row is not healthy.
- `command:` lines are the safe copy-paste remediation commands.

A healthy row is `bound` with the desired session claim and converged host,
proxy, and guest state. A same-VM session claim after restart is expected to
show as `degraded` until active carrier state is replayed.

## 3. Check attach availability

Use the busid from the declaration/probe when validating the declaration. The
current non-qemu apply path is unavailable and does not bind the host device:

```bash
d2b device usb attach corp-vm 1-2 --apply
```

Expect a typed `runtime-capability-unsupported` error until the signed
target-local USBIP Process path is shipped.

## 4. Recover a same-VM session claim after restart

VM stop/restart preserves same-VM USBIP session claims within the current host
boot/session. Until Guest detach/import is available, d2b does not replay host
bind/proxy state and preserves the claim instead. After a host reboot,
`/run/d2b/locks/usbip` is recreated empty. Use probe to inspect the observed
state:

```bash
d2b device usb probe
```

Do not release a session claim just because the active carrier is down; the
current daemon cannot safely perform Guest detach or host release.

## 5. Release a claim

The current non-qemu detach path is unavailable and preserves the claim:

```bash
d2b device usb detach corp-vm 1-2 --apply
d2b device usb probe
```

Expect a typed `runtime-capability-unsupported` error. Do not edit the lock,
unbind the host device, or stop a shared proxy manually; release is deferred
until Guest detach can be authenticated and drained.

## Common troubleshooting

Until the U10 limitation is removed, remediation commands that attach or
detach non-qemu USBIP are prospective only and return
`runtime-capability-unsupported`; they must not be replaced with manual host
USBIP, lock, firewall, or proxy mutations.

| Symptom from `d2b device usb probe` or `d2b guest status` | What it means | Remediation |
| --- | --- | --- |
| `status=unbound` or `SESSION-CLAIM=missing` | No session owner exists for the declared busid. | `d2b device usb attach corp-vm 1-2 --apply` |
| Attach says the VM is stopped or `guest-import-unavailable` | The target-local USBIP Process cannot import until the Guest is running. | `d2b guest start corp-vm --apply`, then `d2b device usb attach corp-vm 1-2 --apply` |
| `SESSION-CLAIM=held-by-desired-owner`, `HOST-BIND=bound-to-usbip-host`, and `GUEST=detached` | The host owns and exports the device for this Guest, but target-local Process has not imported it. | Re-run the row's `d2b device usb attach <name> <busid> --apply` command; it converges guest import without releasing the session claim. |
| `SESSION-CLAIM=held-by-other-owner` / `lock-held-by-other-owner` | Another Guest owns the session claim. | `d2b device usb detach <owner> 1-2 --apply`, then `d2b device usb attach corp-vm 1-2 --apply` |
| `SESSION-CLAIM=stale-owner`, `SESSION-CLAIM=corrupt`, or `invalid-persisted-lock-claim` | The session claim cannot be safely trusted as a healthy owner. | Do not edit the lock file. Run the probe's `command:` line if present; otherwise `d2b device usb detach corp-vm 1-2 --apply`, then `d2b device usb probe`. |
| `HOST-BIND=bound-to-unexpected-driver` | The device is present but still owned by another host driver or local application. | Close local consumers of the device, then `d2b device usb attach corp-vm 1-2 --apply`. |
| `carrier-unavailable`, `host-bind-unavailable`, or a departed-device reason | The physical device, `usbip-host` carrier, or host bind state is not present. | Reconnect the device, run `d2b device usb probe`, then `d2b device usb attach corp-vm 1-2 --apply` |
| `proxy-unavailable` | The per-env proxy/backend carrier is not listening or is stale. | `d2b device usb attach corp-vm 1-2 --apply`; if it repeats, inspect `/var/lib/d2b/audit/broker-<utc-date>.jsonl` for the USBIP runner/broker failure. |
| `policy-failed`, `device-reappeared-with-different-topology`, or topology mismatch | The plugged device does not match the declaration or allowlist. | Fix `usbip.busids` / `d2b.host.usbip.allowlist` or plug the approved device, then `sudo nixos-rebuild switch --flake .#desktop`, `sudo systemctl restart d2bd.service`, and retry `d2b device usb probe`. |
| `stale-host-state` or `stale-guest-state` after a removed claim | Carrier/import state outlived the session claim. | `d2b device usb detach corp-vm 1-2 --apply`, then `d2b device usb probe` |
| `probe-incomplete` | The daemon could not observe enough redacted identity to safely reconcile. | Retry `d2b device usb probe`; if it repeats, fix the declaration or physical topology and rebuild before attaching. |

When the CLI prints a `command:` line, prefer that exact command over the table
above; it was computed from the row's current owner and busid.
