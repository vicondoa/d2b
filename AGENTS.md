# AGENTS.md

Operating manual for AI coding agents (Copilot CLI, GitHub Copilot,
Cursor, …) and human contributors working on **`vicondoa/nixling`
itself**. If you are *consuming* nixling in your own NixOS host
config, start at [README.md](./README.md) instead — this file is for
people changing the framework.

## What this is

Nixling is an opinionated NixOS desktop microVM framework built on
top of [microvm.nix]: a single CLI, per-VM systemd-isolated sidecars
(GPU / audio / swtpm / virtiofsd / store-sync), per-env isolated
networks with an auto-declared NAT/DHCP "net VM", a per-VM
`/nix/store` hardlink farm, and a documented JSON manifest contract
sized for a future Rust port. See [README.md](./README.md) and
[`docs/explanation/design.md`](./docs/explanation/design.md) for the
full picture and threat model.

[microvm.nix]: https://github.com/microvm-nix/microvm.nix

## Repo layout

```
.
├── README.md                       <- consumer-facing entry point
├── AGENTS.md                       <- this file
├── SECURITY.md                     <- disclosure policy + threat-model summary
├── CHANGELOG.md                    <- Keep a Changelog, grouped under `## Unreleased`
├── LICENSE                         <- Apache-2.0
├── flake.nix                       <- public surface: nixosModules / templates / checks
├── flake.lock
├── nixos-modules/                  <- THE framework
│   ├── default.nix                 <- aggregator imported as nixosModules.default
│   ├── options.nix / options-*.nix <- option schema (site / envs / vms)
│   ├── assertions.nix              <- eval-time invariants (CIDR overlap, platform gate, …)
│   ├── lib.nix                     <- internal helpers (subnetIp, mkMac, …)
│   ├── host.nix / host-*.nix       <- host activation, users, polkit, sidecars, keys, audit
│   ├── network.nix / net.nix       <- per-env bridges + auto-declared net VM
│   ├── store.nix                   <- per-VM /nix/store hardlink farm
│   ├── manifest.nix                <- JSON manifest emitter (versioned contract)
│   ├── cli.nix                     <- `nixling` CLI package + wrapper
│   └── components/                 <- toggleable per-VM features
│       ├── graphics.nix            <- virtio-gpu + Wayland cross-domain
│       ├── tpm.nix                 <- per-VM swtpm 2.0
│       ├── usbip.nix               <- YubiKey USBIP passthrough
│       ├── home-manager.nix        <- HM-as-NixOS-module inside the guest
│       └── audio/{guest,host}.nix  <- vhost-user-sound + PipeWire mediation
├── pkgs/                           <- patched cloud-hypervisor / crosvm / vhost-device-sound
├── scripts/                        <- helper shell utilities packaged into the CLI
├── tests/                          <- see "Test layout" below
├── examples/                       <- minimal / graphics-workstation / multi-env / with-entra-id
├── templates/default/              <- `nix flake init -t github:vicondoa/nixling`
└── docs/                           <- Diataxis tree (explanation / how-to / reference)
```

New behaviour belongs in a focused file under `nixos-modules/`
(or `nixos-modules/components/` for per-VM toggles), wired in
from `nixos-modules/default.nix`. Don't fatten existing files.

## Build & validate

Three commands cover the entire static gate. Run them from the repo
root after every change; CI runs the same set on every PR.

```bash
# 1. Flake-level eval, both systems we support.
nix flake check --no-build --all-systems

# 2. The top-level static gate. Parses every framework .nix file,
#    runs the smoke evals (workload + graphics + aarch64), exercises
#    `tests/assertions-eval.sh`, validates the manifest against its
#    JSON Schema, and iterates `nix flake check` over every example
#    flake. Runs from the repo root by default; override the root
#    with ROOT=<path> if you're driving it from elsewhere.
bash tests/static.sh

