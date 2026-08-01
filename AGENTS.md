# AGENTS.md

Operating manual for AI coding agents (Copilot CLI, GitHub Copilot,
Cursor, …) and human contributors working on **`vicondoa/d2b`
itself**. If you are *consuming* d2b in your own NixOS host
config, start at [README.md](./README.md) instead - this file is for
people changing the framework.

## What this is

d2b is an opinionated NixOS desktop microVM framework that
owns its microVM substrate end-to-end. The control plane is
**daemon-only**: `d2bd` supervises every per-VM DAG and
`d2b-priv-broker` dispatches every audited host mutation.
There are no per-VM systemd templates, no host-singleton framework
services, and no legacy bash CLI; see
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md) for the
binding architectural decision.

What the framework provides: per-env isolated networks with an
auto-declared NAT/DHCP "net VM", a per-VM `/nix/store` hardlink farm,
toggleable per-VM components (graphics, TPM, USBIP, audio), and the
versioned bundle/manifest contract that grounds the broker dispatcher.
See [README.md](./README.md) and
[`docs/explanation/design.md`](./docs/explanation/design.md) for the
full picture and threat model.

## Start here

This file is the index. It carries the rules; the detail lives one link away
in [`docs/contributing/`](./docs/contributing/). Find the row for what you are
about to do, read that doc, then come back.

| If you are about to... | Read first |
| --- | --- |
| Change any code | "Build and validate" below, then commit before validating |
| Touch a **critical subsystem** | The index below, then [critical-subsystems.md](./docs/contributing/critical-subsystems.md) |
| Add, move, or retire a test | [`tests/AGENTS.md`](./tests/AGENTS.md) - binding, read it before touching the test tree |
| Run a gate, a heavy lane, or a build that needs debug symbols | [gates-and-lints.md](./docs/contributing/gates-and-lints.md) - what each Layer-1 job covers, the heavy-lane semaphore, build profiles, spec-literal lints |
| Run or respond to a panel round | "Panel review" below, then [panel-review.md](./docs/contributing/panel-review.md) |
| Open a worktree, land a PR, or reclaim disk | [workflow.md](./docs/contributing/workflow.md) - worktrees, stacked PRs, edit/commit/validate, disk and cache hygiene |
| Write a changelog entry or commit message | [changelog-and-commits.md](./docs/contributing/changelog-and-commits.md) |
| Add a per-VM feature, a unit, or a broker op | [architecture.md](./docs/contributing/architecture.md) and [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md) |
| Do anything security-relevant | "Don'ts" below - that section is exhaustive and binding |
| Run an ADR, a panel round, or an autopilot wave | [copilot-agents.md](./docs/contributing/copilot-agents.md) - agents, skills, model binding, wave ids |

Two rules that override everything else:

- **Existing code is canon.** When a spec, plan, README, or reference doc
  disagrees with committed, passing code, the code wins. Document the drift;
  do not silently re-align the code to the prose. This applies to this file
  too: if you change a load-bearing behaviour described here, update it in the
  same commit.
- **Commit before you validate.** Untracked files are invisible to
  `nix flake check` and every eval that follows the same path. Forgetting to
  `git add` a new module is the most common "why didn't my change apply?".

## Repo layout

