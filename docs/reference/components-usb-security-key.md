# `d2b.host.usb.securityKey.*` and `d2b.vms.<vm>.usb.securityKey.*`

> Reference for the `usb.securityKey` component: the d2b CTAP/WebAuthn
> security-key proxy.
> Source: [`nixos-modules/components/usb-security-key.nix`](../../nixos-modules/components/usb-security-key.nix)
> Host-side wiring: [`nixos-modules/host-usb.nix`](../../nixos-modules/host-usb.nix)
> CLI integration: [`packages/d2b/src/usb/security_key.rs`](../../packages/d2b/src/usb/security_key.rs)
> (`d2b usb security-key status|sessions|cancel|test`)

## What this component does

Enables a CTAP/WebAuthn proxy so that opted-in VMs can authenticate with a
host-attached FIDO2 security key (YubiKey, Security Key NFC, etc.) through
Firefox or any other WebAuthn-capable browser, without USB device transfer.

The guest receives a virtual FIDO2 HID device at `/dev/hidraw*` created by the
guest frontend via Linux `/dev/uhid`. Fixed 64-byte CTAPHID reports travel on a
credit-bounded named stream inside an authenticated `security-key`
ComponentSession over the allocator-provided AF_VSOCK endpoint. The frontend
does not accept the former unauthenticated length-prefixed relay. Only one
active CTAP ceremony runs per physical key at any time; concurrent requests
from multiple VMs are serialized with a configurable queue timeout.

This component does **not** share, clone, or simultaneously forward USB
ownership to multiple guests. It is a protocol-level CTAP proxy that enforces
one active ceremony per physical device. For USB passthrough use cases (PIV,
CCID, OTP) see [`components-usbip.md`](./components-usbip.md).

## Mutual-exclusion rule

`usb.securityKey.enable = true` and `usbip.yubikey = true` are mutually
exclusive for the same VM and same physical key. The eval-time assertion fires
if both are configured for the same VM:

```
d2b: usb.securityKey and usbip.yubikey are mutually exclusive for VM
'<vm>'. Disable usbip.yubikey before enabling usb.securityKey.
```

See [migration guide](../how-to/migrate-usbip-yubikey-to-security-key.md) for
the transition path.

