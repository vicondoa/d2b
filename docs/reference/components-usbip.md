# `nixling.vms.<vm>.usbip.*`

> Reference for the `usbip` component module (YubiKey passthrough).
> Source: [`nixos-modules/components/usbip.nix`](../../nixos-modules/components/usbip.nix)
> Host-side wiring: [`nixos-modules/network.nix`](../../nixos-modules/network.nix), [`nixos-modules/host.nix`](../../nixos-modules/host.nix)
> CLI integration: [`packages/nixling/src/lib.rs`](../../packages/nixling/src/lib.rs) (`nixling usb attach|detach|probe`). There is no bash helper for this surface.

## What this component does

Enables on-demand passthrough of a host-side YubiKey (USB vendor ID
`1050`) into a VM via USBIP. When `nixling.site.yubikey.enable = true`
and some enabled VM in an env sets `usbip.yubikey = true`, the host
materializes a broker-spawned per-env `usbipd` backend listening on TCP
`<backendPort>` (usbipd has no `--host` flag, so it binds to
`0.0.0.0`; firewall rules — see "Host-side resources" — restrict
source addresses to host loopback, so it's the operational equivalent
of a loopback bind but enforced via netfilter rather than by the
socket). A broker-spawned `socat` proxy binds exactly the env's
uplink-bridge IP at TCP 3240; the guest loads `vhci_hcd` and ships the
`usbip` CLI so it can `usbip attach` against that proxy. The hot-plug
ceremony — bind on host, attach in VM, detach + unbind on Ctrl-C — is
orchestrated by the host-side `nixling usb attach|detach|probe` CLI
surface dispatched through the daemon → broker `SpawnRunner` path.

The component itself only declares the **guest-side** wiring. All
host-side machinery (usbipd backend + proxy broker-spawned runners,
udev rules, firewall rules, the `usbip-host` kernel module) lives
elsewhere — see "Host-side resources" below.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.usbip.yubikey` | bool | `false` | YubiKey USBIP passthrough opt-in for this VM. Loads `vhci_hcd` in the guest and installs `usbip` so the USB CLI can redirect a plugged-in Yubico device. |
| `nixling.vms.<vm>.usbip.busids` | list of string | `[]` | Exact USBIP busids the daemon should advertise for this VM in `host.json.environments[].usbipBusidLocks[].busIds`. Leave empty to preserve the legacy `pending` fallback for older fixtures. |
| `nixling.host.usbip.allowlist` | list of `{ vendor, product }` | `[]` | Host-wide vendor:product policy copied into each `host.json.environments[].usbipBusidLocks[].vendorProductAllowlist` row. Use hex strings such as `0x1050` / `0x0407` to allow only specific hardware families even when busids change across replug events. |

Site-level dependency:

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.site.yubikey.enable` | bool | `true` | Host-side Yubikey support: Yubico udev rules for vendor `1050` (GROUP=kvm, MODE=0660, TAG+=uaccess). The `usbip-host` kernel module is loaded only when this option is on **and** at least one enabled VM sets `usbip.yubikey = true`. Set `false` on hosts that do not use YubiKeys — per-VM `usbip.yubikey = true` still pulls in the guest-side bits, but the host has no Yubikey-specific machinery loaded. |

## Options (guest-side propagation)

None. The component module is imported directly into the guest's
config by `host.nix` (`++ lib.optional vm'.usbip.yubikey
./components/usbip.nix`).

## Host-side resources created

Per opted-in env (declared in [`network.nix`](../../nixos-modules/network.nix); materialized only when `nixling.site.yubikey.enable = true` and at least one enabled VM in that env sets `usbip.yubikey = true`):

> There are no `nixling-sys-<env>-usbipd-{backend,proxy}` systemd
> units. The broker spawns backend/proxy runners under
> `nixling.slice/sys-<env>/usbipd-*`, and the hardening shape
> documented below is enforced as the runner contract.
>
> `ModprobeIfAllowed{module: "usbip-host"}` runs before the first
> `UsbipBackend` runner for each env. Per-attach `usbip bind` /
> `unbind` / `attach` / `detach` steps run as broker-spawned
> one-shot runners with per-env busid locking, pidfd handoff, and
> audit coverage.