```
.
├── README.md                       <- consumer-facing entry point
├── AGENTS.md                       <- this file
├── SECURITY.md                     <- disclosure policy + threat-model summary
├── CHANGELOG.md                    <- Keep a Changelog, grouped under `## [Unreleased]`
├── LICENSE                         <- Apache-2.0
├── flake.nix                       <- public surface: nixosModules / templates / checks
├── flake.lock
├── .github/workflows/              <- CI-only checks that stay out of root `flake.checks`
├── nixos-modules/                  <- THE framework
│   ├── default.nix                 <- aggregator imported as nixosModules.default
│   ├── options.nix / options-*.nix <- option schema (site / envs / vms)
│   ├── assertions.nix              <- eval-time invariants (CIDR overlap, platform gate, …)
│   ├── lib.nix                     <- internal helpers (subnetIp, mkMac, …)
│   ├── index.nix                   <- normalized internal VM/env/component index
│   ├── host.nix / host-*.nix       <- host activation, users, polkit, sidecars, keys, audit
│   ├── network.nix / net.nix       <- per-env bridges + auto-declared net VM
│   ├── store.nix                   <- per-VM /nix/store hardlink farm
│   ├── manifest.nix                <- JSON manifest emitter (versioned contract)
│   └── components/                 <- toggleable per-VM features
│       ├── graphics.nix            <- virtio-gpu + Wayland cross-domain
│       ├── tpm.nix                 <- per-VM swtpm 2.0
│       ├── usbip.nix               <- YubiKey USBIP passthrough
│       ├── home-manager.nix        <- HM-as-NixOS-module inside the guest
│       └── audio/{guest,host}.nix  <- vhost-user-sound + PipeWire mediation
├── pkgs/                           <- patched cloud-hypervisor / crosvm / vhost-device-sound
├── packages/                       <- Rust workspace; pinned rust-toolchain.toml
│   ├── d2b-core/              <- shared bundle DTOs, typed errors, privilege metadata
│   ├── d2b-host/              <- host-side lifecycle primitives (argv, hardlink farm, ifnames)
│   ├── d2b-contracts/          <- public + private wire contracts
│   ├── d2b/                   <- rust-native CLI
│   ├── d2bd/                  <- unprivileged public daemon / supervisor
│   ├── d2b-priv-broker/       <- privileged broker for audited host mutations
│   ├── d2b-guest-shell-runner/ <- standalone static guest helper for persistent shell feasibility
│   └── xtask/                     <- schema / docs codegen helpers; see
│                                      `docs/adr/0000` + `docs/adr/0009`
├── tests/                          <- see "Test layout" below
├── examples/                       <- minimal / graphics-workstation / multi-env / with-entra-id
├── templates/default/              <- `nix flake init -t github:vicondoa/d2b`
└── docs/                           <- Diataxis tree (explanation / how-to / reference)
                                       plus `docs/adr/` architecture decision records
```

New behaviour belongs in a focused file under `nixos-modules/`
(or `nixos-modules/components/` for per-VM toggles), wired in
from `nixos-modules/default.nix`. Don't fatten existing files.

## Build and validate

Use the top-level `Makefile` targets. The shell scripts under `tests/` are
implementation details unless a target or `tests/AGENTS.md` says otherwise.

`nix develop` gives you the toolchain every gate expects. The gate scripts
bootstrap a private toolchain when it is missing, so working inside the dev
shell just skips that setup.

CI splits the Rust gate into three independent jobs, `make
test-rust-api-surface`, `make test-rust-main` and `make test-rust-remaining`,
behind the stable required `test-rust` rollup context. `make test-rust` still
runs all three partitions exactly once, so it stays the local command; reach
for a partition target only to rerun the part that failed. The API census is a
separate shard because it shares nothing with the workspace build; see
[gates and lints](./docs/contributing/gates-and-lints.md).

```bash
make check        # PR-equivalent Layer-1 gate; runs tests/layer1-jobs.json
make test-unit    # Layer-1 development umbrella (skips the preflight phase)
make test         # Layer 1 + container integration
```

Individual Layer-1 jobs, in `tests/layer1-jobs.json` local phase order:
`check-tier0`, `check-inventory`, `test-lint`, `test-changelog`, `test-rust`,
`test-proofs`, `test-flake`, `test-nix-unit`, `test-policy`, `test-drift`,
`test-runtime-ledger`, `test-performance-budgets`, `test-fixture-contracts`.

**`tests/layer1-jobs.json` is authoritative** for both the job list and its
enforcement classification. A job is enforcing unless it carries
`"enforcement": "advisory"`. Advisory means the command still runs and a
nonzero result still fails, but a guarded skip is permitted - so **an advisory
result must never be cited as validation evidence**. Re-read the manifest
rather than assuming the split is fixed; today only `test-performance-budgets`
is advisory.

Two coverage traps worth knowing before you claim a change is validated:

- **`test-rust` excludes `d2b-contract-tests`**, so a green `test-rust` does
  not validate the fixture-dependent contract and policy layer. That runs in
  `test-fixture-contracts`.
- **Doctests and `harness = false` binaries are not nextest surfaces** and get
  explicit companion runs. Several `compile_fail` doctests are capability
  seals. Do not "simplify" them away.

Before opening an agent-owned PR, run the host/manual tiers locally; the PR
pipeline does not:

```bash
make test-integration       # Layer 2 container tests; needs podman
make test-host-integration  # runNixOSTest VM checks; NixOS + KVM, x86_64 only
```

**Heavy lanes take a slot.** Every Layer-2, host-integration, hardware, live,
and perf-heavy command runs through one semaphore granting two slots per uid.
Run the public targets (`make test-integration`, `make test-host-integration`,
`make test-hardware`, `make perf`), never the internal `heavy-lane-*` targets,
which fail closed outside the gate. Details, provisioning, and the rule that
every new live/hardware/perf entrypoint must carry a self-guard block:
[gates-and-lints.md](./docs/contributing/gates-and-lints.md).

The runtime ledger, the spec-literal lint allowlist, and the D116 envelope
negative-example marker all have exemption rules that are easy to get wrong.
They are documented in the same file. The short version: the spec-literal
lints honour **no** author-suppression marker, and D116 honours exactly one,
in one pinned file, exactly once.

## Development workflow

Detail in [workflow.md](./docs/contributing/workflow.md). The binding rules:

- **`main` and `v3` are protected.** Changes land via PR, never direct push.
  `v3` is the clean-break integration lineage and never merges to `main`.
- **One logical change per commit.** Mechanical reformats or renames go in
  their own commit.
- **Use worktrees for parallel scopes**, one per agent or concurrent scope.
  When your scope is done and green, merge it back to the primary clone
  yourself; finished work on a side branch is not done, it is awaiting
  integration, and that is a state you own.
- **Concurrent slices share one worktree, so destructive git is banned.**
  Never run `git checkout --` or `git restore` on a path your slice does not
  own: uncommitted work has no reflog entry, so that is an unrecoverable
  delete of a sibling's work. Never run a package-wide formatter; format the
  single file.
- **Never `git add -A` while a build, test, or gate is running.** Those write
  scratch into the worktree. Stage the specific paths you touched.
- **Put throwaway artifacts in the gitignored `.scratch/`**, never beside
  production code or tests.
- **Test eval expressions must resolve the flake via `git+file://$ROOT`**
  (the `d2b_flake_ref` helper), never a bare path. A bare path makes Nix copy
  the entire working tree into the store, including multi-GiB cargo artifacts:
  measured at ~36 GB and 5+ minutes per cold eval, versus under a second.