## Host options (`d2b.host.usb.securityKey.*`)

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `d2b.host.usb.securityKey.enable` | `bool` | `false` | Enable the host-side security-key broker. Required before any VM can opt in. When `false`, no broker socket is created and no udev rules for FIDO-class HID devices are installed. |
| `d2b.host.usb.securityKey.devices` | `listOf deviceSelector` | `[]` | Restrict broker access to specific physical devices. An empty list grants no security-key access until selectors are declared. See [Device selectors](#device-selectors). |
| `d2b.host.usb.securityKey.ceremony.timeoutSecs` | `int` | `120` | Maximum seconds the broker waits for a user-presence touch before cancelling an active CTAP ceremony and returning `CTAPHID_ERROR` to the guest. |
| `d2b.host.usb.securityKey.queue.timeoutSecs` | `int` | `15` | Maximum seconds a second VM's request waits while another ceremony is in progress. When this elapses, the queued VM receives an immediate timeout error. |
| `d2b.host.usb.securityKey.notifications.enable` | `bool` | `true` | Emit desktop notifications for ceremony start, touch-wait, contention, and failure events via the d2b notification subsystem. |
| `d2b.host.usb.securityKey.notifications.showRpId` | `bool` | `true` | Include the RP ID (relying-party domain, e.g. `github.com`) in notifications when it can be safely parsed from the CTAP request. Set `false` to omit RP information from all notifications. |

## Per-VM options (`d2b.vms.<vm>.usb.securityKey.*`)

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `d2b.vms.<vm>.usb.securityKey.enable` | `bool` | `false` | Enable the virtual FIDO2 HID frontend for this VM. Requires `d2b.host.usb.securityKey.enable = true`. The guest frontend is supervised by the `d2bd` DAG executor and creates `/dev/hidraw*` inside the guest. |

## Eval-time assertions

The following conditions are enforced at NixOS eval time (before any build):

1. **Host prerequisite**: `d2b.vms.<vm>.usb.securityKey.enable = true` requires
   `d2b.host.usb.securityKey.enable = true`. Enabling a VM without the host
   option produces an assertion failure with a clear message.

2. **Mutual exclusion**: `usbip.yubikey = true` and `usb.securityKey.enable = true`
   cannot both be set for the same VM. The assertion message names the
   conflicting VM.

3. **Guest control prerequisite**: `usb.securityKey.enable = true` requires
   `guest.control.enable = true` on the same VM (the DAG supervisor needs
   guest control to manage the frontend node lifecycle).

## Device selectors

`d2b.host.usb.securityKey.devices` is a list of selector objects. An empty
list (the default) grants no device access. Each selector must include a
stable label, USB vendor/product IDs, and an optional serial:

```nix
{
  label = "yubikey-primary";
  vendorId = 4176;  # 0x1050, Yubico
  productId = 1031; # 0x0407, YubiKey 5 NFC
  serial = null;    # or "XXXXXXXXXXXX" to disambiguate identical keys
}
```

> **Note:** Raw `/dev/hidrawN` paths are not supported as selectors because
> device indices change across reboots and when other HID devices are added or
> removed. Broad vendor-only selectors are not supported; each configured
> device must name a specific vendor/product pair from the FIDO allowlist.

## Host-side resources (broker-owned, not NixOS module-declared)

The host broker owns the following runtime resources. They are **not** declared
as NixOS systemd services or static units; they are created and torn down by
the `d2bd`/`d2b-priv-broker` pipeline at runtime:

| Resource | Owner | Path / name |
|----------|-------|-------------|
| Broker AF_UNIX socket | `d2bd` (via broker spawn) | `/run/d2b/usb-sk-broker.sock` |
| Per-VM AF_VSOCK listener | `d2bd` DAG executor | CID mapped to VM index, port `14319` |
| udev rule for FIDO HID devices | broker op `UsbSecurityKeyUdevRule` | `/run/udev/rules.d/72-d2b-fido.rules` |
| Per-device file descriptor | broker op `UsbSecurityKeyOpenDevice` | returned via SCM_RIGHTS to `d2bd` |
| Lease state file | `d2bd` | `/run/d2b/usb-sk/lease.json` |
| Event log | `d2bd` | `/run/d2b/usb-sk/events.jsonl` |

## Guest-side resources

When `usb.securityKey.enable = true`, the guest NixOS config includes:

- The `d2b-fido-front` binary (added to `environment.systemPackages` by the
  component module).
- `services.udev.packages = [ pkgs.libfido2 ]` (ensures `/dev/hidraw*` is
  accessible to the `plugdev` group in the guest).
- The `plugdev` group is added to the guest's admin user.

The DAG node lifecycle (start, retry/backoff on disconnect, stop on VM
shutdown) is supervised entirely by `d2bd` on the host. No per-VM systemd
unit is declared in the guest or host NixOS config for this component.

The allocator supplies a nonzero reconnect generation and a 32-byte channel
binding to both ComponentSession peers. Missing or malformed session material
fails the frontend closed; it never retries with the old raw relay. UHID
remains active across authenticated session reconnects, but reports are queued
only within the fixed ComponentSession credit window.

Before forwarding a complete browser request, the frontend buffers at most one
bounded CTAPHID message per channel and applies the canonical host-mediated
device policy. Read-only discovery is allowed. Credential creation and
assertion ceremonies are marked as requiring approval from the authenticated
controller. Reset, credential management or deletion, biometric enrollment,
authenticator configuration, vendor commands, legacy CTAPHID message commands,
and unknown commands are denied locally. UHID traffic is never treated as
approval.

## CLI surface

```
d2b usb security-key <SUBCOMMAND>

Subcommands:
  status    Show configured keys, virtual-device health per VM, active lease
  sessions  Show active and recent security-key request sessions
  cancel    Cancel a stuck or in-progress security-key request
  test      Smoke-check virtual HID presence in a VM and host broker reachability
  help      Print help
```

Full CLI contract: see [`cli-contract.md`](./cli-contract.md) and the golden
output files under [`cli-output/`](./cli-output/) prefixed `usb-security-key-`.

## Notification events

Security-key proxy events are emitted through the standard d2b notification
subsystem. For the machine-readable event schema, see
[`usb-security-key-events.md`](./usb-security-key-events.md).
