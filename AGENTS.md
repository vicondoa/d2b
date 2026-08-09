# AGENTS.md

Operating manual for AI coding agents (Copilot CLI, GitHub Copilot,
Cursor, …) and human contributors to **`vicondoa/d2b` itself**. If you
*consume* d2b in your NixOS host config, start at [README.md](./README.md).

## What this is

d2b is an opinionated NixOS desktop microVM framework that owns its microVM
substrate end-to-end. Its control plane is **daemon-only**: `d2bd` supervises
every per-VM DAG and `d2b-priv-broker` dispatches every audited host mutation.
No per-VM systemd templates, no host-singleton framework
services, and no legacy bash CLI; see
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md) for binding
decision.

Framework provides per-env isolated networks with auto-declared NAT/DHCP "net
VM", a per-VM `/nix/store` hardlink farm, toggleable per-VM components
(graphics, TPM, USBIP, audio), and the versioned bundle/manifest contract
grounding the broker dispatcher.
See [README.md](./README.md) and
[`docs/explanation/design.md`](./docs/explanation/design.md) for the
picture and threat model.

## Start here

Index: rules; detail: [`docs/contributing/`](./docs/contributing/). Find the
row, then read it.

| If you are about to... | Read first |
| --- | --- |
| Change any code | "Build and validate" below, then commit before validating |
| Touch **critical subsystem** | index below, then [critical-subsystems.md](./docs/contributing/critical-subsystems.md) |
| Add, move, or retire test | [`tests/AGENTS.md`](./tests/AGENTS.md) - binding, read it before touching test tree |
| Run gate, heavy lane, or build that needs debug symbols | [gates-and-lints.md](./docs/contributing/gates-and-lints.md) - what each Layer-1 job covers, heavy-lane semaphore, build profiles, spec-literal lints |
| Run or respond to panel round | "Panel review" below, then [panel-review.md](./docs/contributing/panel-review.md) |
| Open worktree, land PR, or reclaim disk | [workflow.md](./docs/contributing/workflow.md) - worktrees, stacked PRs, edit/commit/validate, disk and cache hygiene |
| Write changelog entry or commit message | [changelog-and-commits.md](./docs/contributing/changelog-and-commits.md) |
| Add per-VM feature, unit, or broker op | [architecture.md](./docs/contributing/architecture.md) and [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md) |
| Do anything security-relevant | "Don'ts" below - that section is exhaustive and binding |
| Run ADR, panel round, or autopilot wave | [copilot-agents.md](./docs/contributing/copilot-agents.md) - agents, skills, model binding, wave ids |
| Change feature artifact after its first write | `d2b-spec-edit` owns batch; read [copilot-agents.md](./docs/contributing/copilot-agents.md) |

Two rules override everything else:

- **Existing code is canon.** When spec, plan, README, or reference doc
  disagrees with committed, passing code, code wins. Document drift; do not
  silently re-align prose. If load-bearing behavior here changes, update this
  file in the same commit.
- **Commit before you validate.** Untracked files are invisible to
  `nix flake check` and evals using the same path. Forgetting to `git add` a
  new module is the common "why didn't my change apply?" failure.

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
├── packages/                       <- unified product Rust workspace
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

New behaviour belongs in focused file under `nixos-modules/`
(or `nixos-modules/components/` for per-VM toggles), wired in
from `nixos-modules/default.nix`. Don't fatten existing files.

## Build and validate

Use top-level `Makefile` targets. Shell scripts under `tests/` are
implementation details unless a target or `tests/AGENTS.md` says otherwise.

`nix develop` provides the toolchain every gate expects. Gate scripts bootstrap
a private toolchain when missing, so a dev shell skips that setup.

The unified product workspace is rooted at `packages/Cargo.toml` and
`packages/Cargo.lock`; the separate walker remains under
`tests/tools/no-bash-ast-walker`. The Cargo/Bazel workspace,
policy-context manifest, regeneration sequence, and native-check inventory are documented in
[Bazel and policy workflows](./docs/contributing/bazel-and-policy.md).
`make test-rust` is still the aggregate over the eight Rust leaf jobs. See
[gates and lints](./docs/contributing/gates-and-lints.md) for coverage.

```bash
make check        # PR-equivalent Layer-1 gate; runs tests/layer1-jobs.json
make test-unit    # Layer-1 development umbrella (skips the preflight phase)
make test         # Layer 1 + container integration
```