- **`nixling.slice/sys-<env>/usbipd-backend` runner** — runs
  `usbipd -4 --tcp-port <backendPort>`. usbipd has no `--host` flag
  so it binds to `0.0.0.0`; the broker-managed `inet nixling`
  `input` chain drops non-loopback ingress to each backend port, so
  the effective path is host-local proxy → `127.0.0.1:<backendPort>`.
  Pre-spawn: host-prep DAG op `ModprobeIfAllowed{module:
  "usbip-host"}`. The broker runs this root-only backend in a private
  mount + PID namespace with seccomp, `CAP_NET_RAW` only, masked host
  secret directories, a fresh procfs, a masked `/dev`, and only the
  locked USB device node visible.
- **`nixling.slice/sys-<env>/usbipd-proxy` runner** —
  `socat TCP-LISTEN:3240,bind=<env.hostUplinkIp>,fork,max-children=4,reuseaddr
  TCP:127.0.0.1:<backendPort>`. Requires + after the matching backend
  runner. `CapabilityBoundingSet = ""`.

Firewall carve-outs (canonical `inet nixling` table per
[ADR 0013](../adr/0013-w3-firewall-coexistence-policy.md) +
[`inet-nixling-chains.md`](./inet-nixling-chains.md)):

The broker emits these source-based carve-outs through the existing
`UsbipBindFirewallRule` broker op. Carve-out removal is performed by
re-invoking `UsbipBindFirewallRule` with a `destroy: true` payload
field; there is no separate firewall-unbind op. The carve-outs land
in the canonical `forward` chain inside the `inet nixling` table
BEFORE the generic allow/drop rule. The carve-out matrix translates
the legacy iptables semantics 1:1:

- DROP source ≠ 127.0.0.1 to the env's backend loopback port.
- DROP source ∉ `<env.uplinkSubnet>` to TCP 3240 on the env's
  uplink bridge.
- ACCEPT source ∈ `<env.uplinkSubnet>` to TCP 3240 on the env's
  uplink bridge.

