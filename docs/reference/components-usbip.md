# `nixling.vms.<vm>.usbip.*`

> Reference for the `usbip` component module (YubiKey passthrough).
> Source: [`nixos-modules/components/usbip.nix`](../../nixos-modules/components/usbip.nix)
> Host-side wiring: [`nixos-modules/network.nix`](../../nixos-modules/network.nix) (lines 484–650), [`nixos-modules/host.nix`](../../nixos-modules/host.nix)
> CLI integration: [`nixos-modules/cli.nix`](../../nixos-modules/cli.nix) (`do_usb`)

## What this component does

Enables on-demand passthrough of a host-side YubiKey (USB vendor ID
`1050`) into a VM via USBIP. The host runs a per-env `usbipd` backend
listening on TCP `<backendPort>` (usbipd has no `--host` flag, so it
binds to `0.0.0.0`; firewall rules — see "Host-side resources" —
restrict source addresses to host loopback, so it's the operational
equivalent of a loopback bind but enforced via netfilter rather than
by the socket). A `systemd-socket-proxyd` front then binds exactly
the env's uplink-bridge IP at TCP 3240; the guest loads
`vhci_hcd` and ships the `usbip` CLI so it can `usbip attach` against
that proxy. The hot-plug ceremony — bind on host, attach in VM,
detach + unbind on Ctrl-C — is orchestrated by the host-side
`nixling usb <vm>` CLI subcommand.

The component itself only declares the **guest-side** wiring. All
host-side machinery (usbipd backend + proxy units, udev rules,
firewall rules, the `usbip-host` kernel module) lives elsewhere — see
"Host-side resources" below.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.usbip.yubikey` | bool | `false` | YubiKey USBIP passthrough opt-in for this VM. Loads `vhci_hcd` in the guest and installs `usbip` so `nixling usb <vm>` can redirect a plugged-in Yubico device. |

Site-level dependency:

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.site.yubikey.enable` | bool | `true` | Host-side Yubikey support: udev rules for vendor `1050` (GROUP=kvm, MODE=0660, TAG+=uaccess) AND the `usbip-host` kernel module. Set `false` on hosts that do not use YubiKeys — per-VM `usbip.yubikey = true` still pulls in the guest-side bits, but the host has no Yubikey-specific machinery loaded. |

## Options (guest-side propagation)

None. The component module is imported directly into the guest's
config by `host.nix` (`++ lib.optional vm'.usbip.yubikey
./components/usbip.nix`).

## Host-side resources created

Per env (declared in [`network.nix`](../../nixos-modules/network.nix)):

- **`nixling-sys-<env>-usbipd-backend.service`** — runs
  `usbipd -4 --tcp-port <backendPort>`. usbipd has no `--host` flag
  so it binds to `0.0.0.0`; the iptables rules below (DROP source ≠
  127.0.0.1 to the backend port) restrict effective reachability to
  host loopback. `ExecStartPre` runs `modprobe usbip-host`. Confined
  with `NoNewPrivileges`, `CapabilityBoundingSet = "CAP_NET_BIND_SERVICE
  CAP_NET_RAW"`, `RestrictAddressFamilies = "AF_INET AF_INET6
  AF_UNIX AF_NETLINK"`, `LockPersonality`.
- **`nixling-sys-<env>-usbipd-proxy.socket`** — `ListenStream =
  <env.hostUplinkIp>:3240`. Binds *exactly* the env's uplink-bridge
  IP, so usbipd is unreachable on the WAN interface or any other
  address. `wantedBy = [ "sockets.target" ]`.
- **`nixling-sys-<env>-usbipd-proxy.service`** —
  `systemd-socket-proxyd 127.0.0.1:<backendPort>`. `requires` +
  `after` the matching backend service. `CapabilityBoundingSet = ""`.

Iptables (inserted at position 1 in `nixos-fw`, so they win first-
match against NixOS's generated accepts):

- DROP source ≠ 127.0.0.1 to the env's backend loopback port.
- DROP source ∉ `<env.uplinkSubnet>` to TCP 3240 on the env's
  uplink bridge.
- ACCEPT source ∈ `<env.uplinkSubnet>` to TCP 3240 on the env's
  uplink bridge.

Per host (in [`host.nix`](../../nixos-modules/host.nix)), gated on
`nixling.site.yubikey.enable`:

- udev rules for vendor `1050` on `hidraw` + `usb` subsystems:
  `GROUP="kvm" MODE="0660" TAG+="uaccess"`.
- `boot.kernelModules += [ "usbip-host" ]`.
- The `/dev/kvm` lock-down rule (`KERNEL=="kvm", GROUP="kvm",
  MODE="0660"`) is unconditional and not part of the yubikey gate.

CLI (`nixling usb <vm>` in `cli.nix`):

- Scans `/sys/bus/usb/devices/*/idVendor` for `1050`.
- Acquires an exclusive flock on `/run/nixling/usbipd.lock` (mode
  0660 root:nixling-launcher, created by tmpfiles).
- Stops other envs' usbipd proxy/backend units so the device is
  bound in exactly one env at a time.
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
  <vm>` orchestrator can SSH in and issue `usbip attach` / `usbip
  detach`.

## Runtime invariants

- The YubiKey is exposed to at most one env at any moment. The
  flock + the cross-env "stop other proxies" step in
  `usbip_exclusive_attach` is the enforcement mechanism; switching
  to another env requires re-running `nixling usb <other-vm>`,
  which steals the lock and detaches first.
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
  the launcher user. The `nixling-launcher` polkit allowlist does
  NOT include `usbipd` — the CLI escalates via `sudo usbip
  bind/unbind` for the bind step itself.
- The `nixling-<vm>-gpu` user is not in the kvm group strictly for
  USB; it's there for `/dev/kvm`. USBIP traffic flows over TCP, not
  device-node ACLs.

## Common gotchas / failure modes

- **Known gap: per-env usbipd units materialise even when no VM
  opts in.** Each `nixling.envs.<env>` declares
  `nixling-sys-<env>-usbipd-backend.service` and the matching proxy
  socket unconditionally — regardless of whether any workload VM
  in that env sets `usbip.yubikey = true`. The units are idle when
  nothing opts in, but they are still installed. Conditional
  materialisation is tracked for v0.2.0; the relevant code is in
  [`network.nix:484-650`](../../nixos-modules/network.nix). See the
  ["Known gaps" section of CHANGELOG.md](../../CHANGELOG.md) for the
  authoritative entry.
- **`nixling usb <vm>` failing with "no Yubico USB device".** The
  host has no `1050:*` device plugged in, or `nixling.site.yubikey
  .enable = false` and the udev rules are absent. Plug the key in,
  or flip the site flag.
- **`nixling usb <vm>` failing with "VM at <ip> is not reachable".**
  The target VM has not been started — run `nixling up <vm>` first.
  Also requires the VM to have `staticIp`, `ssh.user`, and
  `ssh.keyPath` resolvable.
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
- [CLI contract](./cli-contract.md) — `nixling usb <vm>` subcommand.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  end-to-end example with `usbip.yubikey = true`.
- [CHANGELOG.md](../../CHANGELOG.md) — "Known gaps" section for the
  unconditional-per-env-materialisation tracking.