Individual Layer-1 jobs, in `tests/layer1-jobs.json` order:
`check-tier0`, `check-inventory`, `test-lint`, `test-changelog`, `test-rust`,
`test-proofs`, `test-flake`, `test-nix-unit`, `test-policy`, `test-drift`,
`test-runtime-ledger`, `test-performance-budgets`, `test-fixture-contracts`.

**`tests/layer1-jobs.json` is authoritative** for the job list and enforcement
classification. A job is enforcing unless it carries
`"enforcement": "advisory"`. Advisory means command still runs and a
nonzero result still fails, but guarded skip is permitted - so **advisory
result must never be cited as validation evidence**. Re-read manifest
rather than assuming split is fixed; today only `test-performance-budgets`
is advisory.

Two coverage traps matter before claiming validation:

- **Layer-1 Rust orchestration excludes `d2b-contract-tests` from its Rust
  shards** by setting `D2B_SKIP_FIXTURE_BUILD=1`, then runs enforcing
  `test-fixture-contracts` separately. Local `make test-rust` includes fixture
  and CLI contract surfaces when Nix is available. Cite
  `test-fixture-contracts`, not the Rust shard, for Layer-1
  fixture-dependent coverage.
- **Doctests and `harness = false` binaries are not nextest surfaces** and need
  explicit companion runs. Several `compile_fail` doctests are capability
  seals; do not "simplify" them away.

Before opening an agent-owned PR, run host/manual tiers locally; the PR
pipeline does not:

```bash
make test-integration       # Layer 2 container tests; needs podman
make test-host-integration  # runNixOSTest VM checks; NixOS + KVM, x86_64 only
```

**Heavy lanes take slot.** Every Layer-2, host-integration, hardware, live, or
perf-heavy command uses one semaphore granting two slots per uid. Run public
targets (`make test-integration`, `make test-host-integration`,
`make test-hardware`, `make perf`), never internal `heavy-lane-*` targets,
which fail closed outside the gate. Details, provisioning, and the rule that
every new live/hardware/perf entrypoint carries a self-guard block:
[gates-and-lints.md](./docs/contributing/gates-and-lints.md).

Runtime ledger, spec-literal lint allowlist, and D116 envelope negative-example
marker have exemptions that are easy to misread. They are documented in that file.
Short version: spec-literal
lints honour **no** author-suppression marker, and D116 honours exactly one,
in one pinned file, exactly once.

Prompt policy is checked locally. Caveman provenance is under
`third_party/caveman/v1.10.0/`, pinned to tag `v1.10.0` at commit
`fcf7663366c217dc8f334a11028de52ed950ceab`; `UPSTREAM.json` carries three
SHA-256 values. Selected delivery and review lanes may use optional full transient
communication. Explicit `normal` or `off` wins. Mode never changes persisted
prose, panel verdict JSON, finding bars, or panel requirements, except the
compressed prompt corpus checked by
`scripts/copilot/prompt-corpus.mjs`.

## Development workflow

Detail in [workflow.md](./docs/contributing/workflow.md). Binding rules:

- **`main` and `v3` are protected.** Changes land via PR, never direct push.
  `v3` is clean-break integration lineage and never merges to `main`.
- **One logical change per commit.** Mechanical reformats or renames go in
  their own commit.
- **Use worktrees for parallel scopes**, one per agent or concurrent scope.
  When done and green, merge the branch back to the primary clone yourself;
  finished side branch work still awaits integration.
- **Concurrent slices share one worktree, so destructive git is banned.**
  Never run `git checkout --` or `git restore` on an unowned path:
  uncommitted work has no reflog, so this unrecoverably deletes sibling work.
  Never run a package-wide formatter; format one file.
- **Never `git add -A` while build, test, or gate is running.** Those write
  scratch into the worktree. Stage specific paths.
- **Put throwaway artifacts in gitignored `.scratch/`**, never beside
  production code or tests.
- **Route existing feature-artifact writes through `d2b-spec-edit`.** A
  `speckit-*` command may create only designated absent artifacts: `specify`
  creates feature directory, initial `spec.md`, and first requirements
  checklist; `plan` creates absent plan, research, data-model, contracts, or
  quickstart artifacts; `tasks` creates absent `tasks.md`; `checklist` creates
  absent checklist. `clarify` batches answers, `analyze` stays read-only,
  `implement` reports checkbox changes, `converge` prepares exact append
  content, and
  `autopilot` and memory fold route feature-directory writes through editor.
  Once file exists, editor owns every later write and refuses root escape.
- **Test eval expressions must resolve flake via `git+file://$ROOT`**
  (`d2b_flake_ref` helper), never bare path. bare path makes Nix copy
  entire working tree into store, including multi-GiB cargo artifacts:
  measured at ~36 GB and 5+ minutes per cold eval, versus under second.
