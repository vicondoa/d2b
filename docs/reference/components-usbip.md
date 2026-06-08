# `nixling.vms.<vm>.usbip.*`

> Reference for the `usbip` component module (YubiKey passthrough).
> Source: [`nixos-modules/components/usbip.nix`](../../nixos-modules/components/usbip.nix)
> Host-side wiring: [`nixos-modules/network.nix`](../../nixos-modules/network.nix), [`nixos-modules/host.nix`](../../nixos-modules/host.nix)
> CLI integration: [`packages/nixling/src/lib.rs`](../../packages/nixling/src/lib.rs) (`nixling usb attach|detach|probe`). The pre-P6 bash CLI helper that lived in `nixos-modules/cli.nix` was deleted in P6 per ADR 0015.

## What this component does

Enables on-demand passthrough of a host-side YubiKey (USB vendor ID
`1050`) into a VM via USBIP. When `nixling.site.yubikey.enable = true`
and some enabled VM in an env sets `usbip.yubikey = true`, the host
materializes a per-env `usbipd` backend listening on TCP
`<backendPort>` (usbipd has no `--host` flag, so it binds to
`0.0.0.0`; firewall rules — see "Host-side resources" — restrict
source addresses to host loopback, so it's the operational equivalent
of a loopback bind but enforced via netfilter rather than by the
socket). A `systemd-socket-proxyd` front then binds exactly the env's
uplink-bridge IP at TCP 3240; the guest loads `vhci_hcd` and ships the
`usbip` CLI so it can `usbip attach` against that proxy. The hot-plug
ceremony — bind on host, attach in VM, detach + unbind on Ctrl-C — is
orchestrated by the host-side `nixling usb attach|detach|probe` CLI
surface dispatched through the v1.0 daemon → broker `SpawnRunner`
path (per ADR 0015). The pre-P6 interactive `nixling usb <vm>` bash
helper was retired in P6.

The component itself only declares the **guest-side** wiring. All
host-side machinery (usbipd backend + proxy broker-spawned runners,
udev rules, firewall rules, the `usbip-host` kernel module) lives
elsewhere — see "Host-side resources" below.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.usbip.yubikey` | bool | `false` | YubiKey USBIP passthrough opt-in for this VM. Loads `vhci_hcd` in the guest and installs `usbip` so the USB CLI can redirect a plugged-in Yubico device. |
| `nixling.vms.<vm>.usbip.busids` | list of string | `[]` | Exact USBIP busids the daemon should advertise for this VM in `host.json.environments[].usbipBusidLocks[].busIds`. Leave empty to preserve the legacy `pending` fallback for v0.4-era fixtures. |
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

> **v1.0 status (per [ADR 0015](../adr/0015-daemon-only-clean-break.md)):**
> the pre-P6 `nixling-sys-<env>-usbipd-{backend,proxy}.{socket,service}`
> systemd units were retired in P6 and respawned by the broker's
> `SpawnRunner` DAG under `nixling.slice/sys-<env>/usbipd-*` cgroup
> leaves. The hardening shape (caps, RestrictAddressFamilies, etc.)
> documented below is preserved as the minijail-profile contract
> the broker enforces on the runner spawn — the difference is the
> supervisor (broker pidfd table instead of systemd's service
> manager). The bullets below use the historical systemd unit
> identifiers for traceability with the cgroup leaf names.

- **`nixling.slice/sys-<env>/usbipd-backend` runner** (pre-P6:
  `nixling-sys-<env>-usbipd-backend.service`) — runs
  `usbipd -4 --tcp-port <backendPort>`. usbipd has no `--host` flag
  so it binds to `0.0.0.0`; the iptables rules below (DROP source ≠
  127.0.0.1 to the backend port) restrict effective reachability to
  host loopback. Pre-spawn: `modprobe usbip-host`. Confined
  with `NoNewPrivileges`, `CapabilityBoundingSet = "CAP_NET_BIND_SERVICE
  CAP_NET_RAW"`, `RestrictAddressFamilies = "AF_INET AF_INET6
  AF_UNIX AF_NETLINK"`, `LockPersonality`.
- **`nixling.slice/sys-<env>/usbipd-proxy` socket** (pre-P6:
  `nixling-sys-<env>-usbipd-proxy.socket`) — `ListenStream =
  <env.hostUplinkIp>:3240`. Binds *exactly* the env's uplink-bridge
  IP, so usbipd is unreachable on the WAN interface or any other
  address.
