# Migrate from `usbip.yubikey` WebAuthn to `usb.securityKey`

This guide covers the configuration changes required when you move a VM from
the legacy `usbip.yubikey = true` path for browser-based WebAuthn to the new
`usb.securityKey.enable = true` CTAP proxy.

For the new feature overview, see
[`use-usb-security-key.md`](./use-usb-security-key.md).

## What changes

### Old path: USBIP YubiKey passthrough

`usbip.yubikey = true` imports the entire physical USB device into one VM at a
time. The guest loads `vhci_hcd`, the broker exports the device, and the guest
kernel runs the full USB device driver. The physical key is exclusively owned
by one VM at a time; to switch VMs, you run `d2b usb detach` and then
`d2b usb attach`.

Limitations for WebAuthn:

- Only one VM can use the key simultaneously; the second must wait for an
  explicit detach/attach cycle.
- Hot-unplug events are visible to the VM (the key disappears then reappears
  from Firefox's perspective).
- The guest USB stack is exposed to the device's full USB protocol surface.

### New path: CTAP/WebAuthn proxy

`usb.securityKey.enable = true` keeps the physical key under host control. The
guest frontend creates a virtual FIDO2 HID device via `/dev/uhid`; the host
broker serializes CTAP traffic to the physical key over an AF_VSOCK channel.

Benefits for WebAuthn:

- Multiple VMs can hold virtual devices simultaneously; the broker serializes
  the actual CTAP ceremonies (one at a time per physical key).
- No USB bus churn between VMs.
- The guest USB stack is not exposed to the physical key; only CTAP HID report
  frames cross the trust boundary.

### Mutual-exclusion rule

**`usbip.yubikey = true` and `usb.securityKey.enable = true` are mutually
exclusive for the same physical key and VM in phase 1.** The d2b eval-time
assertion fires if both options are set for the same VM:

```
d2b: usb.securityKey and usbip.yubikey are mutually exclusive for VM
'personal-dev'. Disable usbip.yubikey before enabling usb.securityKey.
```

If you need both USBIP (for PIV/CCID/OTP use cases) and the CTAP proxy (for
WebAuthn), they must use different physical keys or different VMs. See the
[USBIP reference](../reference/components-usbip.md) for non-WebAuthn USBIP
use cases that do not share a device with the security-key proxy.

## Step-by-step migration

### 1. Identify which VMs use `usbip.yubikey` for WebAuthn

```bash
grep -r "usbip.yubikey" /etc/nixos/modules/d2b-config.nix
```

For each VM using `usbip.yubikey = true` and requiring WebAuthn (rather than
PIV or OTP), apply the steps below.

### 2. Disable `usbip.yubikey` for the migrating VM

```nix
# Before
d2b.vms.personal-dev = {
  usbip.yubikey = true;
  usbip.busids = [ "1-3.4.3" ];
};

# After
d2b.vms.personal-dev = {
  # usbip.yubikey removed — physical key is now owned by the host broker
};
```

Leave `usbip.busids` or `d2b.site.yubikey.enable` in place only if you have
other VMs that still use USBIP for non-WebAuthn purposes (PIV/CCID/OTP).

### 3. Remove any existing USBIP session claim for the device

Before the rebuild, release any existing USBIP lease on the device:

```bash
d2b usb detach personal-dev 1-3.4.3 --apply
```

If the key is not currently claimed, the detach is a no-op.

### 4. Enable the host proxy and VM opt-in

```nix
d2b.host.usb.securityKey.enable = true;
d2b.vms.personal-dev.usb.securityKey.enable = true;
```

### 5. Rebuild

```bash
sudo nixos-rebuild switch --flake .#desktop
```

### 6. Verify

Inside the VM:

```bash
fido2-token -L
# Expect: /dev/hidrawN: vendor=0xd2b0 product=0x0001 (d2b security key)
```

From the host:

```bash
d2b usb security-key test personal-dev
d2b usb security-key status
```

## Mixed scenario: WebAuthn and PIV/CCID on the same YubiKey

If you need USBIP passthrough for PIV/CCID/OTP **and** the CTAP proxy for
WebAuthn on the **same YubiKey**:

1. Use a second physical security key (USB-A and NFC variants both work).
   Assign one key to the CTAP proxy via `usb.securityKey` and the other to
   the VMs that need PIV/CCID via `usbip.yubikey`.

2. Alternatively, if only one VM needs PIV and a different VM needs WebAuthn,
   assign the physical key to the VM that needs PIV via USBIP and configure
   the WebAuthn VM with its own separate security key via the CTAP proxy.

There is no phase-1 support for a single physical key to serve both CTAP proxy
and USBIP concurrently. This is a known limitation.

## Checking which path is active

```bash
d2b usb probe
```

USBIP claims appear in the `session_claims` list. If a device that previously
appeared there is now absent, it has been released from USBIP ownership.

```bash
d2b usb security-key status
```

Virtual-device health and active leases appear here after the migration.