The legacy iptables `nixos-fw` rules (an interim implementation
that inserted at position 1 in `nixos-fw` to win first-match
against NixOS's generated accepts) were retired in favour of the
daemon-owned broker `inet nixling` table.
Implementations MUST emit via the broker `UsbipBindFirewallRule`
broker op so the carve-out ordering is enforced by
`nixling_host::nftables::NftBatch::assert_carveout_ordering`.
The op is invoked by the host-prep DAG (before the
`UsbipBackend` SpawnRunner starts for each env) and by the
per-attach state machine (before `UsbipBindOneShot` SpawnRunner
runs); see [ADR 0018](../adr/0018-microvm-nix-removal.md) §
"Disposition matrix" USBIP row for the full lifecycle.

Per host (in [`host.nix`](../../nixos-modules/host.nix)):

- When `nixling.site.yubikey.enable = true`, udev rules for vendor
  `1050` on `hidraw` + `usb` subsystems:
  `GROUP="kvm" MODE="0660" TAG+="uaccess"`.
- `boot.kernelModules += [ "usbip-host" ]` only when
  `nixling.site.yubikey.enable = true` **and** at least one enabled
  VM sets `usbip.yubikey = true`.
- The `/dev/kvm` lock-down rule (`KERNEL=="kvm", GROUP="kvm",
  MODE="0660"`) is unconditional and not part of the yubikey gate.

CLI (`nixling usb attach|detach|probe` in the Rust CLI).

- Scans `/sys/bus/usb/devices/*/idVendor` for `1050`.
- Acquires an exclusive flock on `/run/nixling/usbipd.lock` (mode
  `0660 root:nixling`, created by tmpfiles).
- Stops other envs' usbipd backend/proxy runners so the device is
  bound in exactly one env at a time (broker `SignalRunner` against
  the per-env DAG leaves under `nixling.slice/sys-<env>/usbipd-*`).
- `usbip bind -b <busid>` on the host, `usbip attach -r <hostIp> -b
  <busid>` inside the VM via SSH, holds the foreground until Ctrl-C,
  then detaches + unbinds.

## Guest-side resources created

The entire `components/usbip.nix` is two lines of payload:

```nix
{
  boot.kernelModules = [ "vhci_hcd" ];
  environment.systemPackages = [ pkgs.linuxPackages.usbip ];
}
```

- `vhci_hcd` lets `usbip attach` materialise the redirected device
  as `/dev/hidraw<N>` (or a raw USB node) inside the guest kernel.
- The `usbip` CLI is needed in-guest so the host-side `nixling usb
  attach|detach` Rust CLI can SSH in and issue `usbip attach` / `usbip
  detach`. Host-side `usbip bind/unbind`, firewall, and proxy
  reconciliation still dispatch through the daemon → broker path; the
  guest import uses the framework-managed SSH key and known-hosts file.

## Runtime invariants

- The YubiKey is exposed to at most one env at any moment. The
  flock + the cross-env "stop other proxies" step in the
  exclusive-attach path is the enforcement mechanism; switching
  to another env requires re-running `nixling usb attach <vm>`,
  which steals the lock and detaches first. The same enforcement now
  lives in the Rust CLI's `nixling usb attach` dispatch through the
  broker.
- The host-side proxy listens only on `<env.hostUplinkIp>:3240`. A
  workload VM in env A cannot reach env B's usbipd via routing —
  the nftables `ACCEPT source ∈ <env.uplinkSubnet>` carve-out (per
  ADR 0013 + this doc's "Firewall carve-outs" section above) keys
  on the env's own uplink subnet.
- The guest's `usbip attach` connects to its own env's
  `usbipdHostIp` (the host-side end of that env's uplink bridge),
  not the host's WAN address.

## Hardening notes

- Backend listens on TCP `<backendPort>` (bound to `0.0.0.0`
  because usbipd has no `--host` flag); the broker-managed nftables
  `inet nixling` input chain explicitly drops non-loopback ingress to
  backend ports, making the backend effectively loopback-only even
  though the socket itself is all-interface-bound. The cross-env proxy
  is a bounded `socat` instance with an empty capability set.
- Backend retains only the USBIP backend capability set. The proxy runs
  with an empty capability set and listens only on `<env.hostUplinkIp>:3240`.
- The kvm-group YubiKey udev grant (`GROUP="kvm" MODE="0660"`) is
  the smallest set that lets `usbip bind` work without `sudo` for
  the launcher user. The `usbip bind/unbind` step itself is
  dispatched through the broker `SpawnRunner` runner on the per-env
  DAG (`nixling.slice/sys-<env>/usbip-bind`), so no `sudo`
  escalation is required.
- The `nixling-<vm>-gpu` user is not in the kvm group strictly for
  USB; it's there for `/dev/kvm`. USBIP traffic flows over TCP, not
  device-node ACLs.

## Common gotchas / failure modes

- **`nixling usb attach <vm> <busid> --apply` failing with "no Yubico USB device".** The
  host has no `1050:*` device plugged in, or `nixling.site.yubikey
  .enable = false` and the udev rules are absent. Plug the key in,
  or flip the site flag.
- **`nixling usb attach <vm> <busid> --apply` failing with "VM at <ip> is not reachable".**
  The target VM has not been started — run `nixling vm start <vm>` first.
  Also requires the VM to have `staticIp`, `ssh.user`, and
  `ssh.keyPath` resolvable. That stable address is for operator
  convenience, not anti-spoofing; see the
  [design threat-model note](../explanation/design.md).
- **`usbip attach` succeeds but the YubiKey doesn't appear in
  `lsusb` inside the VM.** `vhci_hcd` failed to load — check
  `dmesg` in the guest. Verify the component is enabled
  (`usbip.yubikey = true`) so the module pulls in the kernel
  module.
- **Cross-env interference.** Running `nixling usb <vm-in-env-A>`
  while another env has the key attached steals the lock and stops
  env B's proxy units. This is intentional but can surprise
  multi-env users; expect a brief disconnect on the previous env.

## See also

- [Design / threat model](../explanation/design.md)
- [Manifest schema](./manifest-schema.md) — `units.usbipBackend` /
  `units.usbipProxy` (per-env, not per-VM).
- [CLI contract](./cli-contract.md) — `nixling usb attach|detach|probe` subcommands.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  end-to-end example with `usbip.yubikey = true`.
- [CHANGELOG.md](../../CHANGELOG.md) — release history for USBIP gating and related fixes.