- **Never clear `RUSTC_WRAPPER` to make a command work.** The repo-local
  wrapper already falls back to plain rustc when sccache is absent.
- **Run `nix-collect-garbage` after each wave merge**, and prune old system
  generations periodically; each pins 1-2 GiB.

## Panel review

Detail, including each role's focus and the harness notes, in
[panel-review.md](./docs/contributing/panel-review.md). The binding rules:

- **Multi-phase plans pass a panel gate at each phase boundary**, twice per
  phase: once on the plan before any implementation is dispatched, and once on
  the integrated diff before the next phase begins.
- **`signoff` is `true` iff `recommendations` is `[]`.** A phase closes only
  on unanimous sign-off from the full roster. **Green tests do not waive this
  gate.** The canonical precedent: a Wave-1 panel returned 0/8 sign-offs with
  11 HIGH findings that the static gate caught none of.
- **The default roster is ten roles**: `software`, `test`, `nixos`,
  `networking`, `security`, `rust`, `product`, `docs`, `observability`,
  `kernel`.
- **Reviewers do not rerun validation.** Prompts carry the evidence the
  integrator already ran, and instruct reviewers to reason over it rather than
  stampeding the shared Nix store and cargo target while implementation agents
  are still running. Missing or insufficient validation is a finding.
- **Rounds after the first are delta reviews** and carry two ranges: the delta
  since that reviewer last reviewed, and the full branch for context. Any
  content change invalidates every prior sign-off in the phase.
- **Fix rounds address only the findings raised.** A genuine defect found
  while fixing something else is still out of scope; file it separately.
  Unrequested changes are new content, new content invalidates the round's
  evidence, and the gate recedes while the deliverable sits finished.

Escape hatches are narrow: trivial fixes with no semantic change,
documentation-only changes that do not describe load-bearing behaviour, and
time-critical hotfixes, which still require a post-fix panel.

The once-per-wave binding panel is enforced in code by
`packages/xtask/src/delivery/panel.rs`: ten records, one per role, unanimous,
all bound to the same snapshot, with provider, model, and reasoning effort
pinned. There is no override and no partial pass.

## Changelog and commits

Detail in
[changelog-and-commits.md](./docs/contributing/changelog-and-commits.md). The
binding rules:

- **Every PR that changes code ships release notes**, either as a
  `CHANGELOG.md` entry under `## [Unreleased]` or as a fragment under
  `changelog.d/`. **Use a fragment when more than one branch is in flight** -
  two branches appending to the same block is a guaranteed conflict.
