# `examples/minimal` — single headless VM, single env

The "is d2b for me?" sanity test. About 25 lines of `flake.nix`
plus a small `configuration.nix` get you:

- one isolated network environment named `personal`,
- one headless workload VM named `personal-dev` joined to that env,
- and the full per-env plumbing rendered around them — bridges,
  an auto-declared net VM, dnsmasq, NAT, USBIP proxy.

No graphics, no audio, no TPM, no USBIP — those are layered on top
in the `graphics-workstation` example. Start here.

## The flake (25 lines)

```nix
{
  description = "Minimal d2b example — one headless workload VM in one env";

  inputs = {
    d2b.url   = "github:vicondoa/d2b/v0.1.0";  # ← use this in real consumers
    nixpkgs.follows = "d2b/nixpkgs";
  };

  outputs = { self, nixpkgs, d2b, ... }: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

Two things to note:

1. **`d2b.nixosModules.default` is the entire framework surface.**
   Importing it lights up `d2b.site`, `d2b.envs.<env>`, and
   `d2b.vms.<name>` options under your top-level config.
2. **`nixpkgs.follows = "d2b/nixpkgs"`.** Sharing d2b's pinned
   nixpkgs keeps option types aligned between framework and consumer.
   Mixing nixpkgs revisions is a common source of subtle eval errors.

> **Note on the in-tree path** — the version of `flake.nix` checked
> into this directory uses `d2b.url = "path:../..";` so the
> example can be evaluated against the in-tree framework without a
> network. When you copy this layout into your own repo, swap it for
> a real flake ref (`github:vicondoa/d2b/v0.1.0` or a pinned
> revision).

## The consumer config

`configuration.nix` is split into three labelled sections:

```nix
d2b.site = {
  waylandUser = null;            # ← headless: no Wayland forwarding
  launcherUsers = [ ];
  yubikey.enable = false;
};

d2b.envs.personal = {
  lanSubnet    = "10.99.0.0/24"; # workload VMs land in here
  uplinkSubnet = "192.0.2.0/30"; # point-to-point host ↔ net VM
};

d2b.vms.personal-dev = {
  enable   = true;
  env      = "personal";         # bind to the env above
  index    = 10;                 # → 10.99.0.10
  ssh.user = "alice";
  config = {                     # NixOS module merged into the GUEST
    networking.hostName = "personal-dev";
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

Declaring just `d2b.envs.personal = { … };` and one workload VM
expands into a surprisingly large amount of host plumbing. After
the rebuild, on the host you will find:

| Resource                                           | Purpose                                                                 |
|----------------------------------------------------|-------------------------------------------------------------------------|
| `br-personal-up`                                       | /30 point-to-point bridge: host `.1` ↔ net VM `.2`.                     |
| `br-personal-lan`                                      | /24 LAN bridge: net VM `.1` ↔ workload VMs `.10–.250`. **Host has no IP on this bridge.** |
| `sys-personal-net` (microVM)                           | Auto-declared headless net VM. Runs NAT, dnsmasq, and the per-env firewall blocklist. Set to `autostart = true`. |
| `personal-dev` (microVM)                                | Your declared workload VM. Tap on `br-personal-lan`, IP `10.99.0.10`, DHCP-driven inside the guest. |
| USBIP runners                                           | Not materialised by this headless starter unless a VM opts into `usbip.yubikey = true`; see the USBIP reference/how-to before adding YubiKey passthrough. |
| Per-VM store farm                                  | Daemon-owned hardlink farm under `/var/lib/d2b/vms/<vm>/store/` mirroring each VM's closure. |
| `/var/lib/d2b/keys/personal-dev_ed25519`       | Framework-managed Ed25519 key for SSH into `personal-dev`. Regenerated on activation if missing. |
| `d2b` CLI on `$PATH`                           | `d2b list` shows declared VMs + env metadata; `d2b switch personal-dev --apply` rebuilds and live-applies inside the running VM. |

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
  set `d2b.site.waylandUser`, then flip `graphics.enable`,
  `audio.enable`, and `usbip.yubikey` on a workload VM.
- **Add a second env** — `examples/multi-env` demonstrates two
  parallel `d2b.envs.<name>` instances with no cross-traffic
  between them.
- **Add Entra ID** — `examples/with-entra-id` consumes the sibling
  `entrablau` flake to put a domain-joined VM behind d2b
  without the framework knowing about Himmelblau.

## After activation

After `sudo nixos-rebuild switch`, the host has the env's bridges
up and the auto-declared net VM running. The single workload VM
is **not** autostarted.

```bash
d2b list
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# personal-dev       personal      false     false false   10.99.0.10      stopped
# sys-personal-net   personal  false     false false   192.0.2.2       running (net-vm)

d2b status
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# personal-dev       personal      false     false false   10.99.0.10      stopped
# sys-personal-net   personal  false     false false   192.0.2.2       running (net-vm)
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-personal-up           UP         up      UP           ok
# br-personal-lan          NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)

# STATUS legend:
#   running      — supervised by d2bd with a live runner.
#                  Net VMs are tagged `running (net-vm)`.
#   stopped      — not running.

d2b vm start personal-dev --apply
ssh -i /var/lib/d2b/keys/personal-dev_ed25519 alice@10.99.0.10 hostname
# personal-dev

d2b vm stop personal-dev --apply
```

## Common gotchas

- **`waylandUser = null` is mandatory while no VM uses graphics
  or audio.** The moment you flip `graphics.enable = true` or
  `audio.enable = true` on any VM without setting `waylandUser`
  to a real user, eval fails. That's the assertion gate the
  example deliberately exercises.
- **`/var/lib/d2b` MUST be on the same filesystem as
  `/nix/store`.** The per-VM `/nix/store` is a hardlink farm; a
  cross-FS layout fails with a fatal error from
  `d2b-store-sync`.
- **CIDR overlap is an eval error.** `lanSubnet` and
  `uplinkSubnet` must be disjoint from each other, from any
  other env, and from `d2b.hostLanCidrs`.
- **The framework key under `/var/lib/d2b/keys/` is the only
  way `d2b` itself talks to the VM.** Removing it forces a
  fresh keypair on next activation.

## After subsequent rebuilds

`nixos-rebuild switch` updates the declared d2b bundle and may
restart `d2bd`, but daemon restarts are continuation events:
running VM runners are re-adopted rather than cycled. After rebuilding,
`d2b list` flags any VM whose declared closure has drifted from the
running one as `[pending restart]`; apply with `d2b vm restart
<vm> --apply`. See
[`templates/default/README.md` — After every subsequent rebuild](../../templates/default/README.md#after-every-subsequent-rebuild)
for the recommended workflow and
[`docs/reference/cli-contract.md`](../../docs/reference/cli-contract.md#pending-restart-signal-v015)
for the exact predicate.

## See also

- [`examples/graphics-workstation`](../graphics-workstation/) — desktop VM with Wayland + audio + USBIP
- [`examples/multi-env`](../multi-env/) — two isolated envs (work + personal)
- [`examples/with-entra-id`](../with-entra-id/) — Entra-ID composition via the sibling flake
- [`templates/default`](../../templates/default/) — scaffold via `nix flake init`
