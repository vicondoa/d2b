# `d2b.vms.<vm>.usbip.*`

> Reference for the `usbip` component module (YubiKey passthrough).
> Source: [`nixos-modules/components/usbip.nix`](../../nixos-modules/components/usbip.nix)
> Host-side wiring: [`nixos-modules/network.nix`](../../nixos-modules/network.nix), [`nixos-modules/host.nix`](../../nixos-modules/host.nix)
> CLI integration: [`packages/d2b/src/lib.rs`](../../packages/d2b/src/lib.rs) (`d2b usb attach|detach|probe`). There is no bash helper for this surface.

## What this component does

Enables on-demand passthrough of a host-side YubiKey (USB vendor ID
`1050`) into a VM via USBIP. When `d2b.site.yubikey.enable = true`
and some enabled VM in an env sets `usbip.yubikey = true`, the host
materializes a broker-spawned per-env `usbipd` backend listening on TCP
`<backendPort>` (usbipd has no `--host` flag, so it binds to
`0.0.0.0`; firewall rules — see "Host-side resources" — restrict
backend ingress to host loopback, so it's the operational equivalent
of a loopback bind but enforced via netfilter rather than by the
socket). A broker-spawned per-env `socat` proxy binds exactly the env's
uplink-bridge IP at TCP 3240; the guest loads `vhci_hcd`, ships the
`usbip` CLI, and advertises guestd's `UsbipImport` capability so
`d2bd` can import/detach through authenticated guest-control. The
hot-plug ceremony is daemon-owned: host bind/unbind and firewall/proxy
reconcile go through the privileged broker, while guest attach/detach goes
through guestd. The CLI sends one intent to `d2bd`; it never SSHes into
the guest for USBIP.

The component itself only declares the **guest-side** wiring. All
host-side machinery (usbipd backend + proxy broker-spawned runners,
udev rules, firewall rules, the `usbip-host` kernel module) lives
elsewhere — see "Host-side resources" below.

USB and HID capabilities remain independent from display; see
[display and virtual I/O capabilities](./display-io-capabilities.md).

## Claim, carrier, and restart model

USBIP state is reported as separate layers because they have different
owners and remediation:

- **Session claim** — the broker-owned per-busid lock under
  `/run/d2b/locks/usbip/<busid>`. It records which VM owns the right
  to expose the physical device for the current host boot/session. The claim
  survives VM stop/restart and daemon restart, but not host reboot because the
  backing path is under `/run`. Only explicit
  `d2b usb detach <vm> <busid> --apply` releases a healthy claim during
  that host session.
- **Active carrier** — transient host/guest state that can disappear across
  unplug, VM stop, daemon restart, or guest-control restart: the
  `usbip-host` module, host driver bind, per-env backend/export readiness,
  per-env proxy listener, and guest import.
- **Policy/topology** — bundle-declared vendor/product and bus/port
  identity checks. Required failures fail before device exposure and require
  fixing the declaration or attaching the approved physical device, then
  rebuilding before retry.

VM stop/restart cleans up guest imports and only runs host unbind when firewall
withdrawal plus targeted stream cleanup can be proven first; otherwise it keeps
the same-VM session claim for manual recovery. VM start reconciles same-VM
session claims from the current host session after guest-control readiness by
replaying host bind/proxy state and re-importing in the guest. Runtime absence,
proxy/backend unavailability, or guest import unavailability degrades
`d2b usb probe` / `d2b status` without pretending the row is healthy.
Required policy/topology failures remain fail-closed.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `d2b.vms.<vm>.usbip.yubikey` | bool | `false` | YubiKey USBIP passthrough opt-in for this VM. Loads `vhci_hcd` in the guest and installs `usbip` so the USB CLI can redirect a plugged-in Yubico device. |
| `d2b.vms.<vm>.usbip.busids` | list of string | `[]` | Exact USBIP busids the daemon should advertise for this VM in `host.json.environments[].usbipBusidLocks[].busIds`. Leave empty to preserve the legacy `pending` fallback for older fixtures. |
| `d2b.host.usbip.allowlist` | list of `{ vendor, product }` | `[]` | Host-wide vendor:product policy copied into each `host.json.environments[].usbipBusidLocks[].vendorProductAllowlist` row. Use hex strings such as `0x1050` / `0x0407` to allow only specific hardware families even when busids change across replug events. |

Site-level dependency:

| Option | Type | Default | Description |
|---|---|---|---|
| `d2b.site.yubikey.enable` | bool | `true` | Host-side Yubikey support: Yubico udev rules for vendor `1050` (GROUP=kvm, MODE=0660, TAG+=uaccess). The `usbip-host` kernel module is loaded only when this option is on **and** at least one enabled VM sets `usbip.yubikey = true`. Set `false` on hosts that do not use YubiKeys — per-VM `usbip.yubikey = true` still pulls in the guest-side bits, but the host has no Yubikey-specific machinery loaded. |

