# USB security-key proxy architecture

This document explains the design rationale for the d2b CTAP/WebAuthn
security-key proxy and answers the questions: "Why not USB passthrough?",
"How does this compare to USBIP?", and "How does it relate to Qubes
`qubes-app-u2f`?".

This is the *explanation* quadrant of the [Diataxis] structure. For task
instructions, see [`../how-to/use-usb-security-key.md`](../how-to/use-usb-security-key.md).
For the option schema and CLI contract, see
[`../reference/components-usb-security-key.md`](../reference/components-usb-security-key.md).

[Diataxis]: https://diataxis.fr/

## Contents

- [1. The problem: one YubiKey, two Firefox VMs](#1-the-problem-one-yubikey-two-firefox-vms)
- [2. Why not USB sharing or passthrough](#2-why-not-usb-sharing-or-passthrough)
- [3. CTAP/WebAuthn proxy architecture](#3-ctapwebauthn-proxy-architecture)
- [4. Comparison with USBIP](#4-comparison-with-usbip)
- [5. Comparison with Qubes qubes-app-u2f](#5-comparison-with-qubes-qubes-app-u2f)
- [6. Trust model and security properties](#6-trust-model-and-security-properties)
- [7. Known limitations](#7-known-limitations)
- [8. Phase-2 hardening option: sys-usb backend](#8-phase-2-hardening-option-sys-usb-backend)

---

## 1. The problem: one YubiKey, two Firefox VMs

A d2b workstation typically has several Firefox VMs — a `personal-dev` realm
and a `work-aad` realm — and a single physical YubiKey used for WebAuthn
authentication on sites like GitHub, GitLab, and corporate identity providers.

The naive solution — USBIP passthrough — transfers the physical USB device into
one VM at a time. Switching VMs requires an explicit `d2b usb detach` / `d2b usb attach`
cycle, during which Firefox in both VMs sees the key disappear and reappear.
This is disruptive when switching contexts rapidly.

The goal is: **both VMs can request the YubiKey at any time without USB bus
churn, with the host serializing actual ceremonies and the user receiving clear
feedback about which VM is authenticating.**

---

## 2. Why not USB sharing or passthrough

### USB ownership is inherently exclusive

All USB transfer methods model one driver/client per device at a time:

- **USB/IP** (RFC 3503 + Linux `usbip-host` / `vhci_hcd`): device-ownership
  transfer. The importing VM loads the device driver and owns all URBs. The
  exporting host has no device access while the VM holds the import.

- **QEMU USB passthrough** (`usb-host`): the QEMU process opens the host
  `/dev/bus/usb` node and feeds URBs to the guest VM. Same exclusive-ownership
  model.

- **SPICE USB redirection** and commercial USB-over-IP stacks (VirtualHere,
  USB/IP derivatives): same model — one client, one owner.

There is no Linux kernel API for USB device *sharing* at the URB level. CTAP
tokens are HID devices, and the HID subsystem does allow multiple `hidraw`
readers, but only at the host kernel level — not across KVM guest isolation
boundaries.

### The wrong abstraction for FIDO/WebAuthn

WebAuthn does not need USB ownership transfer. It needs the ability to send
CTAP command frames (64 bytes each) to a FIDO token and receive responses.
The CTAP HID transport is a simple, stateless application-level protocol on
top of USB HID; it does not require the guest to run a USB host controller or
device driver.

Transferring full USB ownership into a guest for WebAuthn exposes:
- The guest to the token's full USB protocol surface (firmware update, OTP
  serial, PIV/CCID, OpenPGP).
- The host USB stack to a potentially compromised guest (USB driver attack
  surface).
- A disruptive device-attach/detach lifecycle every time the owner changes.

---

## 3. CTAP/WebAuthn proxy architecture

The d2b security-key proxy separates the two roles:

```
Firefox in personal-dev / work-aad
  │  WebAuthn (navigator.credentials.get)
  ▼
guest libfido2 / browser internal
  │  CTAP HID reports via /dev/hidraw* (virtual device)
  ▼
d2b-fido-front (guest frontend, daemon-supervised DAG node)
  │  creates /dev/hidraw* via Linux /dev/uhid
  │  relays 64-byte CTAP HID reports
  ▼
AF_VSOCK (CID = VM index, port 14319)
  ▼
d2bd (host daemon) — lease serializer
  │  one active ceremony per physical key
  │  queues, timeouts, cancellation, audit log
  ▼
broker op UsbSecurityKeyOpenDevice → SCM_RIGHTS fd
  │  d2b-priv-broker resolves stable selector → opens /dev/hidrawN
  ▼
physical YubiKey (host hidraw node)
  │  CTAP HID reports, user touch, responses
  ▼
response relayed back through the same path
```

### Guest frontend (`d2b-fido-front`)

The guest frontend is a daemon-supervised DAG node (not a NixOS systemd unit).
It:

1. Opens `/dev/uhid` in the guest and registers a FIDO2 HID descriptor,
   creating a new `/dev/hidraw*` node that browsers and `libfido2` treat as a
   normal security key.
2. Connects to the host over AF_VSOCK (CID derived from the VM's d2b index,
   port `14319`).
3. Relays 64-byte CTAP HID reports bidirectionally between the guest kernel
   (via the `uhid` device) and the host broker.
4. Handles reconnection with exponential backoff so the virtual device
   survives guest-starts-before-host-broker and daemon restarts.
5. Destroys and recreates the `/dev/uhid` device cleanly when the VSOCK
   connection drops, so browsers see a clean device re-appear rather than
   a stuck/stale HID descriptor.

### Host broker

The host broker runs inside `d2bd` and enforces one active CTAP ceremony per
physical key:

1. Authenticates VM connections by the kernel-supplied AF_VSOCK peer CID (not
   by any in-band guest claim).
2. Authorizes the connecting CID against the d2b VM index with
   `usb.securityKey.enable = true`.
3. Acquires a per-physical-key lease before forwarding any CTAP traffic.
4. Parses CTAPHID headers (per FIDO Alliance CTAPHID spec §7): enforces 64-byte
   report framing, tracks channel/transaction state from `CTAPHID_INIT` through
   response, and translates logical channel IDs (CIDs) so two guests cannot
   collide on the physical token's channel namespace.
5. Sends `CTAPHID_CANCEL` on guest disconnect if a ceremony is mid-flight.
6. Emits structured events to `/run/d2b/usb-sk/events.jsonl` and notifies the
   d2b desktop notification subsystem.

### Privileged device access

Opening `/dev/hidrawN` for a FIDO-class device requires elevated privilege. The
broker dispatches a `UsbSecurityKeyOpenDevice` operation to `d2b-priv-broker`,
which:

1. Resolves the configured stable selector to a `/dev/hidrawN` path.
2. Verifies the device's usage-page is `0xF1D0` (FIDO) and optionally verifies
   vendor/product/serial.
3. Opens the path and returns the file descriptor to `d2bd` via `SCM_RIGHTS`.
4. Logs the operation to the d2b audit log.

`d2b-priv-broker` does not hold the file descriptor after the transfer; it is
owned exclusively by `d2bd` for the lifetime of the active ceremony.

---

## 4. Comparison with USBIP

| Dimension | USBIP (`usbip.yubikey`) | CTAP proxy (`usb.securityKey`) |
|-----------|------------------------|-------------------------------|
| Protocol level | USB URBs (device ownership transfer) | CTAP HID application frames |
| Physical key location | Exported to the VM; not accessible from host while imported | Stays on host; broker owns the `hidraw` fd |
| Guest sees | Real YubiKey USB device (all interfaces: OTP, PIV, CCID, FIDO) | Virtual FIDO2 HID device only (`/dev/hidraw*`) |
| Multi-VM access | One VM at a time; manual detach/attach to switch | Multiple VMs hold virtual devices; broker serializes ceremonies |
| Device churn on VM switch | Device unplugs and replugs in both VMs | No device churn; virtual devices persist |
| Guest USB stack exposure | Guest runs full USB device driver | Guest sees only HID report traffic |
| Use cases | PIV, CCID, OTP, firmware update, any USB function | WebAuthn / FIDO2 only (CTAP HID) |
| When to use | Non-WebAuthn YubiKey functions (PIV smart card, OTP, OpenPGP) | Firefox/browser WebAuthn authentication |

**Use USBIP when** you need PIV, CCID, OTP, or firmware update access inside
a VM. Only one VM can own the device at a time, but those functions do not
require sharing.

**Use the CTAP proxy when** you want multiple VMs to use the same key for
browser WebAuthn without USB churn.

**Mutual exclusion**: because CTAP proxy ownership and USBIP ownership are
incompatible for the same physical key at the same time, d2b enforces a
compile-time mutual-exclusion assertion. See the
[migration guide](../how-to/migrate-usbip-yubikey-to-security-key.md).

---

## 5. Comparison with Qubes `qubes-app-u2f`

[Qubes OS][qubes] provides the closest existing prior art. `qubes-app-u2f`
(also called `qubes-ctap`) is a Python CTAP proxy with:

- A **frontend** in the browser qube that creates a virtual USB-like FIDO HID
  device via Linux `uhid`. Firefox sees a normal local security key.
- A **backend** in `sys-usb` (the USB-isolation VM) that talks to the physical
  token using `python-fido2`.
- **Policy** enforced by the Qubes qrexec system: each CTAP operation type
  (`ctap.GetInfo`, `ctap.ClientPin`, `u2f.Register`, `u2f.Authenticate`) is a
  separate qrexec service, so Qubes policy can allow or deny per-operation
  per-qube.

[qubes]: https://www.qubes-os.org/

### How d2b differs

| Dimension | Qubes `qubes-app-u2f` | d2b security-key proxy |
|-----------|----------------------|------------------------|
| Transport | qrexec (Qubes-proprietary IPC) | AF_VSOCK (Linux VM sockets, standard) |
| Backend location | `sys-usb` VM (USB-isolated VM) | Host broker (`d2bd`) |
| Policy granularity | Per-qrexec-service per-qube | Per-VM enable/disable; RP ID denylist/allowlist in future policy |
| Implementation language | Python (`python-fido2`) | Rust (`d2bd`, `d2b-fido-front`) |
| Token multiplexing | Single backend mux returns first valid response | Serialized lease per physical key; queue/timeout for second VM |
| WINK support | Not supported (documented limitation) | Not supported in phase 1 |
| Credential compartmentalization | Optional per-credential qube binding | Not in phase 1; planned as future RP policy |

### Why d2b does not use `sys-usb` in phase 1

A Qubes-style `sys-usb` VM provides better USB-stack isolation: the host OS
never parses USB device traffic from the token because the USB controller is
assigned to the dedicated VM.

d2b uses a host-side broker in phase 1 for these reasons:

1. **Existing trust model**: d2b already treats the host as the trusted control
   plane for VM lifecycle, VSOCK relays, USBIP binding, and per-VM policy. A
   host broker is consistent with this model and does not require new
   device-VM lifecycle machinery.

2. **USBIP state**: the existing USBIP path already has `busids` claimed by the
   d2b host-side for YubiKey passthrough. Introducing `sys-usb` first requires
   reworking USB controller/device ownership before any CTAP proxy behavior is
   proven.

3. **Narrower first implementation**: a host broker prototype can validate
   CTAP proxy correctness, Firefox WebAuthn compatibility, and contention
   behavior before adding a new trusted VM, its lifecycle ordering, policy
   routing, and recovery surface.

A `sys-usb`-style backend remains a phase-2 hardening option (see
[§8](#8-phase-2-hardening-option-sys-usb-backend)).

---

## 6. Trust model and security properties

### VM authorization

VM connections to the host broker are authorized by the kernel-supplied
AF_VSOCK peer CID. The CID is set by the KVM hypervisor and cannot be forged
by guest software. The broker maps CID to the d2b VM index and checks whether
that VM has `usb.securityKey.enable = true` in the active bundle.

No in-band claims from the guest (e.g., a VM claiming to be `work-aad` in the
protocol header) are trusted.

### Protocol boundary

The broker is protocol-aware, not a raw byte pipe:

- It enforces 64-byte CTAPHID report framing (rejects malformed lengths).
- It parses CTAPHID channel/transaction state and translates logical CIDs
  so two guests cannot collide on the physical token's channel namespace.
- It sends `CTAPHID_CANCEL` when a guest disconnects mid-ceremony.

### Log scrubbing

The broker never logs:
- Raw CTAP payload bytes.
- FIDO PINs or PIN/UV authentication material.
- Assertion signatures or credential private material.

It logs only: VM identity, stable key selector, high-level operation type,
RP ID (if safely parsed and `notifications.showRpId = true`), lease lifecycle
events, result, and error class.

### Physical key stays on the host

The guest never touches the host USB bus. USB firmware update commands,
OTP serial access, PIV commands, and OpenPGP operations are not reachable via
the CTAP proxy. Guests with `usb.securityKey.enable = true` but without
`usbip.yubikey = true` cannot access these interfaces.

---

## 7. Known limitations

- **Not simultaneous**: the broker serializes ceremonies per physical key. Two
  VMs cannot run an active CTAP transaction at exactly the same time. The queue
  window (default 15 seconds) is shorter than typical browser WebAuthn timeouts
  so the second VM fails predictably rather than hanging.

- **WINK not forwarded**: the `CTAPHID_WINK` command (ask the token to identify
  itself visually) is not forwarded in phase 1. The physical key's normal
  user-presence blink occurs without a virtual-device WINK trigger.

- **No per-credential RP policy**: the broker does not enforce which VMs may
  use which credentials in phase 1. All opted-in VMs share access to the same
  physical key's credential store. Per-RP or per-credential VM allowlists are
  planned as a future policy option.

- **One physical key**: the proxy serializes one physical key. Multiple physical
  keys are supported by adding separate selector entries; each key has its own
  independent lease.

- **Phase 1 mutual exclusion with USBIP**: CTAP proxy and USBIP cannot target
  the same physical key from the same VM simultaneously. This is a deliberate
  simplification; a future phase may relax this for non-overlapping time windows.

---

## 8. Phase-2 hardening option: sys-usb backend

A Qubes-style `sys-usb` backend is viable as a future hardening option if the
threat model changes from "host is the trusted d2b control plane" to "the host
OS should not parse HID traffic from the token":

```
Firefox VM → guest uhid frontend → AF_VSOCK → d2bd (host) → AF_VSOCK → sys-usb broker VM → physical YubiKey
```

In this design:
- The physical USB controller or YubiKey device is assigned to a dedicated
  `sys-usb` backend VM.
- The d2b host acts only as a routing and authentication layer, not as a CTAP
  intermediary.
- The `sys-usb` VM is the only process that opens the `hidraw` node.

This reduces the host OS USB attack surface at the cost of a new trusted VM,
additional lifecycle ordering, and more complex recovery. It is a hardening
path, not a phase-1 requirement.