# 3. Optional focused checks (called transitively by static.sh, also
#    useful standalone while iterating):
bash tests/assertions-eval.sh        # 10 negative assertion cases
bash tests/net-vm-network-eval.sh    # net VM networkd config invariants
```

Layer-2 integration tests (`tests/nixling-store.sh`, `tests/audio.sh`)
require a live host with nixling activated; they're documented in
[`tests/README.md`](./tests/README.md). Most contributors do **not**
need to run them — Layer 1 is the PR gate.

## Development workflow

### Worktrees for parallel agents

When several agents (or several humans, or a mix) work on disjoint
scopes concurrently, use git worktrees instead of branching in
place. One worktree per agent keeps each context isolated and makes
the final merge trivial.

```bash
# From the primary clone, one worktree per concurrent scope:
git worktree add -b phase-<name> ../nixling-<name> main
```

Each agent commits inside its own worktree on its own
`phase-<name>` branch. When the scopes are genuinely disjoint
(different files, or non-overlapping regions of the same file), the
integrator does an octopus merge back to `main`:

```bash
git checkout main
git merge --no-ff phase-a phase-b phase-c
```

If two branches touch the same lines, fall back to a normal
sequential merge with conflict resolution — octopus only works for
clean disjoint scopes.

### Edit → commit → validate

Commit before running `static.sh` / the smoke evals. Two reasons:

1. Untracked files are invisible to `nix flake check` (and to any
   eval that follows the same code path). Forgetting to `git add` a
   new module is the #1 "why doesn't my change apply?" pitfall.
2. Consumer hosts that vendor nixling tend to ship auto-backup
   tooling that catch-all-commits any dirty tree. That's a
   consumer-side concern, but the habit of committing-then-building
   is the right one to carry into framework work too.

### "Existing code is canon"

When the spec, plan, README, or any reference doc disagrees with the
**code that is actually committed and passing tests**, the code
wins. Document the drift, don't silently re-align the code to the
prose.

- If you are working in a Copilot CLI session with a `plan.md`
  under `~/.copilot/session-state/<session-id>/`, add a row to the
  plan's "Spec corrections" table describing the discrepancy and
  which side you kept.
- Otherwise, mention the drift in the commit message body
  (e.g. `Spec correction: docs/reference/cli-contract.md claimed
  exit code 3 for "VM not found"; code returns 2. Kept code.`).

This rule applies to AGENTS.md too: if you change a load-bearing
behaviour described here, update this file in the same commit.

### Naming conventions

Host-visible resources follow strict naming so the CLI, the manifest,
and operators can locate them mechanically. Don't invent new shapes.

| Resource                                  | Pattern                                              |
| ----------------------------------------- | ---------------------------------------------------- |
| User-facing per-VM unit                   | `nixling@<vm>.service`                               |
| Underlying microvm.nix backend unit       | `microvm@<vm>.service`                               |
| Per-VM GPU sidecar                        | `nixling-<vm>-gpu.service`                           |
| Per-VM audio sidecar                      | `nixling-<vm>-snd.service`                           |
| Per-VM TPM emulator                       | `nixling-<vm>-swtpm.service`                         |
| Per-VM virtiofsd                          | `microvm-virtiofsd@<vm>.service` (upstream microvm.nix unit; see note below) |
| Per-VM store sync                         | `nixling-<vm>-store-sync.service`                    |
| Auto-declared per-env net VM              | `nixling@sys-<env>-net.service` (backed by `microvm@sys-<env>-net.service`) |
| Other system-level VMs (auto-declared)    | `nixling@sys-<env>-<purpose>.service`                |
| System-level (non-VM) framework services  | `nixling-sys-<purpose>.service`                      |
| System users for the above                | Same name as the corresponding service.              |
| Polkit launcher group                     | `nixling-launcher` (singleton)                       |

VM names are validated at eval time:

- Regex: `^[a-z][a-z0-9-]*$`.
- Reserved prefix: `sys-` (only the framework declares `sys-*` VMs).
- Reserved exact name: `launcher`.

Breaking any of these is a hard assertion in `nixos-modules/assertions.nix`.

> **Virtiofsd naming note.** Virtiofsd is the one per-VM sidecar
> nixling does *not* wrap into a `nixling-<vm>-*` unit. It's
> shipped as upstream microvm.nix's `microvm-virtiofsd@<vm>.service`
> template, which the CLI references directly. Renaming it to
> `nixling-<vm>-virtiofsd.service` is tracked for v0.2.0 — a clean
> rename would require refactoring microvm.nix's `Wants=` chain
> between `microvm@<vm>.service` and the virtiofsd template, plus
> the `OnFailure=` plumbing, so it's deferred until the rest of
> the framework is stable.

### Component split & sibling flakes

The **core framework** in this repo covers: graphics, tpm, usbip,
audio, network, the auto-declared net VM, the per-VM store, the
CLI, the manifest contract.

Anything **identity- or workload-specific** lives in a sibling
flake and is composed per-VM:

- [`vicondoa/nixos-entra-id`][nixos-entra-id] — Microsoft Entra ID
  joins (Himmelblau + TPM-bound machine credential).

The composition pattern is intentionally one-way: sibling flakes
know nothing about nixling, and nixling knows nothing about them.
Consumers compose them on a specific workload VM:

```nix
nixling.vms.work.config.imports = [
  inputs.nixos-entra-id.nixosModules.default
];
```

If you're tempted to add a new sibling-shaped concern (e.g. a
specific desktop environment, a particular dev-shell flavour) to
the core framework, consider whether it belongs in its own flake
instead. The bar for landing it in core is: "every nixling user
plausibly wants this, and the framework cannot do the right thing
without it."

[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id

### VM lifecycle policy (v0.1.5+)

Every per-VM lifecycle service the framework owns or touches
carries `restartIfChanged = false`. This is a framework
invariant; new sidecars added in this repo MUST carry the same
flag.

Covered services:

- `nixling@<vm>.service` ([host-wrapper.nix](nixos-modules/host-wrapper.nix))
- `microvm@<vm>.service` (upstream; overridden via host-known-hosts.nix's drop-in)
- `microvm-virtiofsd@<vm>.service` (upstream; overridden via store.nix's drop-in)
- `nixling-<vm>-swtpm.service` ([host-sidecars.nix](nixos-modules/host-sidecars.nix))
- `nixling-<vm>-snd.service` ([components/audio/host.nix](nixos-modules/components/audio/host.nix))
- `nixling-<vm>-gpu.service` ([host-sidecars.nix](nixos-modules/host-sidecars.nix))

The motivating constraint: graphics VMs run cloud-hypervisor IN
the GPU sidecar. Restarting the sidecar terminates CH and
evaporates every in-flight piece of session state. For headless
VMs the damage is smaller but still material. Consumers apply
pending changes explicitly via `nixling restart <vm>` (clean
down+up of the same closure) or `nixling switch <vm>` (per-VM
closure rebuild + live activation via SSH).

Drift detection is via two per-VM symlinks under
`/var/lib/nixling/vms/<vm>/`:

- `current` — points at the latest declared closure; updated by
  `nixos-rebuild switch`.
- `booted` — points at the closure the running VM actually
  exec'd. Updated by:
  - `microvm-set-booted@<vm>.service` for headless/net VMs (upstream).
  - The `nixling-<vm>-gpu.service` `ExecStartPre` (`+`-prefixed → root)
    for graphics VMs ([host-sidecars.nix](nixos-modules/host-sidecars.nix)).
    Graphics VMs bypass upstream's `microvm@<vm>.service` template,
    so they don't get `microvm-set-booted` for free; the framework's
    GPU sidecar takes ownership.

`nixling list` flags any VM where `booted != current` AND the VM
is running with `[pending restart]`; `nixling status <vm>` prints
both store paths and the exact remediation command.

#### Adding new per-VM units

Any new framework-owned per-VM service that the user might be
running through (i.e., not a build-time or activation-only
oneshot) MUST also carry `unitConfig.X-RestartIfChanged = false`
(or equivalently `restartIfChanged = false` on the NixOS-side
declaration). If your new service legitimately needs to restart
on config change (e.g., it's only a periodic timer or a one-shot
helper), document the reasoning in a comment so future readers
know why this one is different.

The convention also extends to `wantedBy` declarations: per-VM
`wantedBy = [ "multi-user.target" ]` must ALWAYS go through
`systemd.targets.multi-user.wants` symlinks, never via per-instance
`systemd.services."nixling@${name}"` declarations. NixOS
materializes per-instance declarations as SEPARATE unit files
(not drop-ins on the template) and the per-instance file lacks
the template's `ExecStart`/`ExecStop` → systemd refuses with
"no SuccessAction" at boot. See
[host-wrapper.nix](nixos-modules/host-wrapper.nix)'s
`systemd.targets.multi-user.wants = map (n: "nixling@${n}.service")` block.

## Test layout

| File                                  | Role                                                                                         |
| ------------------------------------- | -------------------------------------------------------------------------------------------- |
| `tests/static.sh`                     | **Top-level Layer-1 gate.** Parse, `flake check`, smoke evals, assertions, manifest contract, per-example flake checks. Runs from repo root; override with `ROOT=<path>`. |
| `tests/smoke-eval.nix`                | Workload smoke: minimal consumer-style nixosSystem, builder's native system.                 |
| `tests/smoke-eval-graphics.nix`       | Same shape, with `graphics.enable = true`. x86_64-only.                                      |
| `tests/smoke-eval-aarch64.nix`        | Headless smoke cross-evaluated on aarch64-linux (multi-arch eval-graph regression gate).     |
| `tests/assertions-eval.sh`            | 10 negative cases: CIDR overlap, platform gate, missing `waylandUser`, etc. Each must fail eval with the expected message. |
| `tests/net-vm-network-eval.sh`        | Net VM networkd-config invariants — most importantly the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`. |
| `tests/nixling-store.sh`              | Layer 2, optional. Per-VM store + `nixling build/switch/…` lifecycle. Requires a live host.  |
| `tests/audio.sh`                      | Layer 2, optional. Audio sidecar + host PipeWire surface. Requires a live host.              |
| `tests/lib.sh`                        | Shared shell helpers (logging, skip-detection, root-path derivation).                        |