- **Follow [Keep a Changelog](https://keepachangelog.com/) and semver.** The
  version in `CHANGELOG.md` is the single source of truth. Merging to `v3`
  with a new version header triggers the tag, binary build, and release.
- **Commit subjects are short, imperative, and area-prefixed**
  (`net: fix 10-eth-dhcp neutralization`). Explain *why* in the body, wrapped
  at ~72 columns; the diff shows the what.
- **Commits on feature branches carry a trailing wave tag**, `( W3 )`,
  `( W2fu1 H3 )`, or the qualified form `( spec001w1 )`. Every commit from a
  panel-fix round must carry the relevant tag.
- **No AI, tool, or model attribution** in commit subjects, bodies, PR
  descriptions, changelog entries, or shipped docs. No `Co-authored-by`
  trailer for AI tools unless explicitly requested.
- **Sign-offs and GPG signing are not used.**

**Process markers stay out of shipped artifacts.** Wave, phase, revision,
follow-up, round, and finding tags (`W3`, `W4-fu`, `P6`, `D5/P2.3`,
`( W1fu3 H20 )`) organise work; they are not shipped. Keep them out of source
comments, shipped docs prose, user-facing CLI and error text, CI job and step
names, and **every** CHANGELOG section including `[Unreleased]`. They remain
welcome in planning artifacts, this file and the other process docs, ADRs, and
feature-branch commit messages. The ban is enforced by `scan_process_markers`
in `tests/tools/tier0-first-pass.sh` via `make check-tier0`, against a frozen
allowlist; that script is authoritative for governed paths and exceptions.
There are two deliberate functional exceptions: the consumer-facing
`d2b.defaultSwitchReadiness.<wave>` option surface, and the delivery tool's
closed `W0`-`W8` namespace under `packages/xtask/src/delivery/`.

## Test layout

The test tree has a binding local operating manual:
[`tests/AGENTS.md`](./tests/AGENTS.md). Read it before adding,
moving, or retiring test coverage. It defines the closed Layer-1 set,
the Layer-2 exceptions, the exact file locations, and the pin/ledger
updates required for each change.

At a glance:

| Location | Role |
| --- | --- |
| `tests/test-*.sh`, `tests/static.sh`, `tests/runner.sh` | Make-target drivers and orchestrators; do not add a new top-level shell gate unless `tests/AGENTS.md` explicitly permits it. |
| `tests/unit/nix/cases/` | Auto-discovered nix-unit eval cases. After adding/removing one, run `make nix-unit-pin`. |
| `tests/unit/nix/eval-cases/`, `tests/unit/smoke/` | Flake-check and smoke-eval definitions. After adding/removing a flake check, run `make flake-matrix-pin`. |
| `packages/<crate>/src/**`, `packages/<crate>/tests/*.rs` | Rust unit and binary integration tests. Prefer these over shell gates when behaviour is hermetic. |
| `packages/d2b-contract-tests/tests/` | Rendered-artifact contract tests and policy lints. The fixture-dependent crate is excluded from `test-rust`; its fixture-backed tests run in the enforcing `test-fixture-contracts` lane, while selected hermetic policy files have separate enforcing entrypoints. |
| `tests/unit/gates/`, `tests/unit/meta/` | Drift and meta gates; closed set. Regenerate affected artifacts with the matching `xtask gen-*` command instead of adding another gate. |
| `tests/integration/containers/` | Container integration tests run by `make test-integration`; host/manual pre-PR tier. |
| `tests/host-integration/*.nix` | runNixOSTest VM checks run by `make test-host-integration`; local NixOS/KVM pre-PR tier, not the PR pipeline. |
| `tests/integration/live/`, `tests/host-integration/hardware/` | Live-host and hardware tests. Manual only; require deployed state or real devices. |

## CI / `flake.checks`

The root flake exposes these eval-only checks under
`flake.checks.<system>`:

| Check name             | What it evaluates                                                         |
| ---------------------- | ------------------------------------------------------------------------- |
| `eval-minimal`         | `examples/minimal/configuration.nix` against the framework module set.    |
| `eval-multi-env`       | `examples/multi-env/configuration.nix` (two isolated envs).               |
| `eval-template`        | `templates/default/configuration.nix` with sentinel fields overridden so the assertion block passes (TODO 2/3 substitutes). |
| `eval-graphics`        | `examples/graphics-workstation/configuration.nix`. **x86_64-linux only** - the framework's `checkVmPlatform` gate refuses graphics on aarch64. |

`with-entra-id` is intentionally absent from the root `flake.checks`
because it depends on the sibling `entrablau` input, which the
core flake does not (and should not) pull in. Its own flake is
still eval-checked by `tests/static.sh` during the per-example
iteration step, and CI also runs
`.github/workflows/eval-with-entra-id.yml` to execute
`nix flake check --no-build --all-systems --no-write-lock-file`
inside the example directory without coupling the root flake to the
sibling input.

## Critical subsystems - handle with care

Touch these only with a clear plan and a corresponding test run. Each row
links to its full invariants in
[critical-subsystems.md](./docs/contributing/critical-subsystems.md); **read
that section before changing the subsystem**, because the one-line risk here
is a warning, not the contract.

| System | Where | Risk if broken |
| --- | --- | --- |
| [Net VM networking / firewall](docs/contributing/critical-subsystems.md#net-vm-networking-firewall) | `nixos-modules/net.nix` (the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus the per-env MTU/MSS and east-west wiring) | Net VM dual-stacks DHCP on its uplink, breaks NAT, or weakens same-env isolation unexpectedly. Validate with `tests/unit/nix/cases/net-vm-network.nix`. |
| [Per-VM `/nix/store` hardlink farm](docs/contributing/critical-subsystems.md#per-vm-nixstore-hardlink-farm) | `nixos-modules/store.nix` | The guest's `/nix/store` MUST be the per-VM closure-only farm `/var/lib/d2b/vms/<vm>/store`, never the host's full `/nix/store`. Serving the host store re-leaks it to every guest. Needs `/var/lib/d2b` and `/nix/store` on the same filesystem. |
| [TPM persistence (per-VM swtpm)](docs/contributing/critical-subsystems.md#tpm-persistence-per-vm-swtpm) | `/var/lib/d2b/vms/<vm>/swtpm/` | Holds the per-VM TPM 2.0 NVRAM + EK seed. |
| [USBIP passthrough](docs/contributing/critical-subsystems.md#usbip-passthrough) | `nixos-modules/components/usbip.nix` (eval-time gating) + broker `UsbipBindFirewallRule` + `SpawnRunner` (per-busid attach process supervised by `d2bd`) | Eval-time gating still scopes attach to opted-in envs (validated by `tests/unit/nix/cases/usbip-gating.nix`). |
| [GPU sidecar (graphics VMs)](docs/contributing/critical-subsystems.md#gpu-sidecar-graphics-vms) | `nixos-modules/components/graphics.nix` + broker `SpawnRunner` for cloud-hypervisor on graphics VMs | Graphics VMs run cloud-hypervisor with the GPU device attached. |
| [Video sidecar (graphics VMs)](docs/contributing/critical-subsystems.md#video-sidecar-graphics-vms) | `nixos-modules/components/video/guest.nix` | `graphics.videoSidecar = true` is an explicit opt-in H264 decode path: guest `virtio_media` + patched Cloud Hypervisor and crosvm. Must use the `d2b-<vm>-video` principal, never `d2b-<vm>-gpu`. |
| [UI color contract / niri backend](docs/contributing/critical-subsystems.md#ui-color-contract-niri-backend) | `nixos-modules/ui-colors.nix` | The compositor-agnostic `d2b.site.ui` / `d2b.envs.<env>.ui` / `d2b.vms.<vm>.ui` color model is the source of truth for host/env/VM/state colors. |
| [ComponentSession capability boundary](docs/contributing/critical-subsystems.md#componentsession-capability-boundary) | `packages/d2b-contracts/src/v3/component_session.rs` | Authenticated transport evidence and attachment credits are consumed into a private single session owner; do not add a clone/accessor that lets callers reuse admission evidence. `SessionAuthority` is sealed and must stay sealed. |
| [Zone message bus boundary](docs/contributing/critical-subsystems.md#zone-message-bus-boundary) | `packages/d2b-bus/src/{router,registry,authorization,streams,operations}.rs` | Registration consumes the single-owner capability admission; comparing a clonable token is insufficient. |
| [Authoritative subject resolution](docs/contributing/critical-subsystems.md#authoritative-subject-resolution) | `packages/d2b-bus/src/router.rs` (`ZoneRegistrar`) | `ZoneRegistrar` **exclusively owns and consumes** subject resolution: a peer is mapped to a subject from registrar-private state using verified peer evidence. Never accept a caller-supplied subject. |
| [Capability mint surface allowlist](docs/contributing/critical-subsystems.md#capability-mint-surface-allowlist) | `packages/d2b-api-surface/`, `tests/golden/api-surface/`, `packages/d2b-bus/tests/public_mint_surface.rs` | The **enforcing compiler leg** uses stable trait-solver ambiguity assertions in the defining crates. |
| [Resource controller effects boundary](docs/contributing/critical-subsystems.md#resource-controller-effects-boundary) | `packages/d2b-controller-toolkit/src/` + `packages/d2b-core-controller/src/` | Controller and core-reconciliation engines are test-only and unwired from the absent production store/watch dispatcher. |
| [Unsafe-local provider, launcher, and persistent-shell helper](docs/contributing/critical-subsystems.md#unsafe-local-provider-launcher-and-persistent-shell-helper) | `nixos-modules/options-realms-workloads.nix` | `unsafe-local` is explicit and default-denied. |
| [Manifest contract](docs/contributing/critical-subsystems.md#manifest-contract) | `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix` | Version-pinned via `manifestVersion`. |
| [Manifest bundle - private artifacts](docs/contributing/critical-subsystems.md#manifest-bundle---private-artifacts) | `docs/reference/manifest-bundle.md` + `docs/reference/schemas/v2/*.json` + `packages/d2b-core/src/` bundle DTOs + `nixos-modules/bundle*.nix` | Sensitive bundle artifacts install at `root:d2bd` 0640 and ground every broker/sandbox/runner behaviour. |
| [Control plane - `d2bd` + `d2b-priv-broker`](docs/contributing/critical-subsystems.md#control-plane---d2bd-d2b-priv-broker) | `packages/d2b-contracts/**` + `packages/d2b-core/**` + `packages/d2bd/**` + `packages/d2b-priv-broker/**` (sibling workspace) | The **only** persistent root surfaces the framework declares. |
| [Storage lifecycle / restart / synchronization](docs/contributing/critical-subsystems.md#storage-lifecycle-restart-synchronization) | Planned generated contracts in `d2b-core::{storage,process_restart,sync}` + broker storage/sync ops | Managed paths, restart adoption, locks, leases, cleanup, and degraded-state reporting are control-plane contracts. |
| [Eval-time assertions](docs/contributing/critical-subsystems.md#eval-time-assertions) | `nixos-modules/assertions.nix` | These are the framework's contract with consumers. |
| [Guest-control exec session table](docs/contributing/critical-subsystems.md#guest-control-exec-session-table) | `packages/d2bd/src/{exec_session,exec_session_real}.rs` | Arbitrary `d2b vm exec` is **admin-only**; configured `d2b launch` local-VM items may use the same backend with launcher authority. guestd runs every exec as the workload user, never root. |
| [Unsafe-local persistent shells](docs/contributing/critical-subsystems.md#unsafe-local-persistent-shells) | `packages/d2bd/src/` shell dispatch + `packages/d2b-unsafe-local-helper/src/` | `d2b shell` remains **admin-only** for every provider. |
| [Lifecycle permission group](docs/contributing/critical-subsystems.md#lifecycle-permission-group) | `nixos-modules/host-users.nix` | Membership in `d2b` + `SO_PEERCRED` at `public.sock` accept time is the **only** lifecycle authorisation surface. |
| [SSH key generation / rotation](docs/contributing/critical-subsystems.md#ssh-key-generation-rotation) | `nixos-modules/host-keys.nix` | The framework owns `${cfg.site.keysDir}/<vm>_ed25519`. |
| [virtiofsd sandbox model](docs/contributing/critical-subsystems.md#virtiofsd-sandbox-model) | `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles) | virtiofsd profiles MUST declare zero host capabilities (`capabilities = []`), `requiresStartRoot = false`, and a `userNamespace` block mapping in-namespace root to the per-share principal (ADR 0021). |

## Don'ts (security-relevant)

- **Don't remove `lib.mkForce` from the net VM's `10-eth-dhcp`
  neutralizer.** Verify any reshape of `net.nix` against
  `tests/unit/nix/cases/net-vm-network.nix` first.
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
- **Don't reintroduce a per-VM systemd unit or a host-singleton
  framework service.** Every per-VM lifecycle step lives inside
  `d2bd`'s DAG executor with privileged side effects routed
  through a typed `d2b-priv-broker` op (ADR 0015). Policy coverage
  lives in `packages/d2b-contract-tests/tests/policy_units.rs` and
  `policy_docs.rs`; run the enabled fixture-contract lane because
  those checks are not part of `test-rust`.
- **Don't reintroduce a bash CLI fallback or env-knob escape
  hatch.** The Rust CLI is the only operator surface;
  `D2B_LEGACY_BASH_OPT_IN`, `D2B_LEGACY_CLI`, and
  `D2B_NATIVE_ONLY` are no-ops.
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
- **Don't leak internal process markers into shipped artifacts.**
  Wave/phase/revision/follow-up/finding tags (`W3`, `W4-fu`, `P6`,
  `D5/P2.3`, `( W1fu3 H20 )`) belong in planning artifacts,
  pre-release `[Unreleased]`, ADRs, this file's process sections,
  and feature-branch commits - never in shipped source comments,
  shipped docs prose, CLI help/error text, or any CHANGELOG section.
  See [Changelog and commits](#changelog-and-commits).
  The functional `d2b.defaultSwitchReadiness.<wave>` option
  surface is the one deliberate exception.
- **Don't spell a dash with anything but the ASCII hyphen `-`.** Not in
  source, comments, string literals, CLI help or error text, documentation
  prose, ADRs, specs, changelog entries, commit messages, or PR bodies.
  The banned class is every non-ASCII dash codepoint: U+2010 hyphen,
  U+2011 non-breaking hyphen, U+2012 figure dash, U+2013 en dash,
  U+2014 em dash, U+2015 horizontal bar, U+2212 minus sign, U+FE58 small
  em dash, and U+FF0D fullwidth hyphen. Where one of those would have
  separated clauses, use a spaced hyphen ` - ` or restructure the
  sentence; where it joined a range or a compound, close it up to `-`.
  This rule names codepoints rather than printing characters, because the
  gate below would flag this very line. `make check-tier0` scans every
  tracked and every non-ignored untracked file and fails closed with the
  offending `file:line` list, so a reintroduced character breaks the build
  rather than surviving review. When a test genuinely needs one of them
  (a parser tolerance case, the gate's own patterns) spell it as an escape
  such as `"\u{2014}"` or `$'\u2014'`, never as the character.
  One hazard is worth knowing before you paste text in: the ADR-046
  work-item tokenizer treats a typographic dash as a token separator but a
  plain hyphen as an id character, so an id range that was spelled with a
  typographic dash fuses into a single grammatically valid but nonexistent
  id when normalized. `spec-registry` fails closed on the dangling
  dependency rather than corrupting the graph. Respell such a range as an
  enumeration instead of defeating the check; see the `Dependency/owner`
  cell for `ADR046-network-005` in
  `docs/specs/ADR-046-resources-network.md` for the shape that survives
  normalization.
- **Don't let a host process hold realm credentials, or treat relay
  identity as local auth (ADR 0032).** Realm relay/session/provider
  credentials, remote node registries, and realm audit belong inside
  a per-realm gateway guest VM - never in `d2bd`, the broker, the
  host bundle, host-readable storage, or any host-side activation
  artifact. A relay-authenticated peer is never mapped to local
  `Admin`; `SO_PEERCRED` + `d2b` group membership stays the only
  local lifecycle authz surface. Work and personal realms never share
  a gateway guest or an L2 bridge.
- **Don't add ad-hoc storage, ACL, cleanup, or lock ownership paths.**
  Storage and synchronization changes must fit the ADR 0034 contract:
  broker-resolved opaque ids, anchored path resolution, OFD locks with
  `O_CLOEXEC`, explicit fd transfer only, restart-aware adoption before
  cleanup, and typed degraded-state reporting instead of broad chmod,
  chown, setfacl, or `/run/d2b` sweeps. Every new host-mutable
  path or lock surface must add or reuse a generated `storage.json` /
  `sync.json` row, name a single repair owner, and route repair through
  that owner rather than adding a second activation/broker/daemon fixer.
- **Don't write a host mutation outside its ownership marker, and don't
  proceed past a foreign one.** Every d2b host mutation is delimited so
  foreign configuration can be preserved byte for byte: nftables rules and
  chains in the `inet d2b` table carry
  `comment "d2b managed: <ownership-id>"` and foreign tables are never
  flushed; `/etc/hosts` and `/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf`
  are delimited by `# d2b-managed begin` / `# d2b-managed end`;
  systemd-networkd is detection-only and d2b never writes it. Finding a
  foreign marker where d2b expects its own is **fail-closed**
  (`path-safety-violation`, `nm-managed-foreign-conflict`,
  `foreign-nft-rule-preserved`), never a signal to overwrite. Full
  conventions in
  [critical-subsystems.md](./docs/contributing/critical-subsystems.md#cgroup-slice-naming-and-ownership-markers).
- **Don't mutate a d2b cgroup outside the delegation contract.** One
  canonical slice, `/sys/fs/cgroup/d2b.slice`, with per-VM directories at
  `d2b.slice/<vm>/<role>/` and a process-free VM layer. Never write
  `cpuset.cpus.partition` on a d2b-owned cgroup, never use threaded
  cgroups, never `cgroup.kill` the slice or any ancestor of a daemon-owned
  leaf, and never mutate the delegated subtree as uid 0 after privilege
  drop. The host cgroup root is never chowned.
- **Don't commit an unredacted screenshot or visual artifact.** Before a
  screenshot is committed or attached to a PR or panel prompt, remove every
  secret, credential, API key, and token; remove PII (real names, emails,
  employee or user ids); and remove sensitive output such as host paths,
  internal node names, and realm principals. Use the generic placeholder
  names this file already requires. If it cannot be redacted without losing
  what it demonstrates, describe it in text instead.

## Daemon-only end-state (P6 onward)

The framework declares **exactly three** root-visible units:
`d2bd.service`, `d2b-priv-broker.socket`, and
`d2b-priv-broker.service`. The binding architectural decision
is recorded in
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md).

Agents working on the framework MUST treat the following as the
contract:

- The CLI is the Rust `d2b` binary, full stop. There is no bash
  fallback bridge; `D2B_LEGACY_BASH_OPT_IN`, `D2B_LEGACY_CLI`,
  and `D2B_NATIVE_ONLY` are no-ops.
- There are no framework-declared per-VM systemd units. The per-VM
  lifecycle DAG runs inside `d2bd`; spawned runners
  (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound, USBIP
  attach) are launched by the broker's `SpawnRunner` op and handed
  back to `d2bd` as pidfds via `OpenPidfd` / `SCM_RIGHTS`.
- There are no host-singleton framework services
  (`d2b-ch-exporter`, `d2b-otel-host-bridge`,
  `d2b-net-route-preflight`, `d2b-audit-check[.timer]`,
  `microvms.target`). Their work either moved into `d2bd` or
  was retired with the metric / signal it produced.
- The `d2b.vms.<vm>.supervisor` option has been removed; setting
  it fails eval with a typed friendly message.
- The polkit allowlist for legacy launcher groups is retired.
  `d2b` group membership + `SO_PEERCRED` at
  `public.sock` accept time is the **only** lifecycle authorisation
  surface.
- The Rust CLI does not invoke bash. `tests/tools/no-bash-ast-walker`
  is the enforcing AST-level check in `test-rust`; the companion
  source policy in `packages/d2b-contract-tests/tests/policy_source.rs`
  runs in the enforcing fixture-contract lane
  ([ADR 0017](./docs/adr/0017-no-bash-fallbacks-invariant.md)).

### Verification gates

- `packages/d2b-contract-tests/tests/policy_units.rs` denies retired
  unit names, while `policy_lints.rs` checks the ADR header and
  cross-references and `policy_docs.rs` checks this file's daemon-only
  wording. These fixture-dependent policies are not enforcing
  pull-request evidence until `test-fixture-contracts` is enabled and
  promoted.
- Host exit criterion: on a deployed host,
  `systemctl list-units --no-pager --all | grep -E '^(d2b|microvm)' | wc -l`
  returns `3`.

## References

Process and contributor docs:

- [`docs/contributing/`](./docs/contributing/) - workflow, panel review,
  changelog and commits, gates and lints, critical subsystems, architecture
  conventions.
- [`tests/AGENTS.md`](./tests/AGENTS.md) - binding operating manual for the
  test tree. [`tests/README.md`](./tests/README.md) is the human quick-start.

Binding architectural decisions:

- [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md) - the daemon-only
  end-state: `d2bd` + `d2b-priv-broker` are the only persistent root surfaces.
- [ADR 0017](./docs/adr/0017-no-bash-fallbacks-invariant.md) - the Rust CLI
  never invokes bash.
- [ADR 0018](./docs/adr/0018-microvm-nix-removal.md) - d2b owns its per-VM
  substrate; the `microvm.nix` input is gone.
- [ADR 0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md) - broker
  pre-establishes a user namespace so virtiofsd holds zero host capabilities.
- [ADR 0031](./docs/adr/0031-bare-command-and-detached-exec.md) - bare command
  resolution and detached workload-user exec.
- [ADR 0032](./docs/adr/0032-d2b-v2-constellation-control-plane.md) - the host
  holds no realm credentials, and relay identity is never local auth.
- [ADR 0034](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md) -
  daemon restarts are continuation events; adopt before cleanup.

Design and contracts:

- [README.md](./README.md) - consumer-facing intro and install.
- [SECURITY.md](./SECURITY.md) - disclosure path and scope.
- [CHANGELOG.md](./CHANGELOG.md) - Keep a Changelog.
- [design.md](./docs/explanation/design.md) - threat model and defenses.
- [daemon-lifecycle.md](./docs/explanation/daemon-lifecycle.md) - DAG
  executor, pidfd handoff, supervisor reconciliation.
- [privileges.md](./docs/reference/privileges.md) - broker op catalogue.
- [daemon-api.md](./docs/reference/daemon-api.md) - `public.sock` wire
  surface, audit format, retention.
- [manifest-schema.md](./docs/reference/manifest-schema.md) - the manifest
  contract.
- [cli-contract.md](./docs/reference/cli-contract.md) - lifecycle FSM, signal
  semantics, exit codes.
- [naming-conventions.md](./docs/reference/naming-conventions.md) - canonical
  glossary of internal identifiers.
- [LICENSE](./LICENSE) - Apache-2.0.