- **Never clear `RUSTC_WRAPPER` to make command work.** repo-local
  wrapper already falls back to plain rustc when sccache is absent.
- **Run `nix-collect-garbage` after each wave merge**, and prune old system
  generations periodically; each pins 1-2 GiB.

## Panel review

Detail, including each role's focus and harness notes, in
[panel-review.md](./docs/contributing/panel-review.md). binding rules:

- Multi-phase work passes a plan gate before implementation and a work gate
  before advance. `signoff` is `true` iff `recommendations` is `[]`; every
  selected lifecycle seat must sign off, and green tests never waive the gate.
- Selection uses the versioned thirteen-seat table. It includes every
  mandatory and triggered seat, meets the applicable floor, and only widens.
  Rust depth is a `software` profile; legacy records retain `rust`.
- One comprehensive discovery produces the stable shared ledger. Fixes are
  ledger-scoped and batched. Verification receives the full ledger, responses,
  evidence, fix delta, and full candidate; it checks resolutions and
  regressions without reopening discovery.
- Reviewers are read-only and do not rerun validation unless explicitly asked.
  Missing evidence is a finding. Unrelated defects found during a fix are
  recorded separately rather than expanding that lifecycle.

Escape hatches are narrow: trivial fixes with no semantic change,
documentation-only changes that do not describe load-bearing behaviour, and
time-critical hotfixes, which still require post-fix panel.

The once-per-wave binding panel is enforced in code by
`packages/xtask/src/delivery/panel.rs`: the request stores the selected roster,
attestation requires one unanimous candidate-bound record per stored role, and
strict fixed-ten legacy artifacts remain readable. No override or partial pass.

## Changelog and commits

Detail in
[changelog-and-commits.md](./docs/contributing/changelog-and-commits.md). The
binding rules:

- **Every PR that changes code ships release notes**, either as a
  `CHANGELOG.md` entry under `## [Unreleased]` or as fragment under
  `changelog.d/`. **Use fragment when more than one branch is in flight** -
  two branches appending to same block is guaranteed conflict.