- **`nixling.slice/sys-<env>/usbipd-proxy` runner** (pre-P6:
  `nixling-sys-<env>-usbipd-proxy.service`) —
  `systemd-socket-proxyd 127.0.0.1:<backendPort>`. Requires +
  after the matching backend runner. `CapabilityBoundingSet = ""`.

Iptables (inserted at position 1 in `nixos-fw`, so they win first-
match against NixOS's generated accepts):

- DROP source ≠ 127.0.0.1 to the env's backend loopback port.
- DROP source ∉ `<env.uplinkSubnet>` to TCP 3240 on the env's
  uplink bridge.
- ACCEPT source ∈ `<env.uplinkSubnet>` to TCP 3240 on the env's
  uplink bridge.

Per host (in [`host.nix`](../../nixos-modules/host.nix)):

- When `nixling.site.yubikey.enable = true`, udev rules for vendor
  `1050` on `hidraw` + `usb` subsystems:
  `GROUP="kvm" MODE="0660" TAG+="uaccess"`.
- `boot.kernelModules += [ "usbip-host" ]` only when
  `nixling.site.yubikey.enable = true` **and** at least one enabled
  VM sets `usbip.yubikey = true`.
- The `/dev/kvm` lock-down rule (`KERNEL=="kvm", GROUP="kvm",
  MODE="0660"`) is unconditional and not part of the yubikey gate.

CLI (`nixling usb attach|detach|probe` in the Rust CLI). The pre-P6 `nixling usb <vm>` bash helper in `cli.nix` was retired in P6 per ADR 0015.

- Scans `/sys/bus/usb/devices/*/idVendor` for `1050`.
- Acquires an exclusive flock on `/run/nixling/usbipd.lock` (mode
  0660 root:nixling-launchers, created by tmpfiles in v1.0; the
  pre-P6 singular `nixling-launcher` group was renamed in P6 per
  ADR 0015).
- Stops other envs' usbipd backend/proxy runners so the device is
  bound in exactly one env at a time (v1.0: broker `SignalRunner`
  against the per-env DAG leaves under `nixling.slice/sys-<env>/usbipd-*`;
  the pre-P6 per-env `nixling-sys-<env>-usbipd-{backend,proxy}.service`
  systemd units were retired in P6 per ADR 0015).
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
  detach`. (The pre-P6 `nixling usb <vm>` bash orchestrator was
  retired in P6 per ADR 0015; the v1.0 path dispatches the same
  in-guest SSH calls through the daemon → broker `SpawnRunner` for
  the host-side `usbip bind/unbind` step.)

## Runtime invariants

- The YubiKey is exposed to at most one env at any moment. The
  flock + the cross-env "stop other proxies" step in the
  exclusive-attach path is the enforcement mechanism; switching
  to another env requires re-running `nixling usb attach <vm>`,
  which steals the lock and detaches first. (The pre-P6
  `usbip_exclusive_attach` bash helper in `cli.nix` was retired
  in P6 per ADR 0015; the same enforcement now lives in the
  Rust CLI's `nixling usb attach` dispatch through the broker.)
- The host-side proxy listens only on `<env.hostUplinkIp>:3240`. A
  workload VM in env A cannot reach env B's usbipd via routing —
  the iptables ACCEPT rule keys on the env's own uplink subnet.
- The guest's `usbip attach` connects to its own env's
  `usbipdHostIp` (the host-side end of that env's uplink bridge),
  not the host's WAN address.

## Hardening notes

- Backend listens on TCP `<backendPort>` (bound to `0.0.0.0`
  because usbipd has no `--host` flag); the iptables rule set
  restricts source to 127.0.0.1, making the backend effectively
  loopback-bound via netfilter rather than socket bind. The
  cross-env proxy is a `systemd-socket-proxyd` instance with
  `CapabilityBoundingSet = ""`.
- Backend retains only `CAP_NET_BIND_SERVICE CAP_NET_RAW` and
  `RestrictAddressFamilies` to IP + UNIX + NETLINK.
- The kvm-group YubiKey udev grant (`GROUP="kvm" MODE="0660"`) is
  the smallest set that lets `usbip bind` work without `sudo` for
  the launcher user. In v1.0 (per ADR 0015 daemon-only) the
  `usbip bind/unbind` step itself is dispatched through the broker
  `SpawnRunner` runner on the per-env DAG
  (`nixling.slice/sys-<env>/usbip-bind`), so no `sudo` escalation is
  required. The pre-P6 polkit-based shortcut where the CLI ran
  `sudo usbip bind/unbind` directly was retired in P6 along with the
  `nixling-launcher` (singular) allowlist.
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
