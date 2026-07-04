# Use the USB security-key proxy for a VM

This guide explains how to configure the d2b CTAP/WebAuthn security-key
proxy so that one or more VMs can use a host-attached security key (YubiKey
or any FIDO2 device) for browser-based WebAuthn authentication without USB
device ownership transfer.

For background on why this is not USB passthrough and how the proxy compares
to USBIP and Qubes, see
[`../explanation/usb-security-key-architecture.md`](../explanation/usb-security-key-architecture.md).

For the complete option reference, see
[`../reference/components-usb-security-key.md`](../reference/components-usb-security-key.md).

## What this achieves

When the security-key proxy is active for a VM:

- The VM sees a virtual FIDO2 HID device at `/dev/hidraw*` (created by
  the guest frontend via Linux `uhid`).
- Firefox, Chromium, and `fido2-token` in the VM treat it as a normal local
  security key.
- The host broker serializes CTAP HID traffic to the physical key; only
  one active ceremony runs at a time per physical device.
- The physical key stays connected to the host; no USB import/export churn.

## Prerequisites

- A host configured with d2b v0.4 or later.
- A FIDO2-class device connected to the host (YubiKey 5, Security Key NFC,
  etc.).
- The target VM has `guest.control.enable = true` (required for the guest
  frontend DAG node to be supervised).
- You are **not** currently using `usbip.yubikey = true` on the same physical
  device for the same VM. Proxy and USBIP ownership of the same device are
  mutually exclusive; see
  [migration guide](./migrate-usbip-yubikey-to-security-key.md).

## Step 1: Enable the host security-key proxy

Add the following to your d2b host configuration:

```nix
d2b.host.usb.securityKey = {
  enable = true;
  # Optional: restrict to specific device stable selectors.
  # Defaults to all FIDO2-class HID devices (vendor 1050, usage-page 0xF1D0).
  # devices = [
  #   { selector = "by-id"; id = "FIDO:1050:0407:..."; }
  # ];
};
```

> **Why `by-id` selectors?** Raw `/dev/hidrawN` paths depend on probe order
> and change across reboots. Use stable selectors (`by-id`, `by-serial`, or
> vendor/product/serial tuples) so the broker resolves the same physical
> device consistently. See the
> [option reference](../reference/components-usb-security-key.md#device-selectors)
> for selector forms.

## Step 2: Enable the proxy for each target VM

For every VM that should receive a virtual security key, add:

```nix
d2b.vms.personal-dev.usb.securityKey.enable = true;
d2b.vms.work-aad.usb.securityKey.enable = true;
```

Only VMs with `usb.securityKey.enable = true` are authorized to connect to
the host broker. VMs without this option cannot reach the broker socket, even
if they share the same network env.

## Step 3: Rebuild and restart

```bash
sudo nixos-rebuild switch --flake .#desktop
```

The activation step:

1. Materializes the broker socket and udev rules for the configured devices.
2. Starts the per-VM guest frontend DAG node supervised by `d2bd`.
3. Creates `/dev/hidraw*` inside each opted-in VM.

If the daemon was already running, restart it so it picks up the new bundle:

```bash
sudo systemctl restart d2bd.service
```

## Step 4: Verify the virtual device appears in the VM

From inside the VM (or via `d2b vm exec personal-dev --`):

```bash
fido2-token -L
```

Expected output includes an entry for the virtual device, for example:

```
/dev/hidraw0: vendor=0xd2b0 product=0x0001 (d2b security key)
```

You can also run the built-in smoke check:

```bash
d2b usb security-key test personal-dev
```

This verifies that the guest virtual HID exists and that the host broker can
enumerate the physical security key.

## Step 5: Use WebAuthn in the browser

No browser configuration is required. Firefox and Chromium already handle
FIDO2/WebAuthn via `libfido2` and the standard `hidraw` udev rules that the
guest frontend inherits.

1. Open a website that requests a security key in the VM's Firefox.
2. Firefox prompts for the security key and, if required, the FIDO PIN.
3. The host broker acquires a lease on the physical key and records which
   VM is authenticating.
4. An optional host notification appears: `personal-dev is using security key`.
5. Touch the physical security key connected to the host when it blinks.
6. Firefox receives the assertion and completes the login.

## Monitoring and status

Check current lease and per-VM virtual-device health:

```bash
d2b usb security-key status
```

Show active and recent session requests:

```bash
d2b usb security-key sessions
```

Cancel a stuck or timed-out request:

```bash
d2b usb security-key cancel --current
# or by session ID:
d2b usb security-key cancel <session-id>
```

## Contention: two VMs requesting at the same time

Only one active CTAP ceremony is allowed per physical key. When a second VM
requests authentication while the first is in progress, it waits up to the
configured queue timeout (default: 15 seconds). The host displays:

```
YubiKey busy: personal-dev is authenticating
```

If the first ceremony completes within the queue timeout, the second proceeds
normally. If not, the second VM's request times out and the browser reports
no available security key.

To tune the timeouts:

```nix
d2b.host.usb.securityKey = {
  ceremony.timeoutSecs = 120;  # max time waiting for a touch
  queue.timeoutSecs = 20;      # max time the second VM waits
};
```

## Troubleshooting

| Symptom | Likely cause | Remedy |
|---------|-------------|--------|
| `fido2-token -L` shows no virtual device | Guest frontend not started | Check `d2b usb security-key status` and `journalctl -u d2bd` |
| `d2b usb security-key test` fails with "broker unreachable" | Host broker socket not ready | Restart `d2bd`; check `d2b.host.usb.securityKey.enable = true` is in the active bundle |
| Firefox sees no security key | Virtual device created but udev rules not applied in guest | Confirm the guest includes `services.udev.packages = [ pkgs.libfido2 ]` and the `plugdev` group is present |
| `usbip.yubikey = true` eval assertion fires | USBIP and security-key proxy declared for the same device | Disable `usbip.yubikey` for VMs that use `usb.securityKey.enable`; see [migration guide](./migrate-usbip-yubikey-to-security-key.md) |
| Physical key not found by broker | Stable selector does not match any present device | Run `d2b usb security-key status` on the host; verify the device is plugged in and the selector matches |

For USBIP-specific passthrough issues (PIV/CCID/OTP use cases that still use
`usbip`), see [`troubleshoot-usbip.md`](./troubleshoot-usbip.md).