- **Follow [Keep a Changelog](https://keepachangelog.com/) and semver.** The
  version in `CHANGELOG.md` is single source of truth. Merging to `v3`
  with new version header triggers tag, binary build, and release.
- **Commit subjects are short, imperative, and area-prefixed**
  (`net: fix 10-eth-dhcp neutralization`). Explain *why* in body, wrapped
  at ~72 columns; diff shows what.
- **Commits on feature branches carry trailing wave tag**, `( W3 )`,
  `( W2fu1 H3 )`, or qualified form `( spec001w1 )`. Every commit from a
  panel-fix round must carry relevant tag.
- **No AI, tool, or model attribution** in commit subjects, bodies, PR
  descriptions, changelog entries, or shipped docs. No `Co-authored-by`
  trailer for AI tools unless explicitly requested.
- **Sign-offs and GPG signing are not used.**

**Process markers stay out of shipped artifacts.** Wave, phase, revision,
follow-up, round, and finding tags (`W3`, `W4-fu`, `P6`, `D5/P2.3`,
`( W1fu3 H20 )`) organise work; they are not shipped. Keep them out of source
comments, shipped docs prose, user-facing CLI and error text, CI job and step
names, and **every** CHANGELOG section including `[Unreleased]`. They remain
welcome in planning artifacts, this file and other process docs, ADRs, and
feature-branch commit messages. ban is enforced by `scan_process_markers`
in `tests/tools/tier0-first-pass.sh` via `make check-tier0`, against frozen
allowlist; that script is authoritative for governed paths and exceptions.
two deliberate functional exceptions: consumer-facing
`d2b.defaultSwitchReadiness.<wave>` option surface, and delivery tool's
closed `W0`-`W8` namespace under `packages/xtask/src/delivery/`.

## Test layout

test tree has binding local operating manual:
[`tests/AGENTS.md`](./tests/AGENTS.md). Read it before adding,
moving, or retiring test coverage. It defines closed Layer-1 set,
Layer-2 exceptions, exact file locations, and pin/ledger
updates required for each change.

At glance:

| Location | Role |
| --- | --- |
| `tests/test-*.sh`, `tests/static.sh`, `tests/runner.sh` | Make-target drivers and orchestrators; do not add new top-level shell gate unless `tests/AGENTS.md` explicitly permits it. |
| `tests/unit/nix/cases/` | Auto-discovered nix-unit eval cases. After adding/removing one, run `make nix-unit-pin`. |
| `tests/unit/nix/eval-cases/`, `tests/unit/smoke/` | Flake-check and smoke-eval definitions. After adding/removing flake check, run `make flake-matrix-pin`. |
| `packages/<crate>/src/**`, `packages/<crate>/tests/*.rs` | Rust unit and binary integration tests. Prefer these over shell gates when behaviour is hermetic. |
| `packages/d2b-contract-tests/tests/` | Rendered-artifact contract tests and policy lints. fixture-dependent crate is excluded from `test-rust`; its fixture-backed tests run in enforcing `test-fixture-contracts` lane, while selected hermetic policy files have separate enforcing entrypoints. |
| `tests/unit/gates/`, `tests/unit/meta/` | Drift and meta gates; closed set. Regenerate affected artifacts with matching `xtask gen-*` command instead of adding another gate. |
| `tests/integration/containers/` | Container integration tests run by `make test-integration`; host/manual pre-PR tier. |
| `tests/host-integration/*.nix` | runNixOSTest VM checks run by `make test-host-integration`; local NixOS/KVM pre-PR tier, not PR pipeline. |
| `tests/integration/live/`, `tests/host-integration/hardware/` | Live-host and hardware tests. Manual only; require deployed state or real devices. |

## CI / `flake.checks`

root flake exposes these eval-only checks under
`flake.checks.<system>`:

| Check name             | What it evaluates                                                         |
| ---------------------- | ------------------------------------------------------------------------- |
| `eval-minimal`         | `examples/minimal/configuration.nix` against framework module set.    |
| `eval-multi-env`       | `examples/multi-env/configuration.nix` (two isolated envs).               |
| `eval-template`        | `templates/default/configuration.nix` with sentinel fields overridden so assertion block passes (TODO 2/3 substitutes). |
| `eval-graphics`        | `examples/graphics-workstation/configuration.nix`. **x86_64-linux only** - framework's `checkVmPlatform` gate refuses graphics on aarch64. |

`with-entra-id` is intentionally absent from root `flake.checks`
because it depends on sibling `entrablau` input, which the
core flake does not (and should not) pull in. Its own flake is
still eval-checked by `tests/static.sh` during per-example
iteration step, and CI runs
`.github/workflows/eval-with-entra-id.yml` to execute
`nix flake check --no-build --all-systems --no-write-lock-file`
inside example directory without coupling root flake to the
sibling input.

## Critical subsystems - handle with care

Touch these only with clear plan and corresponding test run. Each row
links to its full invariants in
[critical-subsystems.md](./docs/contributing/critical-subsystems.md); **read
that section before changing subsystem**, because one-line risk here
is warning, not contract.

| System | Where | Risk if broken |
| --- | --- | --- |
| [Net VM networking / firewall](docs/contributing/critical-subsystems.md#net-vm-networking-firewall) | `nixos-modules/net.nix` (`lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus per-env MTU/MSS and east-west wiring) | Net VM dual-stacks DHCP on its uplink, breaks NAT, or weakens same-env isolation unexpectedly. Validate with `tests/unit/nix/cases/net-vm-network.nix`. |
| [Per-VM `/nix/store` hardlink farm](docs/contributing/critical-subsystems.md#per-vm-nixstore-hardlink-farm) | `nixos-modules/store.nix` | guest's `/nix/store` MUST be per-VM closure-only farm `/var/lib/d2b/vms/<vm>/store`, never host's full `/nix/store`. Serving host store re-leaks it to every guest. Needs `/var/lib/d2b` and `/nix/store` on same filesystem. |
| [TPM persistence (per-VM swtpm)](docs/contributing/critical-subsystems.md#tpm-persistence-per-vm-swtpm) | `/var/lib/d2b/vms/<vm>/swtpm/` | Holds per-VM TPM 2.0 NVRAM + EK seed. |
| [USBIP passthrough](docs/contributing/critical-subsystems.md#usbip-passthrough) | `nixos-modules/components/usbip.nix` (eval-time gating) + broker `UsbipBindFirewallRule` + `SpawnRunner` (per-busid attach process supervised by `d2bd`) | Eval-time gating still scopes attach to opted-in envs (validated by `tests/unit/nix/cases/usbip-gating.nix`). |
| [GPU sidecar (graphics VMs)](docs/contributing/critical-subsystems.md#gpu-sidecar-graphics-vms) | `nixos-modules/components/graphics.nix` + broker `SpawnRunner` for cloud-hypervisor on graphics VMs | Graphics VMs run cloud-hypervisor with GPU device attached. |
| [Video sidecar (graphics VMs)](docs/contributing/critical-subsystems.md#video-sidecar-graphics-vms) | `nixos-modules/components/video/guest.nix` | `graphics.videoSidecar = true` is explicit opt-in H264 decode path: guest `virtio_media` + patched Cloud Hypervisor and crosvm. Must use `d2b-<vm>-video` principal, never `d2b-<vm>-gpu`. |
| [UI color contract / niri backend](docs/contributing/critical-subsystems.md#ui-color-contract-niri-backend) | `nixos-modules/ui-colors.nix` | compositor-agnostic `d2b.site.ui` / `d2b.envs.<env>.ui` / `d2b.vms.<vm>.ui` color model is source of truth for host/env/VM/state colors. |
| [ComponentSession capability boundary](docs/contributing/critical-subsystems.md#componentsession-capability-boundary) | `packages/d2b-contracts/src/v3/component_session.rs` | Authenticated transport evidence and attachment credits are consumed into private single session owner; do not add clone/accessor that lets callers reuse admission evidence. `SessionAuthority` is sealed and must stay sealed. |
| [Zone message bus boundary](docs/contributing/critical-subsystems.md#zone-message-bus-boundary) | `packages/d2b-bus/src/{router,registry,authorization,streams,operations}.rs` | Registration consumes single-owner capability admission; comparing clonable token is insufficient. |
| [Resource mutation seal](docs/contributing/critical-subsystems.md#resource-mutation-seal) | `packages/d2b-resource-store/src/mutation_seal.rs` + `packages/d2b-resource-store-redb/src/` + `packages/d2b-resource-api/src/` | Verified resource writes consume concrete, store-instance-bound seal by value; no generic view or unbound mutation path may return. |
| [Authoritative subject resolution](docs/contributing/critical-subsystems.md#authoritative-subject-resolution) | `packages/d2b-bus/src/router.rs` (`ZoneRegistrar`) | `ZoneRegistrar` **exclusively owns and consumes** subject resolution: peer is mapped to subject from registrar-private state using verified peer evidence. Never accept caller-supplied subject. |
| [Capability mint surface allowlist](docs/contributing/critical-subsystems.md#capability-mint-surface-allowlist) | `packages/d2b-api-surface/`, `tests/golden/api-surface/`, `packages/d2b-bus/tests/public_mint_surface.rs`, `packages/d2b-resource-store/` | **enforcing compiler leg** uses stable trait-solver ambiguity assertions in defining crates. |
| [Resource controller effects boundary](docs/contributing/critical-subsystems.md#resource-controller-effects-boundary) | `packages/d2b-controller-toolkit/src/` + `packages/d2b-core-controller/src/` | Controller and core-reconciliation engines are test-only and unwired from absent production store/watch dispatcher. |
| [Unsafe-local provider, launcher, and persistent-shell helper](docs/contributing/critical-subsystems.md#unsafe-local-provider-launcher-and-persistent-shell-helper) | `nixos-modules/options-realms-workloads.nix` | `unsafe-local` is explicit and default-denied. |
| [Manifest contract](docs/contributing/critical-subsystems.md#manifest-contract) | `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix` | Version-pinned via `manifestVersion`. |
| [Manifest bundle - private artifacts](docs/contributing/critical-subsystems.md#manifest-bundle---private-artifacts) | `docs/reference/manifest-bundle.md` + `docs/reference/schemas/v2/*.json` + `packages/d2b-core/src/` bundle DTOs + `nixos-modules/bundle*.nix` | Sensitive bundle artifacts install at `root:d2bd` 0640 and ground every broker/sandbox/runner behaviour. |
| [Control plane - `d2bd` + `d2b-priv-broker`](docs/contributing/critical-subsystems.md#control-plane---d2bd-d2b-priv-broker) | `packages/d2b-contracts/**` + `packages/d2b-core/**` + `packages/d2bd/**` + `packages/d2b-priv-broker/**` (product workspace) | **only** persistent root surfaces framework declares. |
| [Storage lifecycle / restart / synchronization](docs/contributing/critical-subsystems.md#storage-lifecycle-restart-synchronization) | Planned generated contracts in `d2b-core::{storage,process_restart,sync}` + broker storage/sync ops | Managed paths, restart adoption, locks, leases, cleanup, and degraded-state reporting are control-plane contracts. |
| [Eval-time assertions](docs/contributing/critical-subsystems.md#eval-time-assertions) | `nixos-modules/assertions.nix` | These are framework's contract with consumers. |
| [Guest-control exec session table](docs/contributing/critical-subsystems.md#guest-control-exec-session-table) | `packages/d2bd/src/{exec_session,exec_session_real}.rs` | Arbitrary `d2b vm exec` is **admin-only**; configured `d2b launch` local-VM items may use same backend with launcher authority. guestd runs every exec as workload user, never root. |
| [Unsafe-local persistent shells](docs/contributing/critical-subsystems.md#unsafe-local-persistent-shells) | `packages/d2bd/src/` shell dispatch + `packages/d2b-unsafe-local-helper/src/` | `d2b shell` remains **admin-only** for every provider. |
| [Lifecycle permission group](docs/contributing/critical-subsystems.md#lifecycle-permission-group) | `nixos-modules/host-users.nix` | Membership in `d2b` + `SO_PEERCRED` at `public.sock` accept time is **only** lifecycle authorisation surface. |
| [SSH key generation / rotation](docs/contributing/critical-subsystems.md#ssh-key-generation-rotation) | `nixos-modules/host-keys.nix` | framework owns `${cfg.site.keysDir}/<vm>_ed25519`. |
| [virtiofsd sandbox model](docs/contributing/critical-subsystems.md#virtiofsd-sandbox-model) | `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles) | virtiofsd profiles MUST declare zero host capabilities (`capabilities = []`), `requiresStartRoot = false`, and `userNamespace` block mapping in-namespace root to per-share principal (ADR 0021). |

## Don'ts (security-relevant)

- **Don't remove `lib.mkForce` from net VM's `10-eth-dhcp`
  neutralizer.** Verify any reshape of `net.nix` against
  `tests/unit/nix/cases/net-vm-network.nix` first.
- **Don't relax VM-name regex or reserved prefixes.**
  `sys-*` and `launcher` are reserved so framework can
  declare its own VMs without name collisions and so CLI
  can route subcommands unambiguously.
- **Don't break manifest contract silently.** Schema +
  prose + emitter move together, with `manifestVersion`
  bump and CHANGELOG entry.
- **Don't paper over failing assertion by deleting it.** If
  assertion is wrong, fix its predicate; if predicate
  is right but failure mode is misleading, fix message.
- **Don't reintroduce per-VM systemd unit or host-singleton
  framework service.** Every per-VM lifecycle step lives inside
  `d2bd`'s DAG executor with privileged side effects routed
  through typed `d2b-priv-broker` op (ADR 0015). Policy coverage
  lives in `packages/d2b-contract-tests/tests/policy_units.rs` and
  `policy_docs.rs`; run enabled fixture-contract lane because
  those checks are not part of `test-rust`.
- **Don't reintroduce bash CLI fallback or env-knob escape
  hatch.** Rust CLI is only operator surface;
  `D2B_LEGACY_BASH_OPT_IN`, `D2B_LEGACY_CLI`, and
  `D2B_NATIVE_ONLY` are no-ops.
- **Don't commit secrets, hostnames, real user identifiers, or
  real network ranges.** Use generic names (`alice`,
  `corp-vm`, `work`, `personal`) and RFC1918 / RFC5737 ranges
  in docs and examples. repo has no host-identifier
  leaks today; keep it that way.
- **Don't introduce new linter, formatter, or pre-commit
  hook unless explicitly requested.** `nix flake check`,
  `tests/static.sh`, and `shellcheck` (already wired into
  `static.sh`) are baseline.
- **Don't add new `nixpkgs.overlays` entry or change
  `nixpkgs.url` casually.** overlay surface is part of
  public ABI and overlay churn rebuilds world for
  every consumer.
- **Don't leak internal process markers into shipped artifacts.**
  Wave/phase/revision/follow-up/finding tags (`W3`, `W4-fu`, `P6`,
  `D5/P2.3`, `( W1fu3 H20 )`) belong in planning artifacts,
  pre-release `[Unreleased]`, ADRs, this file's process sections,
  and feature-branch commits - never in shipped source comments,
  shipped docs prose, CLI help/error text, or any CHANGELOG section.
  See [Changelog and commits](#changelog-and-commits).
  functional `d2b.defaultSwitchReadiness.<wave>` option
  surface is one deliberate exception.
- **Don't spell dash with anything but ASCII hyphen `-`.** Not in
  source, comments, string literals, CLI help or error text, documentation
  prose, ADRs, specs, changelog entries, commit messages, or PR bodies.
  banned class is every non-ASCII dash codepoint: U+2010 hyphen,
  U+2011 non-breaking hyphen, U+2012 figure dash, U+2013 en dash,
  U+2014 em dash, U+2015 horizontal bar, U+2212 minus sign, U+FE58 small
  em dash, and U+FF0D fullwidth hyphen. Where one of those would have
  separated clauses, use spaced hyphen ` - ` or restructure the
  sentence; where it joined range or compound, close it up to `-`.
  This rule names codepoints rather than printing characters, because the
  gate below would flag this very line. `make check-tier0` scans every
  tracked and every non-ignored untracked file and fails closed with the
  offending `file:line` list, so reintroduced character breaks build
  rather than surviving review. When test genuinely needs one of them
  (parser tolerance case, gate's own patterns) spell it as escape
  such as `"\u{2014}"` or `$'\u2014'`, never as character.
  One hazard is worth knowing before you paste text in: ADR-046
  work-item tokenizer treats typographic dash as token separator but a
  plain hyphen as id character, so id range that was spelled with a
  typographic dash fuses into single grammatically valid but nonexistent
  id when normalized. `spec-registry` fails closed on dangling
  dependency rather than corrupting graph. Respell such range as an
  enumeration instead of defeating check; see `Dependency/owner`
  cell for `ADR046-network-005` in
  `docs/specs/ADR-046-resources-network.md` for shape that survives
  normalization.
- dash scanner has one exact admission for inert Caveman provenance:
  `third_party/caveman/v1.10.0/LICENSE`,
  `third_party/caveman/v1.10.0/skills/caveman/SKILL.md`, and
  `third_party/caveman/v1.10.0/skills/caveman-compress/SKILL.md`, only while
  each blob matches SHA-256 in
  `third_party/caveman/v1.10.0/UPSTREAM.json`. changed blob or extra vendor
  file loses admission; every other path remains covered by general rule.
  No upstream runtime, script, external install, network access, or content
  upload is permitted.
- **Don't let host process hold realm credentials, or treat relay
  identity as local auth (ADR 0032).** Realm relay/session/provider
  credentials, remote node registries, and realm audit belong inside
  per-realm gateway guest VM - never in `d2bd`, broker, the
  host bundle, host-readable storage, or any host-side activation
  artifact. relay-authenticated peer is never mapped to local
  `Admin`; `SO_PEERCRED` + `d2b` group membership stays only
  local lifecycle authz surface. Work and personal realms never share
  gateway guest or L2 bridge.
- **Don't add ad-hoc storage, ACL, cleanup, or lock ownership paths.**
  Storage and synchronization changes must fit ADR 0034 contract:
  broker-resolved opaque ids, anchored path resolution, OFD locks with
  `O_CLOEXEC`, explicit fd transfer only, restart-aware adoption before
  cleanup, and typed degraded-state reporting instead of broad chmod,
  chown, setfacl, or `/run/d2b` sweeps. Every new host-mutable
  path or lock surface must add or reuse generated `storage.json` /
  `sync.json` row, name single repair owner, and route repair through
  that owner rather than adding second activation/broker/daemon fixer.
- **Don't write host mutation outside its ownership marker, and don't
  proceed past foreign one.** Every d2b host mutation is delimited so
  foreign configuration can be preserved byte for byte: nftables rules and
  chains in `inet d2b` table carry
  `comment "d2b managed: <ownership-id>"` and foreign tables are never
  flushed; `/etc/hosts` and `/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf`
  are delimited by `# d2b-managed begin` / `# d2b-managed end`;
  systemd-networkd is detection-only and d2b never writes it. Finding a
  foreign marker where d2b expects its own is **fail-closed**
  (`path-safety-violation`, `nm-managed-foreign-conflict`,
  `foreign-nft-rule-preserved`), never signal to overwrite. Full
  conventions in
  [critical-subsystems.md](./docs/contributing/critical-subsystems.md#cgroup-slice-naming-and-ownership-markers).
- **Don't mutate d2b cgroup outside delegation contract.** One
  canonical slice, `/sys/fs/cgroup/d2b.slice`, with per-VM directories at
  `d2b.slice/<vm>/<role>/` and process-free VM layer. Never write
  `cpuset.cpus.partition` on d2b-owned cgroup, never use threaded
  cgroups, never `cgroup.kill` slice or any ancestor of daemon-owned
  leaf, and never mutate delegated subtree as uid 0 after privilege
  drop. host cgroup root is never chowned.
- **Don't commit unredacted screenshot or visual artifact.** Before a
  screenshot is committed or attached to PR or panel prompt, remove every
  secret, credential, API key, and token; remove PII (real names, emails,
  employee or user ids); and remove sensitive output such as host paths,
  internal node names, and realm principals. Use generic placeholder
  names this file already requires. If it cannot be redacted without losing
  what it demonstrates, describe it in text instead.

## Daemon-only end-state (P6 onward)

framework declares **exactly three** root-visible units:
`d2bd.service`, `d2b-priv-broker.socket`, and
`d2b-priv-broker.service`. binding architectural decision
is recorded in
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md).

Agents working on framework MUST treat these as the
contract:

- CLI is Rust `d2b` binary, full stop. No bash
  fallback bridge; `D2B_LEGACY_BASH_OPT_IN`, `D2B_LEGACY_CLI`,
  and `D2B_NATIVE_ONLY` are no-ops.
- No framework-declared per-VM systemd units. per-VM
  lifecycle DAG runs inside `d2bd`; spawned runners
  (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound, USBIP
  attach) are launched by broker's `SpawnRunner` op and handed
  back to `d2bd` as pidfds via `OpenPidfd` / `SCM_RIGHTS`.
- No host-singleton framework services
  (`d2b-ch-exporter`, `d2b-otel-host-bridge`,
  `d2b-net-route-preflight`, `d2b-audit-check[.timer]`,
  `microvms.target`). Their work either moved into `d2bd` or
  was retired with metric / signal it produced.
- `d2b.vms.<vm>.supervisor` option has been removed; setting
  it fails eval with typed friendly message.
- polkit allowlist for legacy launcher groups is retired.
  `d2b` group membership + `SO_PEERCRED` at
  `public.sock` accept time is **only** lifecycle authorisation
  surface.
- Rust CLI does not invoke bash. `tests/tools/no-bash-ast-walker`
  is enforcing AST-level check in `test-rust`; companion
  source policy in `packages/d2b-contract-tests/tests/policy_source.rs`
  runs in enforcing fixture-contract lane
  ([ADR 0017](./docs/adr/0017-no-bash-fallbacks-invariant.md)).

### Verification gates

- `packages/d2b-contract-tests/tests/policy_units.rs` denies retired
  unit names, while `policy_lints.rs` checks ADR header and
  cross-references and `policy_docs.rs` checks this file's daemon-only
  wording. These fixture-dependent policies are not enforcing
  pull-request evidence until `test-fixture-contracts` is enabled and
  promoted.
- Host exit criterion: on deployed host,
  `systemctl list-units --no-pager --all | grep -E '^(d2b|microvm)' | wc -l`
  returns `3`.

## References

Process and contributor docs:

- [`docs/contributing/`](./docs/contributing/) - workflow, panel review,
  changelog and commits, gates and lints, critical subsystems, architecture
  conventions.
- [`tests/AGENTS.md`](./tests/AGENTS.md) - binding operating manual for the
  test tree. [`tests/README.md`](./tests/README.md) is human quick-start.

Binding architectural decisions:

- [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md) - daemon-only
  end-state: `d2bd` + `d2b-priv-broker` are only persistent root surfaces.
- [ADR 0017](./docs/adr/0017-no-bash-fallbacks-invariant.md) - Rust CLI
  never invokes bash.
- [ADR 0018](./docs/adr/0018-microvm-nix-removal.md) - d2b owns its per-VM
  substrate; `microvm.nix` input is gone.
- [ADR 0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md) - broker
  pre-establishes user namespace so virtiofsd holds zero host capabilities.
- [ADR 0031](./docs/adr/0031-bare-command-and-detached-exec.md) - bare command
  resolution and detached workload-user exec.
- [ADR 0032](./docs/adr/0032-d2b-v2-constellation-control-plane.md) - host
  holds no realm credentials, and relay identity is never local auth.
- [ADR 0034](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md) -
  daemon restarts are continuation events; adopt before cleanup.

Design and contracts:

- [README.md](./README.md) - consumer-facing intro and install.
- [SECURITY.md](./SECURITY.md) - disclosure path and scope.
- [CHANGELOG.md](./CHANGELOG.md) - Keep Changelog.
- [design.md](./docs/explanation/design.md) - threat model and defenses.
- [daemon-lifecycle.md](./docs/explanation/daemon-lifecycle.md) - DAG
  executor, pidfd handoff, supervisor reconciliation.
- [privileges.md](./docs/reference/privileges.md) - broker op catalogue.
- [daemon-api.md](./docs/reference/daemon-api.md) - `public.sock` wire
  surface, audit format, retention.
- [manifest-schema.md](./docs/reference/manifest-schema.md) - manifest
  contract.
- [cli-contract.md](./docs/reference/cli-contract.md) - lifecycle FSM, signal
  semantics, exit codes.
- [naming-conventions.md](./docs/reference/naming-conventions.md) - canonical
  glossary of internal identifiers.
- [LICENSE](./LICENSE) - Apache-2.0.