The full layered overview, including Layer-2 integration tests, is in
[`tests/README.md`](./tests/README.md).

## CI / `flake.checks`

The root flake exposes these eval-only checks under
`flake.checks.<system>`:

| Check name             | What it evaluates                                                         |
| ---------------------- | ------------------------------------------------------------------------- |
| `eval-minimal`         | `examples/minimal/configuration.nix` against the framework module set.    |
| `eval-multi-env`       | `examples/multi-env/configuration.nix` (two isolated envs).               |
| `eval-template`        | `templates/default/configuration.nix` with sentinel fields overridden so the assertion block passes (TODO 2/3 substitutes). |
| `eval-graphics`        | `examples/graphics-workstation/configuration.nix`. **x86_64-linux only** — the framework's `checkVmPlatform` gate refuses graphics on aarch64. |

`with-entra-id` is intentionally absent from the root `flake.checks`
because it depends on the sibling `nixos-entra-id` input, which the
core flake does not (and should not) pull in. Its own flake is
still eval-checked by `tests/static.sh` during the per-example
iteration step.

## Commit conventions

- **Subject.** Short, imperative, prefixed with the touched
  area: `net: fix 10-eth-dhcp neutralization`,
  `manifest: bump manifestVersion to 2`,
  `cli: tighten exit-code table`.
