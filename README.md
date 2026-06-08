# nixling

> **v1.0 daemon-only.** Every mutating verb dispatches through
> `nixlingd` → `nixling-priv-broker`; the historical bash CLI was
> retired in P6 (see [ADR 0015](docs/adr/0015-daemon-only-clean-break.md)
> and the [v0 → v1 migration guide](docs/how-to/migrate-nixling-v0-to-v1.md)).

**Nixling is opinionated NixOS desktop microVM workspaces — nixling
owns its microVM substrate end-to-end.** ("microVM" = a Linux VM
booted via lightweight VMMs like cloud-hypervisor or crosvm.) **v1.1
removed the historical [microvm.nix] foundational dependency**;
the framework owns its per-VM evaluator
(`nixos-modules/vm-evaluator.nix` + `vm-options.nix`) and spawns
every per-VM runner through `nixling-priv-broker`'s typed
`SpawnRunner` pipeline. Nixling adds:

- A single `nixling` CLI for daily VM ops (`vm start`, `vm stop`,
  `status`, `switch`, `keys rotate`, `audio …`, `usb …`).
- Per-VM broker-spawned sidecars (GPU forward, audio mediation,
  TPM emulation, virtiofsd) running as dedicated system users under
  the supervisor DAG (per-VM pidfd ownership).
- Per-environment isolated networks (point-to-point uplink + LAN
  bridge + auto-declared NAT/DHCP "net VM" + firewall).
- Per-VM `/nix/store` hardlink farm so each guest sees only its
  own closure.
- Nixling-managed Ed25519 SSH keys, generated and rotated per VM
  at activation time.
- A documented JSON manifest + bundle contract shared by the Rust
  CLI and daemon control plane.

