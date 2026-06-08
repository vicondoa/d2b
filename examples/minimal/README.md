# `examples/minimal` — single headless VM, single env

The "is nixling for me?" sanity test. About 25 lines of `flake.nix`
plus a small `configuration.nix` get you:

- one isolated network environment named `work`,
- one headless workload VM named `corp-vm` joined to that env,
- and the full per-env plumbing rendered around them — bridges,
  an auto-declared net VM, dnsmasq, NAT, USBIP proxy.

No graphics, no audio, no TPM, no USBIP — those are layered on top
in the `graphics-workstation` example. Start here.

## The flake (25 lines)

```nix
{
  description = "Minimal nixling example — one headless workload VM in one env";

  inputs = {
    nixling.url   = "github:vicondoa/nixling/v0.1.0";  # ← use this in real consumers
    nixpkgs.follows = "nixling/nixpkgs";
  };

  outputs = { self, nixpkgs, nixling, ... }: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

Two things to note:

1. **`nixling.nixosModules.default` is the entire framework surface.**
   Importing it lights up `nixling.site`, `nixling.envs.<env>`, and
   `nixling.vms.<name>` options under your top-level config.
2. **`nixpkgs.follows = "nixling/nixpkgs"`.** Sharing nixling's pinned
   nixpkgs keeps option types aligned between framework and consumer.
   Mixing nixpkgs revisions is a common source of subtle eval errors.

> **Note on the in-tree path** — the version of `flake.nix` checked
> into this directory uses `nixling.url = "path:../..";` so the
> example can be evaluated against the in-tree framework without a
> network. When you copy this layout into your own repo, swap it for
> a real flake ref (`github:vicondoa/nixling/v0.1.0` or a pinned
> revision).

## The consumer config

`configuration.nix` is split into three labelled sections:

```nix
nixling.site = {
  waylandUser = null;            # ← headless: no Wayland forwarding
  launcherUsers = [ ];
  yubikey.enable = false;
};

nixling.envs.work = {
  lanSubnet    = "10.99.0.0/24"; # workload VMs land in here
  uplinkSubnet = "192.0.2.0/30"; # point-to-point host ↔ net VM
};

nixling.vms.corp-vm = {
  enable   = true;
  env      = "work";             # bind to the env above
  index    = 10;                 # → 10.99.0.10
  ssh.user = "alice";
  config = {                     # NixOS module merged into the GUEST
    networking.hostName = "corp-vm";
    users.users.alice = { isNormalUser = true; uid = 1000; };
  };
};
```

`waylandUser = null` is the explicit "I have no compositor" signal.
Flip it to a real username only when you start declaring graphics or
audio VMs — until then, leaving it null keeps the assertion gate
honest: any future VM that turns on `graphics.enable` or
`audio.enable` will fail eval until you supply a session user.

## What materialises after `nixos-rebuild switch`

Declaring just `nixling.envs.work = { … };` and one workload VM
expands into a surprisingly large amount of host plumbing. After
the rebuild, on the host you will find:

| Resource                                           | Purpose                                                                 |
|----------------------------------------------------|-------------------------------------------------------------------------|
| `br-work-up`                                       | /30 point-to-point bridge: host `.1` ↔ net VM `.2`.                     |
| `br-work-lan`                                      | /24 LAN bridge: net VM `.1` ↔ workload VMs `.10–.250`. **Host has no IP on this bridge.** |
| `sys-work-net` (microVM)                           | Auto-declared headless net VM. Runs NAT, dnsmasq, and the per-env firewall blocklist. Set to `autostart = true`. |
| `corp-vm` (microVM)                                | Your declared workload VM. Tap on `br-work-lan`, IP `10.99.0.10`, DHCP-driven inside the guest. |
| `nixling-sys-work-usbipd-proxy.service`            | Per-env USBIP proxy bound to the uplink host IP. Idle while no workload VM opts in. |
| `nixling-store-sync@*.service` + per-VM timers     | Hardlink farms under `/var/lib/nixling/<vm>/store/` mirroring each VM's closure. |
| `/var/lib/nixling/keys/corp-vm_ed25519`            | Framework-managed Ed25519 key for SSH into `corp-vm`. Regenerated on activation if missing. |
| `nixling` CLI on `$PATH`                           | `nixling list` shows declared VMs + env metadata; `nixling switch corp-vm` rebuilds and live-applies inside the running VM. |

All of that comes from the ~25-line flake plus the small consumer
config in this directory. The framework is opinionated by design;
the trade-off is that there are very few knobs left to turn before
the VM is reachable.

## Verifying the example

From inside this directory:

```bash
# Eval all flake outputs for every system declared.
nix flake check --no-build --all-systems