- **Body.** Wrap at ~72 cols. Explain *why*, not what — the diff
  shows the what.
- **Traceability.** When the change resolves a finding from a
  reviewer wave / phase, reference it inline: `(W5fu1 H1)`,
  `(W6 H3)`, etc. These tags let reviewers cross-link commits to
  the panel that surfaced them.
- **Signing.** Sign-offs / GPG signing are not used.
- **Atomicity.** One logical change per commit. Mechanical
  reformat or rename passes go in their own commit so the
  human-reviewable diff stays small.

## Critical subsystems — handle with care

Touch these only with a clear plan and a corresponding test run.

| System                              | Where                                                                                  | Risk if broken                                                            |
| ----------------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Net VM networkd config              | `nixos-modules/net.nix` (the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`) | Net VM dual-stacks DHCP on its uplink, breaks NAT, **breaks every workload VM's egress**. Validate with `tests/net-vm-network-eval.sh`. |
| Per-VM `/nix/store` hardlink farm   | `nixos-modules/store.nix`, `/var/lib/nixling/vms/<vm>/store{,-meta}/`                   | Requires `/var/lib/nixling` and `/nix/store` on the **same filesystem** — hardlinks can't cross FS boundaries. If they end up split, `nixling switch` refuses with a fatal error. |
| TPM state                           | `/var/lib/nixling/vms/<vm>/swtpm/`                                                     | Holds the per-VM TPM 2.0 NVRAM + EK seed. **Wiping it looks like device tampering to any IdP** (Entra ID, Intune, Bitlocker-style policies) and forces re-enrollment. Never zero it casually. |
| Manifest contract                   | `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix`               | Version-pinned (`manifestVersion`). Adding, removing, or renaming a per-VM field requires bumping the version, updating the schema, and noting it in the CHANGELOG. The `static.sh` md↔json drift gate catches partial updates. |
| Eval-time assertions                | `nixos-modules/assertions.nix`                                                          | These are the framework's contract with consumers. Loosening one silently turns a previously-rejected misconfig into runtime breakage. New assertions need a matching case in `tests/assertions-eval.sh`. |
| Polkit / launcher group             | `nixos-modules/host-polkit.nix`, `host-users.nix`                                       | Wrong here = either no-one can `nixling up`, or anyone in `users` can. The threat model assumes the launcher group is the privilege boundary. |
| SSH key generation / rotation       | `nixos-modules/host-keys.nix`, `host-activation.nix`                                    | The framework owns `${cfg.site.keysDir}/<vm>_ed25519`. `nixling keys rotate` MUST NOT touch consumer-supplied keys. |

## Don'ts (security-relevant)

- **Don't remove `lib.mkForce` from the net VM's `10-eth-dhcp`
  neutralizer.** Verify any reshape of `net.nix` against
  `tests/net-vm-network-eval.sh` first.
- **Don't relax the VM-name regex or reserved prefixes.**
  `sys-*` and `launcher` are reserved so the framework can
  declare its own VMs without name collisions and so the CLI
  can route subcommands unambiguously.
- **Don't break the manifest contract silently.** Schema +
  prose + emitter move together, with a `manifestVersion`
  bump and a CHANGELOG entry.
- **Don't paper over a failing assertion by deleting it.** If
  the assertion is wrong, fix its predicate; if the predicate
  is right but the failure mode is misleading, fix the message.
- **Don't commit secrets, hostnames, real user identifiers, or
  real network ranges.** Use generic names (`alice`,
  `corp-vm`, `work`, `personal`) and RFC1918 / RFC5737 ranges
  in docs and examples. The repo has no host-identifier
  leaks today; keep it that way.
- **Don't introduce a new linter, formatter, or pre-commit
  hook unless explicitly requested.** `nix flake check`,
  `tests/static.sh`, and `shellcheck` (already wired into
  `static.sh`) are the baseline.
- **Don't add a new `nixpkgs.overlays` entry or change
  `nixpkgs.url` casually.** The overlay surface is part of
  the public ABI and overlay churn rebuilds the world for
  every consumer.

## References

- [README.md](./README.md) — consumer-facing intro, install,
  manual integration walkthrough.
- [CHANGELOG.md](./CHANGELOG.md) — Keep-a-Changelog, entries
  accumulate under `## Unreleased` until a tag cuts them.
- [SECURITY.md](./SECURITY.md) — disclosure path + scope.
- [docs/explanation/design.md](./docs/explanation/design.md) —
  threat model, defenses-in-depth list, *Why not X* FAQ.
- [docs/reference/manifest-schema.md](./docs/reference/manifest-schema.md)
  + [docs/reference/manifest-schema.json](./docs/reference/manifest-schema.json)
  — the manifest contract.
- [docs/reference/cli-contract.md](./docs/reference/cli-contract.md) —
  CLI lifecycle FSM, signal semantics, exit codes, JSON vs human output.
- [docs/how-to/migrating-from-microvm.md](./docs/how-to/migrating-from-microvm.md)
  — option mapping + step-by-step migration for users coming from
  raw microvm.nix.
- [tests/README.md](./tests/README.md) — full test layering,
  including Layer-2 integration tests.
- [LICENSE](./LICENSE) — Apache-2.0.
