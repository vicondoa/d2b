# nixling

> ⚠️ **Alpha — v0.1.0 not yet released.** This repo is the result of an
> in-flight refactor extracting nixling from a personal NixOS host into
> a standalone reusable flake. APIs will change before the first tagged
> release.

**Nixling is opinionated NixOS desktop microVM workspaces on top of
[microvm.nix].** ("microVM" = a Linux VM booted via lightweight VMMs
like cloud-hypervisor or crosvm.) It adds, on top of raw microvm.nix:

- A single `nixling` CLI for daily VM ops (`up`, `down`, `status`,
  `switch`, `keys rotate`, `audio …`, `usb …`).
- Per-VM systemd-isolated sidecars (GPU forward, audio mediation,
  TPM emulation, virtiofsd) running as dedicated system users.
- Per-environment isolated networks (point-to-point uplink + LAN
  bridge + auto-declared NAT/DHCP "net VM" + firewall).
- Per-VM `/nix/store` hardlink farm so each guest sees only its
  own closure.
- Nixling-managed Ed25519 SSH keys, generated and rotated per VM
  at activation time.
- A documented JSON manifest contract sized for a future Rust CLI
  port.

**Quickest path to a working host** (full walkthrough under
[Quick start (template path)](#quick-start-template-path) below):

```bash
mkdir my-nixling-host && cd my-nixling-host
nix flake init -t github:vicondoa/nixling   # scaffolds a ~150-line host config
# edit configuration.nix — 7 numbered TODOs, assertions gate the hard ones
sudo nixos-rebuild switch --flake .#desktop
```

Other entry points: see [Where to start](#where-to-start) below for a
table of all four examples (`minimal`, `graphics-workstation`,
`multi-env`, `with-entra-id`) and the manual-integration path.

## Who this is for

Nixling targets the **single-user NixOS desktop** who wants
isolated workspaces — work / personal / risky-dev — on the
same machine, each in its own microVM, with the host compositor
forwarding into the guest natively over Wayland. Concretely:

- One human, one host. Multi-tenant trust boundaries are
  out of scope (see *What nixling is NOT* below).
- Wayland-native. There is no X11 fallback for graphics VMs.
- Headless workloads also work — the same `nixling.envs.<env>`
  + `nixling.vms.<vm>` shape covers CI runners or
  background-service VMs without graphics + audio bits.
- Microsoft Entra ID workspaces are supported via the sibling
  [`vicondoa/nixos-entra-id`][nixos-entra-id] flake (composed
  per-VM, not auto-imported).

If you're after a multi-tenant or production-grade VM platform,
look at raw [microvm.nix], NixOS containers, or
[Qubes OS](https://www.qubes-os.org/).

## What nixling is NOT

- **Not a multi-tenant trust boundary** against a malicious local
  launcher user. SSH keys are readable by anyone in the
  `nixling-launcher` group — see [docs/explanation/design.md] for the
  full threat model.
- **Not a server-VM platform.** Use NixOps or raw microvm.nix.
- **Not a Qubes replacement.** Nixling shares the host kernel; Qubes
  uses Xen hypervisor isolation.
- **Not OCI / container isolation.** Nixling targets full-VM
  boundaries (cloud-hypervisor / crosvm) for kernel-level
  separation between workloads; containers share the host kernel
  surface.
- **Not Spectrum OS or a full OS distribution.** Nixling is a
  framework you compose into an existing NixOS host config; it
  does not replace the host's installer, init, or filesystem
  layout.
- **Not officially supported.** Best-effort hobby project, one
  maintainer, no SLA. Pin to tagged releases.

## Project status

- **Stage:** pre-1.0, alpha
- **Maintainer:** one person
- **Tested on:** NixOS unstable. Runtime tested on `x86_64-linux`
  desktop; eval-tested for headless `aarch64-linux` (the cloud-
  hypervisor + crosvm runtime path is still x86_64-linux-only, but
  the headless eval graph is multi-arch clean).
- **CI:** flake-eval only; full E2E tests run on a private runtime
  host (the original development environment).

See [CHANGELOG.md](./CHANGELOG.md).

## Where to start

Pick the entry point that matches your situation. All four examples
and the template live in this repo; the manual integration path
below ("Manual integration") is for plugging nixling into an
existing host config.

| Path                                        | Audience                                  | Notes                                                           |
|---------------------------------------------|-------------------------------------------|-----------------------------------------------------------------|
| [`templates/default`](./templates/default)  | New host, fastest setup                   | `nix flake init -t github:vicondoa/nixling` — sentinel TODOs + assertion gates |
| [`examples/minimal`](./examples/minimal)    | Read-and-copy headless starter            | One env, one VM, ~25-line flake                                 |
| [`examples/graphics-workstation`](./examples/graphics-workstation) | Desktop VM with Wayland + audio + USBIP | Requires a compositor on the host; `waylandUser` non-null      |
| [`examples/multi-env`](./examples/multi-env) | Two isolated envs (work + personal)       | Demonstrates per-env isolation and route preflight              |
| [`examples/with-entra-id`](./examples/with-entra-id) | Entra-ID-joined VM via the sibling flake  | Composes [`vicondoa/nixos-entra-id`][nixos-entra-id]; needs swtpm + Himmelblau |

## Quick start (template path)

The fastest way to a working nixling host:

```bash
mkdir my-nixling-host && cd my-nixling-host
nix flake init -t github:vicondoa/nixling
# Edit configuration.nix — fill in the 7 numbered TODOs.
# TODOs 2-3 are eval-enforced via assertions (hostname, user,
# SSH key). TODOs 1, 5-7 (hardware, network CIDRs) ship with
# plausible defaults you must still review before activation —
# see templates/default/README.md for the full table.
sudo nixos-rebuild build  --flake .#desktop
sudo nixos-rebuild switch --flake .#desktop
nixling list                          # corp-vm + sys-work-net
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-vm            work      false     false false   10.20.0.10      stopped
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)
nixling status                        # same table + bridge-health footer
nixling up corp-vm
```

The scaffold is ~150 lines and is documented inline. See
[`templates/default/README.md`](./templates/default/README.md) for
the full TODO walk-through.

## Manual integration (without the template)

If you're plugging nixling into an existing NixOS host config
rather than starting fresh, this is the minimum surface area.

**1. Add the flake input.** In your `flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixling.url = "github:vicondoa/nixling";
    nixling.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, nixling, ... }: {
    nixosConfigurations.desktop = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

**2. Drop in a `configuration.nix` block.** This is the minimum
nixling needs from you — pick a Wayland user (alice here) plus
one env + one VM. Everything else (sidecar users, polkit grants,
SSH-key generation, dnsmasq, NAT, firewall, the auto-declared
net VM) is materialised by the framework.

```nix
# configuration.nix
{ pkgs, ... }: {
  # Alice is your Plasma / Sway / Hyprland user.
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" "video" "audio" ];
  };

  # Tell nixling about Alice + grant her the polkit-launcher group
  # so 'nixling up <vm>' works without sudo.
  nixling.site = {
    waylandUser = "alice";
    launcherUsers = [ "alice" ];
    # Set true if you have a Yubikey and want USBIP passthrough.
    yubikey.enable = false;
  };

  # One env. Two CIDRs: a /30 for the host↔net-VM uplink,
  # a /24 for workload VMs on the LAN. RFC 5737 documentation
  # ranges are safe defaults for the uplink; pick whatever
  # 10.x or 192.168.x LAN you want for the workloads.
  nixling.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  # One workload VM in the env. ssh.keyPath is left null, so the
  # framework-managed key under nixling.site.keysDir is used.
  nixling.vms.corp-vm = {
    enable = true;
    env = "work";
    index = 10;                    # workload IP = 10.20.0.10
    ssh.user = "alice";
    config = { ... }: {
      networking.hostName = "corp-vm";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
        # Inside the VM, give Alice a normal shell. The framework
        # injects the authorized SSH key automatically.
      };
    };
  };

  # Optional: declare your host's primary LAN so nixling's CIDR-
  # overlap assertion catches collisions at eval time.
  nixling.hostLanCidrs = [ "192.168.1.0/24" ];

  system.stateVersion = "25.11";
}
```

**3. Build it.**

```bash
sudo nixos-rebuild build --flake .#desktop
sudo nixos-rebuild switch --flake .#desktop
```

The activation creates `/var/lib/nixling/keys/corp-vm_ed25519`
(the framework-managed SSH key), spawns the `sys-work-net` net
VM, materialises `br-work-up` + `br-work-lan` bridges, and
installs the `nixling` CLI on your `$PATH`.

**4. Verify and use.**

```bash
nixling list                          # expect 'corp-vm' + 'sys-work-net'
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-vm            work      false     false false   10.20.0.10      stopped
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)
nixling status                        # same table + "=== Bridge health ===" footer
nixling up corp-vm                    # interactive boot
ssh -i /var/lib/nixling/keys/corp-vm_ed25519 alice@10.20.0.10 hostname
nixling down corp-vm                  # clean shutdown
```

That's it. Add a second env or a second VM by repeating the
`nixling.envs.<env>` / `nixling.vms.<name>` blocks; the framework
deals with bridges, sidecars, polkit grants, and key generation
in lockstep.

## Common gotchas

A handful of things consistently bite first-time users.

- **Same filesystem.** `/var/lib/nixling` must live on the same
  filesystem as `/nix/store`. The per-VM `/nix/store` hardlink
  farm refuses to start otherwise and there is no graceful
  fallback.
- **Wayland-only.** A graphics VM with `nixling.site.waylandUser
  = null` is an eval error. There is no X11 path; the GPU
  sidecar binds the host compositor's `/run/user/<uid>/wayland-0`
  socket directly.
- **`ssh.keyPath` default.** Leave it null and the framework-
  managed key under `${cfg.site.keysDir}/<vm>_ed25519` is used.
  Override only if you supply your own per-VM key. The CLI's
  `nixling keys rotate <vm>` only rotates the framework-managed
  key; consumer-supplied keys are untouched.
- **CIDR overlap is detected.** Two envs whose `lanSubnet` or
  `uplinkSubnet` overlap (including containment like
  `10.0.0.0/16` ⊃ `10.0.1.0/24`) is a hard eval error. Same
  for env-vs-host overlap. Pick non-overlapping ranges.
- **No autostart for graphics VMs.** `autostart = true` on a
  graphics VM is rejected — there is no Wayland session
  available at multi-user.target. Use `autostart = false` (the
  default) and `nixling up <vm>` from a Plasma terminal.
- **Nixling state is secret material.** `/var/lib/nixling/`
  contains per-VM SSH private keys and (for TPM-enabled VMs)
  swtpm state. Treat nixling state directories as secret
  material; back them up only to encrypted, access-controlled
  media.

## Companion flakes

- [`vicondoa/nixos-entra-id`][nixos-entra-id] — unofficial, framework-
  agnostic NixOS module bundle for Microsoft Entra ID auth (via
  Himmelblau) with Intune compliance shimming. Optional. Compose it
  by importing its `nixosModules.default` inside a nixling workload
  VM's `nixling.vms.<name>.config.imports`. The two flakes know
  nothing about each other — composition happens in your consumer
  flake.

## Documentation

Organised as a [Diataxis] tree under [`docs/`](docs/):

- **Tutorials / Examples** — [`examples/`](examples/) and
  [`templates/default/`](templates/default/).
- **How-to** — [`docs/how-to/migrating-from-microvm.md`](docs/how-to/migrating-from-microvm.md).
- **Reference** — [`docs/reference/`](docs/reference/): manifest
  schema, CLI contract, per-component docs (graphics, tpm, usbip,
  audio, home-manager).
- **Explanation** — [`docs/explanation/design.md`](docs/explanation/design.md):
  threat model + design rationale + *Why not X* FAQ.

For security disclosure, see [`SECURITY.md`](SECURITY.md).

### Which doc do I need?

| Goal                                  | Read                                                            |
|---------------------------------------|-----------------------------------------------------------------|
| New user, fastest start               | [`templates/default/`](templates/default/) → [`examples/minimal/`](examples/minimal/) |
| Migrating from `microvm.nix`          | [`docs/how-to/migrating-from-microvm.md`](docs/how-to/migrating-from-microvm.md) |
| Is this secure?                       | [`docs/explanation/design.md`](docs/explanation/design.md) → [`SECURITY.md`](SECURITY.md) |
| How does `<component>` work?          | [`docs/reference/components-<name>.md`](docs/reference/)        |
| Manifest contract                     | [`docs/reference/manifest-schema.md`](docs/reference/manifest-schema.md) + [`manifest-schema.json`](docs/reference/manifest-schema.json) |
| CLI behaviour (exit codes, JSON)      | [`docs/reference/cli-contract.md`](docs/reference/cli-contract.md) |

[Diataxis]: https://diataxis.fr

## License

[Apache-2.0](./LICENSE).

## Further reading

- [CHANGELOG.md](./CHANGELOG.md) — release notes and known gaps.
- [SECURITY.md](./SECURITY.md) — threat model summary and reporting
  channel.

If you are an AI agent or human contributor working on this repo,
the operational manual lives in [`AGENTS.md`](./AGENTS.md) at the
repo root.

[microvm.nix]: https://github.com/microvm-nix/microvm.nix
[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id
[docs/explanation/design.md]: ./docs/explanation/design.md