## Options (guest-side propagation)

None. The component module is imported directly into the guest's
config by `host.nix` (`++ lib.optional vm'.usbip.yubikey
./components/usbip.nix`).

## Host-side resources created

Per opted-in env (declared in [`network.nix`](../../nixos-modules/network.nix); materialized only when `d2b.site.yubikey.enable = true` and at least one enabled VM in that env sets `usbip.yubikey = true`):

> There are no `d2b-sys-<env>-usbipd-{backend,proxy}` systemd
> units. The broker spawns backend/proxy runners under
> `d2b.slice/sys-<env>/usbipd-*`, and the hardening shape
> documented below is enforced as the runner contract.
>
> `ModprobeIfAllowed{module: "usbip-host"}` runs before the first
> `UsbipBackend` runner for each env. Per-attach host `usbip bind` /
> `unbind` steps are broker ops with per-env busid locking and audit
> coverage; guest `usbip attach` / `detach` is an authenticated guestd RPC.
> VM start/stop USB reconciliation threads one bounded reconcile correlation ID
> through its USB broker requests as the broker audit `tracingSpanId`.
> These privileged USB broker requests inherit the broker IPC limiter
> (stable UID/role/operation buckets for daemon-forwarded calls, a
> collapsed direct-peer bucket for non-daemon callers) and the bounded
> audit-write limiter documented in
> [`daemon-api.md`](./daemon-api.md#broker-socket).
> USB reconciliation observability partitions dedupe/rate-limit buckets by
> closed event type plus bounded source kind/VM projection, never by process
> ID, trace ID, bus ID, sysfs path, or serial. Bucket caps are strict: once the
> reserved capacity is full, new partitions collapse into a single `other`
> bucket instead of evicting older keys. Metric labels are static
> (`present`/`none`/`other` for VM source presence), while structured log/event
> DTOs keep unknown but well-shaped VM and error values and reject only unknown
> fields or malformed shapes. Suppressed-event summaries carry the suppressed
> count and window start/end.
>
> Successful `UsbipBind` broker audit records include a root-only forensics
> projection: normalized VID/PID, a boolean `serialObserved`, and HMAC-SHA256
> serial correlations when a serial descriptor exists. The HMAC keyring lives
> under `${d2b.site.stateDir}/secrets/usb-audit-serial-hmac/` as
> root-only `current.key` plus optional `previous.key`. The broker reads the
> files on each bind, so key reload is per-request; no non-root observability
> unit receives key material through systemd credentials, environment variables,
> config files, or IPC. During rotation, operators atomically install a new
> `current.key`, keep the old key as `previous.key` for the 30-day grace
> window, then remove `previous.key` to close the window. While both files are
> present, audit records carry both current and previous correlations and the
> broker emits one structured rotation-window log event per key pair with only
> key IDs, active-key count, grace-window length, and the closed correlation
> version.

- **`d2b.slice/sys-<env>/usbipd-backend` runner** — runs
  `usbipd -4 --tcp-port <backendPort>`. usbipd has no `--host` flag
  so it binds to `0.0.0.0`; the broker-managed `inet d2b`
  `input` chain drops non-loopback ingress to each backend port, so
  the effective path is host-local proxy → `127.0.0.1:<backendPort>`.
  Pre-spawn: host-prep DAG op `ModprobeIfAllowed{module:
  "usbip-host"}`. The broker runs this root-only backend in a private
  mount + PID namespace with seccomp, `CAP_NET_RAW` only, masked host
  secret directories, a fresh procfs, a masked `/dev`, and only the
  locked USB device node visible.
- **`d2b.slice/sys-<env>/usbipd-proxy` runner** —
  `socat TCP-LISTEN:3240,bind=<env.hostUplinkIp>,fork,max-children=4,reuseaddr
  TCP:127.0.0.1:<backendPort>`. Requires + after the matching backend
  runner. `CapabilityBoundingSet = ""`. The listener is never wildcard
  (`0.0.0.0`/`::`) and there is no cross-env proxy. The proxy is a
  generic L4 TCP forwarder; it does not parse USBIP packets and cannot
  selectively identify one busid stream by itself.

Firewall carve-outs (canonical `inet d2b` table per ADR 0013 +
[`inet-d2b-chains.md`](./inet-d2b-chains.md)):

The broker emits these source-based carve-outs through the existing
`UsbipBindFirewallRule` broker op. Single-busid revocation is allowed to
kill an established stream only after the firewall carve-out has been
blocked or withdrawn; if the daemon cannot prove that ordering and an exact
VM/proxy cleanup tuple, detach fails closed instead of killing the shared
per-env proxy listener. The carve-outs land in the canonical `input` chain
inside the `inet d2b` table BEFORE the generic TCP/3240 drop rule. The
carve-out matrix is:

- DROP source ≠ 127.0.0.1 to the env's backend loopback port.
- DROP all TCP/3240 proxy traffic by default.
- ACCEPT only traffic arriving on the env's uplink bridge with
  `ip saddr <env.netUplinkIp>` and `ip daddr <env.hostUplinkIp>`.

The current net VM topology preserves env identity, not workload-VM
source identity, at the host proxy: workload traffic to
`<env.hostUplinkIp>:3240` crosses the net VM and is SNATed to
`<env.netUplinkIp>`. The broker therefore does **not** widen the
carve-out to the whole uplink subnet and does not claim VM-source
scoping. Instead it uses the non-spoofable point-to-point net-VM
uplink identity and fails closed if the host bundle lacks the
uplink IPs or the uplink bridge-port anti-spoofing shape
(`isolated`, neighbor suppression, no learning, no unicast flooding).

The legacy iptables `nixos-fw` rules (an interim implementation
that inserted at position 1 in `nixos-fw` to win first-match
against NixOS's generated accepts) were retired in favour of the
daemon-owned broker `inet d2b` table.
Implementations MUST emit via the broker `UsbipBindFirewallRule`
broker op so the carve-out ordering is enforced by
`d2b_host::nftables::NftBatch::assert_carveout_ordering`.
The op is invoked by the host-prep DAG (before the
`UsbipBackend` SpawnRunner starts for each env) and by the
per-attach state machine (before `UsbipBindOneShot` SpawnRunner
runs); see [ADR 0018](../adr/0018-microvm-nix-removal.md) §
"Disposition matrix" USBIP row for the full lifecycle.

Per host (in [`host.nix`](../../nixos-modules/host.nix)):

- When `d2b.site.yubikey.enable = true`, udev rules for vendor
  `1050` on `hidraw` + `usb` subsystems:
  `GROUP="kvm" MODE="0660" TAG+="uaccess"`.
- `boot.kernelModules += [ "usbip-host" ]` only when
  `d2b.site.yubikey.enable = true` **and** at least one enabled
  VM sets `usbip.yubikey = true`.
- The `/dev/kvm` lock-down rule (`KERNEL=="kvm", GROUP="kvm",
  MODE="0660"`) is unconditional and not part of the yubikey gate.

## Runtime prerequisite contract

For `d2b usb attach <vm> <busid> --apply` to expose a device, all of
these must be true:

1. the target VM is running and guest-control advertises USBIP status/import;
2. the bundle declares USBIP bind/firewall intents for the VM and busid;
3. the session busid claim is missing or already held by the target VM;
4. `usbip-host`, the physical device, host bind operation, per-env backend,
   and per-env proxy can converge;
5. topology/policy checks allow the observed physical device; and
6. the guest can import the device from its own env's
   `<env.hostUplinkIp>:3240` path.

Stable operator remediation uses lifecycle verbs rather than direct lock or
sysfs mutation. Keep procedural recovery in the how-to runbook:
[Troubleshoot USBIP passthrough](../how-to/troubleshoot-usbip.md).

CLI contract (`d2b usb attach|detach|probe` in the Rust CLI):

- Sends one apply/dry-run intent to `d2bd`.
- `attach --apply`: guestd first detaches any stale matching import, the
  broker binds/locks the host busid and reconciles firewall/proxy state, then
  guestd imports the device inside the VM.
- `detach --apply`: for the generic per-env L4 proxy, the daemon first requires
  an immediate-revocation proof: firewall block/withdrawal must precede any
  targeted conntrack deletion or TCP established-socket kill for a proven
  VM/proxy tuple whose source is not hidden by SNAT and whose anti-spoofing
  posture is proven. When that proof is unavailable, detach returns the public
  `usbip-revocation-not-isolated` failure with the target busid and preserves
  the broker-owned claim for manual drain/recovery instead of silently leaving
  an established stream or bouncing unrelated same-env streams.

### Proxy synchronization strategy

The current proxy is per-env, not per-busid: a `socat` L4 listener forwards
`<env.hostUplinkIp>:3240` to that env's loopback backend port. Synchronization
therefore follows the conservative daemon plan in
`packages/d2bd/src/usbip_reconcile_state.rs`:

1. normal attach or single-VM restart performs an optimistic backend/export
   refresh and verifies that the per-env proxy is listening;
2. single-busid detach removes the firewall carve-out before any flow kill,
   asks the device-specific `usbip_sockfd` control to shut down the usbip-host
   stream, then uses only exact VM/proxy tuple cleanup with proven per-VM source
   identity without stopping or rebinding the per-env proxy;
3. no generic sysfs/listener revoke is claimed: TCP may use exact conntrack
   deletion and/or exact established-socket kill, UDP may use exact conntrack
   deletion only, and shared listeners or ambiguous same-env streams are never
   killed for one busid. If the device-specific stream/unbind controls or those
   tuple guarantees are unavailable, revocation fails closed and preserves the
   broker-owned session busid claim for manual drain/recovery; and
4. bouncing same-env active streams is permitted only through an explicit
   bounded-drain or force-recycle policy, which must take an exclusive socket
   lifecycle lock before any rebind.

This means a single VM restart in an env must not disconnect unrelated active
USBIP streams in that same env.

## Guest-side resources created

The entire `components/usbip.nix` is two lines of payload:

```nix
{
  boot.kernelModules = [ "vhci_hcd" ];
  environment.systemPackages = [ pkgs.linuxPackages.usbip ];
  d2b.guestControl.usbipPath = ".../bin/usbip";
}
```

- `vhci_hcd` lets `usbip attach` materialise the redirected device
  as `/dev/hidraw<N>` (or a raw USB node) inside the guest kernel.
- The `usbip` CLI is needed in-guest so guestd can issue `usbip port`,
  `usbip detach`, and `usbip attach` after authenticating the host over
  guest-control. Host-side `usbip bind/unbind`, firewall, and proxy
  reconciliation dispatch through the daemon → broker path.

## Runtime invariants

- A declared busid is exposed to at most one VM/env owner at any moment. The
  broker-owned per-busid claim, host unbind, and firewall carve-out are the
  enforcement mechanisms. The generic per-env proxy is not stopped merely to
  revoke one busid, because doing so would bounce unrelated same-env streams.
- The host-side proxy listens only on `<env.hostUplinkIp>:3240`. A
  workload VM in env A cannot reach env B's usbipd via routing —
  the nftables carve-out (per ADR 0013 + this doc's
  "Firewall carve-outs" section above) keys on the env's own
  uplink bridge, host destination IP, and net-VM uplink source IP.
- Guestd's `usbip attach` connects to its own env's
  `usbipdHostIp` (the host-side end of that env's uplink bridge),
  not the host's WAN address.

## Hardening notes

- Backend listens on TCP `<backendPort>` (bound to `0.0.0.0`
  because usbipd has no `--host` flag); the broker-managed nftables
  `inet d2b` input chain explicitly drops non-loopback ingress to
  backend ports, making the backend effectively loopback-only even
  though the socket itself is all-interface-bound. Each env's proxy is
  a bounded `socat` instance with an empty capability set.
- Backend retains only the USBIP backend capability set. The proxy runs
  with an empty capability set and listens only on `<env.hostUplinkIp>:3240`.
- The kvm-group YubiKey udev grant (`GROUP="kvm" MODE="0660"`) is
  the smallest set that lets `usbip bind` work without `sudo` for
  the launcher user. The `usbip bind/unbind` step itself is
  dispatched through the broker `SpawnRunner` runner on the per-env
  DAG (`d2b.slice/sys-<env>/usbip-bind`), so no `sudo`
  escalation is required.
- The `d2b-<vm>-gpu` user is not in the kvm group strictly for
  USB; it's there for `/dev/kvm`. USBIP traffic flows over TCP, not
  device-node ACLs.

## Failure-mode reference

`d2b usb probe` is the stable read-only diagnostic surface. It reports
session claim state, active host carrier/bind/proxy state, guest import state,
topology/policy state, degraded reasons, and copy-paste remediation commands.
See [`cli-output/usb-probe.md`](./cli-output/usb-probe.md) for the JSON field
contract and degraded reason table.

Troubleshooting walkthroughs belong in Diataxis how-to docs, not in this
reference page. See [Troubleshoot USBIP passthrough](../how-to/troubleshoot-usbip.md)
for operator procedures.

## See also

- [Design / threat model](../explanation/design.md)
- [Manifest schema](./manifest-schema.md) — `units.usbipBackend` /
  `units.usbipProxy` (per-env, not per-VM).
- [CLI contract](./cli-contract.md) — `d2b usb attach|detach|probe` subcommands.
- [Troubleshoot USBIP passthrough](../how-to/troubleshoot-usbip.md) —
  operator recovery workflow.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  end-to-end example with `usbip.yubikey = true`.
- [CHANGELOG.md](../../CHANGELOG.md) — release history for USBIP gating and related fixes.