# Force the consumer's nixosSystem to evaluate fully and produce a
# real drvPath for the system toplevel.
nix eval --no-write-lock-file \
  .#nixosConfigurations.demo.config.system.build.toplevel.drvPath
```

Both commands should succeed without network access (the in-tree
`path:../..` reference resolves locally against the framework
checkout).

## What to do next

- **Add components** — `examples/graphics-workstation` shows how to
  set `nixling.site.waylandUser`, then flip `graphics.enable`,
  `audio.enable`, and `usbip.yubikey` on a workload VM.
- **Add a second env** — `examples/multi-env` demonstrates two
  parallel `nixling.envs.<name>` instances with no cross-traffic
  between them.
- **Add Entra ID** — `examples/with-entra-id` consumes the sibling
  `nixos-entra-id` flake to put a domain-joined VM behind nixling
  without the framework knowing about Himmelblau.

## After activation

After `sudo nixos-rebuild switch`, the host has the env's bridges
up and the auto-declared net VM running. The single workload VM
is **not** autostarted.

```bash
nixling list
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-vm            work      false     false false   10.99.0.10      stopped
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)

nixling status
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-vm            work      false     false false   10.99.0.10      stopped
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-work-up           UP         up      UP           ok
# br-work-lan          NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)

# STATUS legend:
#   systemd      — autostarted by the framework's `nixling@<vm>.service`
#                  wrapper (or the underlying `microvm@<vm>.service`).
#                  Net VMs always show this; tagged `systemd (net-vm)`.
#   interactive  — launched ad-hoc via `nixling up <vm>` from a Plasma
#                  terminal (typical for graphics VMs).
#   stopped      — not running.

nixling up corp-vm
ssh -i /var/lib/nixling/keys/corp-vm_ed25519 alice@10.99.0.10 hostname
# corp-vm

nixling down corp-vm
```

## Common gotchas

- **`waylandUser = null` is mandatory while no VM uses graphics
  or audio.** The moment you flip `graphics.enable = true` or
  `audio.enable = true` on any VM without setting `waylandUser`
  to a real user, eval fails. That's the assertion gate the
  example deliberately exercises.
- **`/var/lib/nixling` MUST be on the same filesystem as
  `/nix/store`.** The per-VM `/nix/store` is a hardlink farm; a
  cross-FS layout fails with a fatal error from
  `nixling-store-sync`.
- **CIDR overlap is an eval error.** `lanSubnet` and
  `uplinkSubnet` must be disjoint from each other, from any
  other env, and from `nixling.hostLanCidrs`.
- **The framework key under `/var/lib/nixling/keys/` is the only
  way `nixling` itself talks to the VM.** Removing it forces a
  fresh keypair on next activation.

## See also

- [`examples/graphics-workstation`](../graphics-workstation/) — desktop VM with Wayland + audio + USBIP
- [`examples/multi-env`](../multi-env/) — two isolated envs (work + personal)
- [`examples/with-entra-id`](../with-entra-id/) — Entra-ID composition via the sibling flake
- [`templates/default`](../../templates/default/) — scaffold via `nix flake init`