**Rust-first quick start** (full walkthroughs under
[Quick start (Rust CLI / examples)](#quick-start-rust-cli--examples) and
[Quick start (template path)](#quick-start-template-path) below):

```bash
# after switching the host config from examples/personal-dev
sudo nixling vm start personal-dev --apply

# after switching the host config from examples/work-entra
sudo nixling vm start work-entra --apply
```

Every mutating verb is daemon-only in v1.0; there is no bash fallback
to disable (per [ADR 0015](docs/adr/0015-daemon-only-clean-break.md);
the historical W14c three-mode bridge was retired in P6).
`NIXLING_NATIVE_ONLY=1` and `NIXLING_LEGACY_BASH_OPT_IN=1` are no-ops
in v1.0; the daemon-only invariant is the default.

Other entry points: see [Where to start](#where-to-start) below for a
table of the doc-friendly example aliases (`personal-dev`,
`graphics-workstation`, `multi-env`, `work-entra`) plus the manual
integration path.

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
  `nixling-launchers` system group (the v1.0 daemon-only authz
  boundary; the broker uses `SO_PEERCRED` at accept time to
  classify peers as `launcher`/`admin`/`deny` per
  [ADR 0015](docs/adr/0015-daemon-only-clean-break.md)) — see
  [docs/explanation/design.md] for the full threat model.
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

- **Stage:** v1.0 daemon-only (per [ADR 0015](docs/adr/0015-daemon-only-clean-break.md))
- **Maintainer:** one person
- **Tested on:** NixOS unstable. Runtime tested on `x86_64-linux`
  desktop; eval-tested for headless `aarch64-linux` (the cloud-
  hypervisor + crosvm runtime path is still x86_64-linux-only, but
  the headless eval graph is multi-arch clean).
- **CI:** flake-eval only; full E2E tests run on a private runtime
  host (the original development environment).

See [CHANGELOG.md](./CHANGELOG.md).

## Where to start

Pick the entry point that matches your situation. The checked flakes
and the doc-friendly alias READMEs all live in this repo; the manual
integration path below ("Manual integration") is for plugging
nixling into an existing host config.

| Path | Audience | Notes |
| --- | --- | --- |
| [`templates/default`](./templates/default) | New host, fastest setup | `nix flake init -t github:vicondoa/nixling` — sentinel TODOs + assertion gates |
| [`examples/personal-dev`](./examples/personal-dev) | Read-and-copy headless starter | Alias of the checked [`examples/minimal`](./examples/minimal) flake; VM name `personal-dev`. |
| [`examples/graphics-workstation`](./examples/graphics-workstation) | Desktop VM with Wayland + audio + USBIP | Requires a compositor on the host; `waylandUser` must be non-null. |
| [`examples/multi-env`](./examples/multi-env) | Two isolated envs (work + personal) | Demonstrates per-env isolation and route preflight. |
| [`examples/work-entra`](./examples/work-entra) | Entra-ID-joined work VM via the sibling flake | Alias of the checked [`examples/with-entra-id`](./examples/with-entra-id) flake; VM name `work-entra`. |
| [`examples/with-observability`](./examples/with-observability) | Single-host telemetry sink + monitored workload VM | Auto-declares the `sys-obs-stack` VM (Grafana/Prometheus/Loki/Tempo) and wires per-VM Alloy agents over virtio-vsock. |

## Quick start (Rust CLI / examples)

The Rust CLI is now the primary documented operator surface. If you
want the exact names used throughout the migration docs, start from
one of these checked example layouts and use the native `vm start`
path:

```bash
# headless personal workspace (examples/personal-dev → examples/minimal)
sudo nixling vm start personal-dev --apply

# Entra workspace (examples/work-entra → examples/with-entra-id)
sudo nixling vm start work-entra --apply
```

Those alias directories exist so the README, examples index, and
migration notes can use stable VM names while CI keeps the checked
flakes in `examples/minimal` and `examples/with-entra-id`.

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
sudo nixling vm start corp-vm --apply
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
one env + one VM. Everything else (sidecar users, SSH-key
generation, dnsmasq, NAT, firewall, the auto-declared
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

  # Tell nixling about Alice + add her to the nixling-launchers
  # system group (the v1.0 daemon-only authz boundary; per
  # ADR 0015 the broker uses SO_PEERCRED at accept time to
  # classify peers — no polkit, no setuid).
  # 'nixling vm start <vm> --apply' works without sudo for
  # users in the nixling-launchers group.
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
nixling vm start corp-vm --apply      # preferred Rust CLI path
ssh -i /var/lib/nixling/keys/corp-vm_ed25519 alice@10.20.0.10 hostname
nixling vm stop corp-vm --apply       # clean shutdown
```

That's it. Add a second env or a second VM by repeating the
`nixling.envs.<env>` / `nixling.vms.<name>` blocks; the framework
deals with bridges, sidecars (broker-spawned per ADR 0015),
SSH-key generation, and dnsmasq in lockstep.

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
  default) and `nixling vm start <vm> --apply` from a Plasma
  terminal.
- **Nixling state is secret material.** `/var/lib/nixling/`
  contains per-VM SSH private keys and (for TPM-enabled VMs)
  swtpm state. Treat nixling state directories as secret
  material; back them up only to encrypted, access-controlled
  media.

## v1.0 daemon-only end-state (post-clean-break)

The Rust CLI is the only operator surface. Per
[ADR 0015](docs/adr/0015-daemon-only-clean-break.md), the v1.0
clean break:

- removed the bash CLI builder (`nixos-modules/cli.nix`),
  `scripts/`, and every bash entrypoint they shipped;
- collapsed the persistent root-visible nixling systemd footprint to
  exactly three units: `nixlingd.service`,
  `nixling-priv-broker.service`, `nixling-priv-broker.socket`;
- routes `nixling vm start|stop|restart|list --apply` exclusively
  through the daemon (no fallback; failures surface as typed
  envelopes per [`docs/reference/cli-contract.md`](docs/reference/cli-contract.md));
- retired the `NIXLING_LEGACY_BASH_OPT_IN` env-var escape hatch for
  lifecycle verbs (now a no-op);
- retired the `NIXLING_NATIVE_ONLY=1` env var (no-op in v1.0; the
  daemon-only invariant is the default per ADR 0015).

### v1.0 verb compatibility

- **Lifecycle (daemon-only in v1.0)**: `vm start`, `vm stop`,
  `vm restart`, `vm list`, plus the `up` / `down` / `restart`
  aliases.
- **Read-only daemon-backed (v1.0)**: `status`, `audit`, `auth
  status`, `host check`, `host doctor`, `host validate`,
  `keys list`, `keys show`.
- **Daemon-first with broker dispatch (v1.0)**: `switch`, `boot`,
  `test`, `rollback`, `gc`, `migrate`, `keys rotate`, `trust`,
  `rotate-known-host`, `host install`, `host prepare`,
  `host destroy`, `host reconcile`, `usb attach`, `usb detach`,
  `usb probe`.
- **Queued for v1.2+ unscheduled (v1.1 only delivers the typed-envelope rendering+remediation per ADR 0017; returns typed exit-78 envelope in v1.0)**:
  `console`, `audio status|mic|speaker|off`. The Rust subcommand
  surface (help, argument parsing, kebab-case alias compatibility)
  is preserved so operator runbooks and shell completions keep
  working; invoking the verb in v1.0 returns a guidance message
  pointing at the migration guide.

See [`docs/how-to/migrate-nixling-v0-to-v1.md`](docs/how-to/migrate-nixling-v0-to-v1.md)
for the operator migration runbook.

## v1.1 upgrade-blockers preview (planning-only — v1.1 not released yet)

> The interim notice below summarises hard blockers and
> non-blocking but action-required cleanups operators will hit
> when v1.1 lands. The authoritative migration guide for v1.1
> ships with v1.1-P12; until then the items below are the
> consolidated planning summary from the v1.1 ADRs and
> CHANGELOG. Consumers preparing v1.0→v1.1 upgrades should
> review these before pinning a v1.1 tag.

### Hard blockers (block a v1.1 upgrade until resolved)

- **Kernel floor uplift to `>= 6.9`.** v1.0 floor is 6.6 (per
  [ADR 0008](docs/adr/0008-supported-platforms-and-rejected-targets.md));
  v1.1 requires Linux 6.9+ for `pidfs`-backed pidfd identity
  (`fstat(2)` of two pidfds for the same kernel process must
  return identical `(st_dev, st_ino)`; pre-6.9
  anon_inode-backed pidfds share the same inode and the
  check is structurally impossible to satisfy). Verify with
  `uname -r`; consumers on long-term-stable distro kernels
  may need a backport pin. See
  [ADR 0008](docs/adr/0008-supported-platforms-and-rejected-targets.md)
  § "v1.1 kernel-floor uplift".
- **`nixling.vms.<vm>.supervisor` removed.** The v1.0 option is
  removed via a per-VM `mkRemovedOptionModule` shim in v1.1;
  setting it will fail eval with a typed friendly message
  pointing here. Delete every assignment of
  `nixling.vms.<vm>.supervisor` (every surviving v0.x consumer
  set it; v1.0 deferred the removal). See
  [ADR 0015](docs/adr/0015-daemon-only-clean-break.md) § Decision
  and CHANGELOG entry for v1.1-P2.
- **`microvm.nix` flake input dropped from `nixling`.** v1.0 still
  pinned `microvm.nix` as a flake input on the nixling side;
  v1.1 removes that pin entirely (every per-VM systemd-template
  surface becomes a broker `SpawnRunner` role per
  [ADR 0018](docs/adr/0018-microvm-nix-removal.md)). Action for
  consumers: if your consumer flake **only inherited** the
  `microvm` input from `nixling.inputs.microvm.follows = ...`
  (i.e., you did not directly add `microvm` as your own input),
  no consumer-side change is required — updating to nixling
  v1.1 and running `nix flake update nixling` (or
  `nix flake lock --update-input nixling` on older Nix) is
  sufficient. If your consumer flake **directly imports**
  `microvm.url` for your own use, decide whether you still
  need it: keep the direct input pinned independently of
  nixling if so, or remove it from your flake's `inputs` block
  and then run `nix flake lock` (with no `--update-input`
  argument) to re-resolve. After either path, verify
  `flake.lock` no longer carries a `microvm` entry sourced
  from nixling by inspecting `jq '.nodes' flake.lock`. The
  complete role-disposition matrix is in
  [ADR 0018](docs/adr/0018-microvm-nix-removal.md)
  § "Sidecar/template retirement — full role matrix".
- **`nixling status` schema bump v2 → v3.** v1.1 introduces
  `StatusOutputV3` (per
  [ADR 0018](docs/adr/0018-microvm-nix-removal.md) § "StatusOutputV3
  schema bump"). v1.0 consumers parsing `nixling status --json`
  output will see additional/renamed fields. v1.1 supports
  `--status-schema-version=2` as a one-release-only
  compatibility flag for tooling that needs more time to
  migrate; the flag is removed in v1.2. Audit
  `nixling status --json` consumers (dashboards, scripts) and
  either pin `--status-schema-version=2` temporarily or update
  them to V3.

### Non-blocking but action-required cleanups (warnings only — upgrade succeeds without these but eval-time noise is emitted until resolved)

- **`nixling.daemonExperimental.enable` becomes a no-op.** v1.0
  used this option to gate broker socket/service enablement;
  v1.1-P4 promotes broker enablement to default-on (broker is
  the only supervisor in v1.0+ daemon-only). The option name
  stays known to the evaluator and continues to type-check, but
  v1.1 emits a NixOS assertions/warning: "nixling.daemonExperimental.enable
  is obsolete in v1.1; remove this option from your consumer
  flake because the broker socket/service are enabled by
  default. Leaving it set has no effect." Upgrade succeeds with
  the warning present; remove the option from your flake at
  your convenience. See
  [ADR 0015](docs/adr/0015-daemon-only-clean-break.md) § Decision
  and the v1.1-P4 CHANGELOG entry.
- **No bash fallbacks (source-only).** Per
  [ADR 0017](docs/adr/0017-no-bash-fallbacks-invariant.md) v1.1
  adds compile-time and CI gates that forbid any new
  `Command::new("bash")`/`/bin/sh`/`/usr/bin/env bash` site.
  This affects only consumers who extended the Rust CLI
  source; pure consumers of the published flake are unaffected.

The bullets above are the consolidated v1.1 planning summary;
each blocker links to its authoritative ADR (single source of
truth). The [CHANGELOG](CHANGELOG.md) `Unreleased` section
enumerates the same items with ADR cross-links. When v1.1 ships,
this README section is replaced by a link to the v1.1 migration
guide.

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
- **How-to** — [`docs/how-to/`](docs/how-to/):
  [`install-nixos-tier1.md`](docs/how-to/install-nixos-tier1.md),
  [`host-prepare.md`](docs/how-to/host-prepare.md),
  [`migrating-from-microvm.md`](docs/how-to/migrating-from-microvm.md),
  [`enable-observability.md`](docs/how-to/enable-observability.md).
- **Reference** — [`docs/reference/`](docs/reference/): manifest
  schema, CLI contract, security runbook, error-envelope guidance,
  and per-component docs (graphics, tpm, usbip, audio,
  home-manager, observability).
- **Explanation** — [`docs/explanation/design.md`](docs/explanation/design.md):
  threat model + design rationale + *Why not X* FAQ.

For security disclosure, see [`SECURITY.md`](SECURITY.md).

### Which doc do I need?

| Goal                                  | Read                                                            |
|---------------------------------------|-----------------------------------------------------------------|
| New user, fastest start               | [`templates/default/`](templates/default/) → [`examples/personal-dev/`](examples/personal-dev/) |
| Migrating from `microvm.nix`          | [`docs/how-to/migrating-from-microvm.md`](docs/how-to/migrating-from-microvm.md) |
| Is this secure?                       | [`docs/explanation/design.md`](docs/explanation/design.md) → [`SECURITY.md`](SECURITY.md) |
| Security incident / USBIP emergency   | [`docs/reference/security-runbook.md`](docs/reference/security-runbook.md) |
| How does `<component>` work?          | [`docs/reference/components-<name>.md`](docs/reference/)        |
| Adding observability to an existing host | [`docs/how-to/enable-observability.md`](docs/how-to/enable-observability.md) → [`docs/reference/components-observability.md`](docs/reference/components-observability.md) |
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
