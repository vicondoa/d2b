# AGENTS.md

Operating manual for AI coding agents (Copilot CLI, GitHub Copilot,
Cursor, …) and human contributors working on **`vicondoa/nixling`
itself**. If you are *consuming* nixling in your own NixOS host
config, start at [README.md](./README.md) instead — this file is for
people changing the framework.

## What this is

Nixling is an opinionated NixOS desktop microVM framework that
owns its microVM substrate end-to-end. The control plane is
**daemon-only**: `nixlingd` supervises every per-VM DAG and
`nixling-priv-broker` dispatches every audited host mutation.
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
├── .github/workflows/              <- CI-only checks that stay out of root `flake.checks`
├── nixos-modules/                  <- THE framework
│   ├── default.nix                 <- aggregator imported as nixosModules.default
│   ├── options.nix / options-*.nix <- option schema (site / envs / vms)
│   ├── assertions.nix              <- eval-time invariants (CIDR overlap, platform gate, …)
│   ├── lib.nix                     <- internal helpers (subnetIp, mkMac, …)
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
│   ├── nixling-core/              <- shared bundle DTOs, typed errors, privilege metadata
│   ├── nixling-host/              <- host-side lifecycle primitives (argv, hardlink farm, ifnames)
│   ├── nixling-ipc/               <- public + private wire contracts
│   ├── nixling/                   <- rust-native CLI
│   ├── nixlingd/                  <- unprivileged public daemon / supervisor
│   ├── nixling-priv-broker/       <- privileged broker for audited host mutations
│   └── xtask/                     <- schema / docs codegen helpers; see
│                                      `docs/adr/0000` + `docs/adr/0009`
├── tests/                          <- see "Test layout" below
├── examples/                       <- minimal / graphics-workstation / multi-env / with-entra-id
├── templates/default/              <- `nix flake init -t github:vicondoa/nixling`
└── docs/                           <- Diataxis tree (explanation / how-to / reference)
                                       plus `docs/adr/` architecture decision records
```

New behaviour belongs in a focused file under `nixos-modules/`
(or `nixos-modules/components/` for per-VM toggles), wired in
from `nixos-modules/default.nix`. Don't fatten existing files.

## Build & validate

> **Test interface = `make` targets** (test-rearchitecture, in progress). Run
> tests via `make`, not ad-hoc scripts. **`make check` is the done-gate: an
> agent has not finished a test-affecting change until `make check` passes.**
>
> | target | what | runs in CI? | needs |
> | --- | --- | --- | --- |
> | `make check` | Layer-1 PR gate (today wraps `static.sh`) | yes (any runner) | Ubuntu+Nix |
> | `make check-ci` | W0: `check` + `test-integration` placeholder | no — `make check` remains the real CI gate today | Ubuntu+Nix |
> | `make test-integration` | W0 placeholder: legacy G-ci scripts only on NixOS+KVM; runNixOSTest CI job lands W4 | no | NixOS host + KVM |
> | `make test-hardware` | real GPU/YubiKey/hardware-TPM passthrough, full microVM boot (G-hw) | **no** | NixOS host **with the devices** |
> | `make check-all` | `check-ci` + `test-hardware` + `perf` | — | NixOS host w/ devices |
> | `make test-{rust,drift,contract,nix-unit,flake,policy}` | focused per-layer run (ledger-driven) | — | — |
> | `make check-inventory` | assert `tests/` is classified 1:1 in `tests/migration-ledger.toml` | yes | — |
>
> W0 does not move live-host scripts into CI: `make check` (today `static.sh`) remains
> the real CI gate. The `test-integration` target is a safe placeholder that skips
> unless it is run on a NixOS host with KVM; the runNixOSTest CI job lands in W4.
> Test→group classification lives in `tests/migration-ledger.toml` (check with
> `make check-inventory`; regenerate with `make ledger-regen`).

The four legacy static tiers below are being repointed behind the `make`
targets wave by wave; pick the one that matches your intent.

```bash
# 1. Flake-level eval, both systems we support.
nix flake check --no-build --all-systems

# 2. Tier-0 fast static gate. Shell syntax + shellcheck on the
#    repo's bash entrypoints in under a minute. Run it before a
#    broader review loop and after doc/shell-only rebases.
bash tests/static-fast-tier0.sh

# 3. Fast PR-loop gate. Catches parse / shellcheck / flake
#    check / rust workspace / bundle invariants / host-prepare
#    canaries / cross-cutting drift in ~13 min cold (~2 min warm),
#    ~520 G peak /nix/store. Run before every commit and after every
#    rebase. Does NOT exercise the eval gates, mid-tier consumer-config
#    evals, manifest contract, broker daemons, per-example
#    flake-check, or audio component — those land in tier (4) below.
bash tests/static-fast.sh

# 4. Full panel/wave-exit gate (the canonical Layer 1 set). Adds
#    smoke-eval, assertions-eval, observability-eval, mid-tier evals,
#    manifest contract, control-plane gates, per-example flake-check,
#    cli-contract-coverage, cli-json-drift. ~30-90 min cold,
#    peak disk capped at ~400 G via per-phase nix store gc.
#    Set NL_GATE_DISK_BUDGET_GIB=300 to fail-closed at the phase
#    boundary if free disk drops below the budget.
bash tests/static.sh

# 5. Optional focused checks (called transitively by static.sh, also
#    useful standalone while iterating):
bash tests/assertions-eval.sh        # consolidated batch eval +
                                     # fallback for the 3 throw cases
                                     # (~13 min cold)
nix-instantiate --eval --strict \
  -E 'let f = import ./tests/smoke-eval-tpm.nix; r = f {}; in r.drvPath' \
  >/dev/null
bash tests/net-vm-network-eval.sh    # net VM networkd config invariants
bash tests/usbip-gating-eval.sh      # host-side USBIP gating + env scoping
bash tests/cli-json.sh               # daemon CLI JSON envelope contract
bash tests/legacy-unit-denylist-eval.sh  # fail-closed gate (ADR 0015)
bash tests/agents-md-rewrite-eval.sh # AGENTS.md docs invariant
```

Layer-2 integration tests (`tests/nixling-store.sh`) require a live
host with `nixlingd` + `nixling-priv-broker` active; they're
documented in [`tests/README.md`](./tests/README.md). Most
contributors do **not** need to run them — Layer 1 is the PR gate.

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

#### Finish-of-work invariant: merge back into the primary clone

A worktree is a workspace, not a destination. When an agent's scope
is done — implementation green, tests green, panel signed off — the
agent merges the worktree branch back into `main` in the **primary
clone (`projects/nixling`)** before declaring the task complete.
Finished work sitting on a side worktree branch is not done; it is
"awaiting integration", which is a state the agent owns, not a state
the agent leaves for the operator.

Concretely, the agent that owns a worktree:

1. Verifies green on the worktree (`cargo test --workspace`, the
   relevant `tests/*.sh` gates, panel signoff for plan-driven work).
2. From the primary clone (`/home/paydro/projects/nixling`),
   fast-forwards (or octopus-merges, per the rules above) the
   worktree's `phase-<name>` branch into `main`.
3. If there is unrelated dirty WIP in the primary clone (operator
   was editing in place), stash it, do the merge, pop the stash,
   resolve any textual conflicts in a way that preserves both sets
   of changes, then leave the operator's WIP unstaged so they can
   commit it on their own terms.
4. Audits sibling worktrees (`git worktree list`) for branches
   whose tip is unmerged but represents abandoned/superseded work;
   flag those for the operator rather than silently dropping them.

Only after the merge lands does the agent call `task_complete`.

### Local host validation after updating nixling

When a host configuration switches to a new nixling checkout (for
example a local `path:/home/paydro/projects/nixling` input), the host
switch updates `/etc/nixling/*` and the system packages but does **not**
restart `nixlingd` (`restartIfChanged = false`). Before runtime
validation, restart the daemon explicitly so it reloads the updated
bundle/process contract and binary paths:

```bash
sudo systemctl restart nixlingd.service
```

Then restart affected VMs with the normal lifecycle commands (on this
host, prefer `nixling down <vm> --apply` followed by
`nixling up <vm> --apply`; `nixling switch <vm>` is not reliable here).

#### Integrator-prep-first pattern (W3 onwards)

For waves whose thematic scopes are NOT file-disjoint by default —
W3 host-prepare is the canonical example, with scopes s1–s5
naturally sharing `packages/nixling-ipc`, `packages/nixling-core`
DTOs, schemas, and `Cargo.toml` workspace pins — the wave is
preceded by an **integrator API/contract prep commit landed
directly on `main`** before any scope worktree is opened. That
prep commit:

- adds every shared crate, DTO module, broker enum variant,
  privileges row, schema regeneration, and `Cargo.toml`
  workspace-dep change the parallel scope commits will read;
- carries the canonical trailing tag `( W3 )` (no scope label
  inside the parens — scope labels are subject-prefix only,
  e.g. `s2 host: reconcile bridge port flags ( W3 )`);
- leaves every scope's owned files untouched so each scope
  worktree opens against a stable contract.

Follow-up rounds use `( W3fu<M> )` for the integrator octopus
merge and `( W3fu<M> H<N> )` for per-finding hardening commits,
matching the W2fu4 H10/H18 canonical-tag rules above.

The W3 file-ownership map lives in the wave plan
(`~/.copilot/session-state/<id>/plan.md` §"W3 file-ownership map"
for the current wave); scope agents read it before opening their
worktree and write only to their listed files.

### Edit → commit → validate

Commit before running `static.sh` / the smoke evals. Two reasons:

1. Untracked files are invisible to `nix flake check` (and to any
   eval that follows the same code path). Forgetting to `git add` a
   new module is the #1 "why doesn't my change apply?" pitfall.
2. Consumer hosts that vendor nixling tend to ship auto-backup
   tooling that catch-all-commits any dirty tree. That's a
   consumer-side concern, but the habit of committing-then-building
   is the right one to carry into framework work too.

For plan-driven multi-phase work, green tests are not enough to
advance the work. See [Panel review](#panel-review): the
integrator may not dispatch implementation subagents for a phase,
or begin the next phase, until the relevant panel gate passes.

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

The framework declares **exactly three** root-visible units. There
is no `nixling@<vm>`-style per-VM unit; `nixlingd` supervises every
per-VM DAG in-process and hands fds to spawned runners via the
broker's `SpawnRunner` / `OpenPidfd` ops.

| Resource                                | Pattern                                |
| --------------------------------------- | -------------------------------------- |
| Public daemon (supervisor)              | `nixlingd.service`                     |
| Privileged broker socket                | `nixling-priv-broker.socket`           |
| Privileged broker service               | `nixling-priv-broker.service`          |
| Lifecycle permission group              | `nixling` (singleton)                  |

VM names are validated at eval time:

- Regex: `^[a-z][a-z0-9-]*$`.
- Reserved prefix: `sys-` (only the framework declares `sys-*` VMs).
- Reserved exact name: `launcher`.

Breaking any of these is a hard assertion in
`nixos-modules/assertions.nix`.

For the canonical glossary of internal identifiers (DAG node names,
bundle-relative artefact paths, broker op IDs) see
[`docs/reference/naming-conventions.md`](./docs/reference/naming-conventions.md).

### Component split & sibling flakes

The **core framework** in this repo covers: graphics, tpm, usbip,
audio, network, the auto-declared net VM, the per-VM store, the
CLI, the manifest contract.

Anything **identity- or workload-specific** lives in a sibling
flake and is composed per-VM:

- [`vicondoa/entrablau.nix`][entrablau] — Microsoft Entra ID
  joins (Himmelblau + TPM-bound machine credential).

The composition pattern is intentionally one-way: sibling flakes
know nothing about nixling, and nixling knows nothing about them.
Consumers compose them on a specific workload VM:

```nix
nixling.vms.work.config.imports = [
  inputs.entrablau.nixosModules.default
];
```

If you're tempted to add a new sibling-shaped concern (e.g. a
specific desktop environment, a particular dev-shell flavour) to
the core framework, consider whether it belongs in its own flake
instead. The bar for landing it in core is: "every nixling user
plausibly wants this, and the framework cannot do the right thing
without it."

[entrablau]: https://github.com/vicondoa/entrablau.nix

### VM lifecycle (daemon-supervised)

`nixlingd` is the sole supervisor for every per-VM lifecycle DAG.
There are no framework-declared per-VM systemd units: child
processes (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound,
USBIP attach) are spawned by the broker via `SpawnRunner`, handed
back to `nixlingd` over `SCM_RIGHTS` as pidfds, and reconciled
against the persisted DAG state under
`/var/lib/nixling/supervisor/state.json`.

The `restartIfChanged = false` invariant applies to the two daemon
units themselves (no per-VM units are emitted):

- `nixlingd.service` carries `restartIfChanged = false`; rebuilds
  do not cycle a running daemon. Operators apply pending daemon
  changes explicitly via `nixling host doctor` (reports the
  pending-restart state) and `systemctl restart nixlingd` once the
  supervisor reconciliation window is acceptable.
- `nixling-priv-broker.service` is socket-activated. It reloads the
  current bundle resolver for each accepted request so a running broker
  does not dispatch stale runner intents after a switch, and it never
  holds in-flight session state across requests.

Drift detection moves from per-VM symlinks into the daemon's
state file. `nixling vm list` flags any VM where the running
closure differs from the latest declared closure with
`[pending restart]`; `nixling vm status <vm>` prints both store
paths and the exact remediation command (`nixling vm restart <vm>`
for a clean down+up, `nixling vm switch <vm>` for a per-VM closure
rebuild + live activation).

#### Adding new per-VM behaviour

New per-VM work belongs **inside the daemon's DAG executor**
(`packages/nixlingd/src/supervisor/`), with any privileged side
effects routed through a typed `nixling-priv-broker` op declared
in `packages/nixling-ipc/` and audited in
`/var/lib/nixling/audit/broker-<utc-date>.jsonl`. Do not introduce
a new `systemd.services.*` declaration in `nixos-modules/` for
per-VM work — the `tests/legacy-unit-denylist-eval.sh` gate will
reject it. See
[`docs/explanation/daemon-lifecycle.md`](./docs/explanation/daemon-lifecycle.md)
for the DAG node taxonomy and
[`docs/reference/privileges.md`](./docs/reference/privileges.md) for
the broker op catalogue.

## Panel review

### Phase gate

Multi-phase plans MUST pass a panel sign-off gate at each phase
boundary. The integrator MUST NOT begin the next phase until 8/8
(or N/N for the plan's panel size) reviewers return `signoff: true`.

For plan-driven work, a "phase" is usually one wave from the plan's
parallelization graph (`Wave 0`, `Wave 1`, ...). For tiny plans that
touch fewer than three files, a single phase covering the whole plan is
acceptable.

For each phase:

1. **Plan review** — panel reviews the plan; iterate until 8/8
   sign-off (or N/N for the selected panel size). The integrator may
   not dispatch implementation subagents until this gate passes.
2. **Implementation** — dispatch subagents in parallel per the
   dependency graph.
3. **Integration** — integrator merges subagent output.
4. **Work review** — panel reviews the integrated diff; iterate via
   fix-subagents until 8/8 sign-off (or N/N for the selected panel
   size).
5. **Advance** — only now may the integrator begin the next phase's
   plan review.

Panel prompts MUST include the validation evidence the integrator already
ran for the phase (commands and pass/fail results) and MUST instruct
reviewers not to rerun tests, builds, evals, or other long validations
unless the integrator explicitly requests that reviewer to do so.
Reviewers should inspect the plan or diff, reason over the supplied
evidence, and call out missing or insufficient validation as a finding
rather than duplicating the validation themselves. This keeps panel
review from stampeding the shared Nix store, cargo target, and git
worktrees while parallel implementation agents are still active.

Each engineer returns a JSON sign-off record shaped like:

```json
{
  "engineer": "software",
  "signoff": true,
  "summary": "What was reviewed and the overall posture.",
  "recommendations": []
}
```

By policy, `signoff` is `true` iff `recommendations` is `[]`.
Otherwise, `recommendations[]` carries the actionable findings. If any
reviewer returns findings, the integrator spawns follow-up
implementation agents, lands the fixes, reruns the tests, and starts
another panel round. Green tests do not waive this gate; a phase closes
only on unanimous sign-off.

Escape hatches are narrow:

- **Trivial fixes** (typo, one-line, no semantic change) may skip the
  panel gate.
- **Time-critical hotfixes** (production breakage) may skip the
  pre-fix panel, but MUST run a post-fix panel before the incident is
  considered closed.
- **Documentation-only changes** may skip the panel gate unless the doc
  change describes a load-bearing behavior.

Autopilot prompts encourage "bias to action." That is in tension with
the panel gate. When in doubt, run the panel. A two-hour panel that
catches one HIGH finding is cheaper than re-doing two days of
integration.

Canonical precedent: an early observability Wave-1 panel returned
0/8 sign-offs with 11 HIGH findings. `tests/static.sh` caught none of
them. This is the canonical "you can't test your way out of needing a
panel" data point.

### Default panel

| Engineer          | Focus |
|-------------------|-------|
| `software`        | Shell + Nix shape of every new module, daemon instrumentation, idempotency of sidecars, error handling in metric exporters. |
| `test`            | Coverage of new option schema, vsock CID collision cases, restart-policy gates, manifest schema drift, and what could regress invisibly. |
| `nixos`           | Module wiring, `lib.mkForce` / `lib.mkDefault` correctness, option declarations, systemd unit composition, and activation ordering. |
| `networking`      | Network surface changes, firewall posture across envs, DHCP/DNS regressions, bridge isolation, and routing invariants. |
| `security`        | Attack surface, host-relay trust posture, capability sets / syscall filters, authz boundaries, telemetry-label PII review, and retention defaults. |
| `rust`            | Rust API shape, error propagation, unsafe/FFI boundaries, schema generation, workspace dependency direction, and testability. |
| `product`         | Operator UX, naming surface, migration/deprecation policy, default-off opt-in shape, and actionable error messages. |
| `docs`            | Diataxis adherence in `docs/{reference,how-to,explanation}/`, CHANGELOG entries, schema md↔json drift, and AGENTS.md updates landing with load-bearing changes. |
| `observability`   | Cardinality of metric labels, span attribute hygiene (no secrets/cmd output/store paths), log/audit shape, retention, and dashboard/exporter correctness. |
| `kernel`          | pidfd, cgroup, namespace, mount, signal, ioctl, and filesystem semantics; kernel-version assumptions and Linux API edge cases. |

Older commits and [CHANGELOG.md](CHANGELOG.md) entries may reference
the historical six-engineer security-hardening roster (`nixos`, `rust`,
`software`, `test`, `networking`, `security`) or the earlier
observability-specific roster. The unified default panel above
supersedes both for new work.

Host-local roster files under `/etc/nixos/scripts/` are operator
configuration and are out of scope for this repository; keep repo docs
focused on the review contract rather than paydro-specific files.

### Commit-tag mapping

The tag examples in [Commit conventions](#commit-conventions) use this
mapping, and every commit that comes out of a panel-fix round MUST
carry the relevant tag:

- `Wn` = wave / phase number from the plan's parallelization graph
- `Wnfu` = first follow-up round on wave `n` after the first panel
  findings land
- `Wnfu<M>` = follow-up round `M` on wave `n` when a specific
  follow-up round must be named (for example `W5fu1`)
- `CN`, `HN`, `MN`, `LN` = finding ordinal `N`, prefixed by the
  severity letter from the JSON output (`critical` → `C`, `high` →
  `H`, `medium` → `M`, `low` → `L`)

Example: `( W1fu1 H3 )` means "wave 1, follow-up round 1,
addresses finding ranked HIGH-3."

Inline references to a specific commit in prose elsewhere may
use the compact form `(W2fu4 H10)` for readability — that's
shorthand for citing a commit, not the literal trailing tag
that the commit subject must end with. The trailing-tag form
in the commit subject itself always uses the spaced canonical
form (e.g. `... ( W2fu4 H10 )`).

### Tooling note

One host-local implementation lives in
`/etc/nixos/scripts/panel-review.{md,sh}` and
`/etc/nixos/scripts/panel-aggregate.sh`. That tooling is paydro's
host-specific implementation, not an upstream nixling dependency;
alternative implementations are welcome if they preserve the same
review contract.

In that implementation, the roster is selected per plan via
`ENGINEERS_FILE`, and each engineer's focus file comes from
`panel-roles/<engineer>.md`.

## Test layout

| File                                  | Role                                                                                         |
| ------------------------------------- | -------------------------------------------------------------------------------------------- |
| `tests/static-fast-tier0.sh`          | **Tier-0 sub-60s fast gate.** Bash syntax + `shellcheck` on repo entrypoints and helpers; no Nix eval/build work. |
| `tests/static-fast.sh`                | **Fast PR-loop gate.** Parse / `shellcheck` / `flake check` / rust workspace / bundle invariants / host-prepare canaries / cross-cutting drift without the full panel-exit set. |
| `tests/static.sh`                     | **Top-level Layer-1 gate.** Parse, `flake check`, smoke evals, assertion/observability/USBIP/autostart/restart-policy eval gates, manifest contract, per-example flake checks. Runs from repo root; override with `ROOT=<path>`. |
| `tests/smoke-eval.nix`                | Workload smoke: minimal consumer-style nixosSystem, builder's native system.                 |
| `tests/smoke-eval-graphics.nix`       | Same shape, with `graphics.enable = true`. x86_64-only.                                      |
| `tests/smoke-eval-tpm.nix`            | TPM host-surface regression gate: swtpm parent-dir ACLs, swtpm flush helper, runner socket ACLs, and ownership-migration invariants. |
| `tests/smoke-eval-aarch64.nix`        | Headless smoke cross-evaluated on aarch64-linux (multi-arch eval-graph regression gate).     |
| `tests/assertions-eval.sh`            | Negative assertion cases (CIDR overlap, naming invariants, platform gate, missing `waylandUser`, reserved-path invariants, boot-cleaned `tmpDir`, etc.). Each bad case must fail eval with the expected message. |
| `tests/usbip-gating-eval.sh`          | Host-side USBIP gating: absent until both host + enabled-VM opt-ins are set, and scoped to opted-in envs only. |
| `tests/niri-vm-borders-eval.sh`       | Opt-in niri KDL border generation: disabled by default, correct window-rule per graphics VM when enabled, per-VM color override, default color stability, and custom `outputPath`. |
| `tests/net-vm-network-eval.sh`        | Net VM networkd + nftables invariants — most importantly the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus per-env MTU/MSS, cross-env drops, and east-west toggles. |
| `tests/volume-mounts-eval.sh`         | Declared `microvm.volumes` invariant: Cloud Hypervisor disk serials and guest `fileSystems` mounts stay aligned, and duplicate/reserved/overlong serials fail eval. |
| `tests/video-sidecar-hardening-eval.sh` | Eval-time hardening gate for the broker `SpawnRunner` video runner descriptor (`AF_UNIX` only, syscall filter, empty capability sets). |
| `tests/minijail-validator-wayland-proxy.sh` | Wayland filter proxy minijail profile gate: mandatory seccomp, empty capabilities, empty device binds, dedicated runtime dir (`/run/nixling-wlproxy/<vm>`), no PipeWire/Pulse socket access; compositor access is granted to the `wlproxy` role by ACL, not by a profile bind mount. |
| `tests/state-dir-acl-runtime.sh`      | **Layer-2 + root-only.** Skips unless `NL_RUN_LAYER2_WITH_SUDO=1 sudo -n bash tests/state-dir-acl-runtime.sh` is run. `.github/workflows/layer2-runtime-with-sudo.yml` is **manual-dispatch only** on a self-hosted `nixling-sudo` runner — never `pull_request` (panel R9 security: passwordless-sudo on PR-controlled checkout). Maintainers dispatch via `gh workflow run layer2-runtime-with-sudo.yml --ref <ref>` after review. See `CONTRIBUTING.md` § "Provisioning the `nixling-sudo` self-hosted runner". |
| `tests/bridge-isolation-runtime.sh`   | Hermetic runtime bridge-isolation test: net-VM port stays reachable, workload taps stay isolated even after peer-style MAC spoofing. |
| `tests/legacy-unit-denylist-eval.sh`  | Fail-closed gate: no example's `nixos-rebuild dry-build` output emits a retired per-VM systemd template or host-singleton framework service (ADR 0015). |
| `packages/nixling-contract-tests/tests/policy_lints.rs::adr_0015_present_with_header_and_cross_references` | Asserts the daemon-only ADR exists, carries the canonical header, and is cross-referenced from `AGENTS.md`. |
| `tests/agents-md-rewrite-eval.sh`     | Asserts `AGENTS.md` does not describe the legacy bash CLI or retired per-VM systemd templates as live framework surfaces. |
| `tests/nixling-store.sh`              | Layer 2, optional. Per-VM `/nix/store` hardlink farm + `nixling vm switch` lifecycle. Requires a live host. |
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
because it depends on the sibling `entrablau` input, which the
core flake does not (and should not) pull in. Its own flake is
still eval-checked by `tests/static.sh` during the per-example
iteration step, and CI also runs
`.github/workflows/eval-with-entra-id.yml` to execute
`nix flake check --no-build --all-systems --no-write-lock-file`
inside the example directory without coupling the root flake to the
sibling input.

## Versioning & changelog

The project follows [Semantic Versioning](https://semver.org/) and
[Keep a Changelog](https://keepachangelog.com/). The CHANGELOG is
organised **by version**, never by development phase.

### Changelog lifecycle

- **While a version is in development**, entries accumulate under the
  top `## [Unreleased]` block. Because `[Unreleased]` is a
  pre-release staging area, it MAY carry fine-grained process detail
  (wave/phase/follow-up/finding notes) if that helps the people
  cutting the release reason about what landed.
- **When a version is cut**, the `[Unreleased]` block is renamed to
  `## [X.Y.Z] - YYYY-MM-DD` and its contents are **summarised by
  version**:
  - Collapse any per-wave/per-phase substructure into the standard
    Keep-a-Changelog groups (`Added`, `Changed`, `Fixed`,
    `Deprecated`, `Removed`, `Security`). There are no
    `### Added (W6)`-style subsection headers in a released section.
  - Strip every internal process marker — wave/phase/revision/
    follow-up/panel/round/finding tags such as `W3`, `W4-fu`,
    `( W1fu3 H20 )`, `P6`, `D5/P2.3` — from the released prose.
  - Each released section reads as a coherent, consumer-facing
    summary of what changed, not as a log of how the work was
    organised internally.
- A fresh empty `## [Unreleased]` block is left at the top after a
  cut. `manifestVersion` / `bundleVersion` bumps and breaking
  changes always get an explicit released entry.

### Process markers stay out of shipped artifacts

Internal development bookkeeping — wave tags (`W3`, `W4-fu`,
`W2-followup`), phase tags (`P0`–`P7`, `v1.1-P4`, `ph6-…`),
decision codes (`D5/P2.3`), follow-up/round/finding refs
(`fu3`, `H20`, `(rust-1)`) — is for organising work, not for
shipping. Do **not** introduce these markers into:

- source comments in `nixos-modules/`, `pkgs/`, or `packages/`;
- shipped docs prose under `docs/{reference,how-to,explanation}/`,
  `README.md`, `SECURITY.md`, or example READMEs;
- any user-facing CLI surface (`clap` `about`/`help`/`long_help`
  text, error/observed-state messages, JSON envelope fields);
- CI workflow names, job names, step names, and test output that a
  contributor sees in GitHub Actions logs. CI labels should describe
  the behavior being validated (for example, "ADR index coverage
  guard" or "host validate dry-run"), not historical phase/process
  codes;
- released CHANGELOG sections.

These markers are still expected and welcome in the contexts where
they are load-bearing:

- planning artifacts (a session `plan.md`, the wave/parallelization
  graph) and pre-release CHANGELOG `[Unreleased]`;
- this file and the other process docs (Panel review, Commit
  conventions, `## Daemon-only end-state (P6 onward)`) that
  *document* the methodology;
- `docs/adr/**` — ADRs are dated historical records and may name the
  wave/phase that produced a decision;
- commit messages and PR descriptions on in-development feature
  branches (see Commit conventions).

Note the deliberate exception: the consumer-facing
`nixling.defaultSwitchReadiness.<wave>` option namespace (keys
`w4Fu`…`p7`), its `readinessWaveSpecs` schema, and the
`/var/lib/nixling/validated/<wave>.json` evidence contract use
`wave`/phase tokens as **functional identifiers**. Those are part of
the public option/schema surface and are not bookkeeping; leave them.

### Landing changes (PR workflow)

`main` is protected: changes land via pull requests, not direct
pushes. Develop on a feature branch (or worktree), validate locally
against the gates above, open a PR, let CI / panel review run, then
squash-merge. The detailed wave-tag commit convention in
[Commit conventions](#commit-conventions) applies to in-development
commits on those feature branches; `main` itself is maintained as a
by-release history.

## Commit conventions

> The trailing wave-tag scheme below applies to in-development
> commits on feature branches / worktrees, where wave/phase tags are
> load-bearing planning context. It does not license process markers
> in shipped code, docs, or released CHANGELOG sections — see
> [Versioning & changelog](#versioning--changelog).

- **Subject.** Short, imperative, prefixed with the touched
  area: `net: fix 10-eth-dhcp neutralization`,
  `manifest: bump manifestVersion to 2`,
  `cli: tighten exit-code table`.
- **Body.** Wrap at ~72 cols. Explain *why*, not what — the diff
  shows the what.
- **Traceability — canonical tag form (forward, W2fu4+).**
  Every commit subject MUST end with a trailing parenthesized
  tag in one of these exact forms:

  - `( W<N> )` — wave-N implementer work (no finding ref)
  - `( W<N>fu<M> )` — wave-N follow-up round M integrator
    merge (no finding ref); merge-shape suffixes like
    `octopus` are NOT permitted in the tag
  - `( W<N>fu<M> <S><N> )` — single finding fixed in
    follow-up round M. The finding-tag is `<S><N>` where
    `<S>` is the severity letter from the reviewer JSON
    (`C` = critical, `H` = high, `M` = medium, `L` = low)
    and `<N>` is the ordinal within that severity. Example:
    `( W2fu1 H3 )` = wave 2, follow-up 1, HIGH-3.
  - `( W<N>fu<M> <S1><N1> <S2><N2> ... )` — multi-finding
    follow-up commit when two or more findings genuinely express
    one coherent change and scattering them would not add
    review value. The trailing tag enumerates every finding
    closed by the commit, separated by single spaces. The commit
    body MUST explicitly call out the multi-finding scope (which
    findings are closed and why batching them in one commit
    aids review). Example: W3fu3 `( W3fu3 H4 H5 H6 )` aligned
    three docs (`privileges.md`, `AGENTS.md`,
    plan.md "Spec corrections") to point at `schemas/v2/` as
    the current bundle baseline in a single coherent commit.
    Reach for the single-finding form by default; reach for
    multi-finding only when the alternative is three or more
    trivially-small commits that all express the same
    statement.
  - `( W<N> <S><N> )` — single finding fixed inside the
    wave itself (rare; usually findings come during follow-ups)
  - `( W<N>a-<H> )` or `( W<N>a H<H> )` — post-wave **opening
    phase** that closes specific Spec-corrections deferrals or
    ships infrastructure work. Used when the work is genuinely
    pre-wave-N+1 prep rather than an in-wave follow-up. Examples:
    `( W3a-1 )` for the W3a-1 testing-infra batched harness,
    `( W4a H1 )` for the W4a-H1 audit retention commit. The
    spelling with the space (`W4a H1`) is what the W4a
    landings used and is the canonical form going forward; the
    dash-form (`W3a-1`) is permitted as a historical exception
    for the W3a commits that already shipped. Multi-finding
    follow-ups within an opening phase use the same
    `( W<N>afu<M> <S1><N1> <S2><N2> ... )` shape as a normal
    wave round (e.g. `( W4afu1 H1 H2 )` for a W4a follow-up
    closing R1 findings).

  Docs-only commits that don't close a specific finding (e.g.
  CHANGELOG.md grouping, AGENTS.md operating-manual updates after
  a wave closes) MAY omit the trailing tag when the subject
  itself is unambiguous about the scope (e.g. `CHANGELOG: W3fu4
  H1 H2 H3 H4 H5 grouped entry (R4 closure)`). Reach for the
  tag form whenever doing so would aid traceability; treat omitting
  it as the exception, not the default.

  No leading-tag form. No partition/topic words inside the
  parenthesized tag — those go in prose. Every commit
  produced in a panel-fix round MUST carry the relevant
  tag; see [Panel review](#panel-review) for the mapping
  and phase-gate policy.

  Historical exception: pre-W2fu4 commits in W0/W1/W2 carry
  some leading-tag variants (`(W2 s3) ...`) and some merge
  subjects with topic words (`(W2fu1 ipc)`, `(W2fu2 octopus)`).
  These remain in history for reference; future waves use the
  canonical form above. See the
  `docs: codify trailing-tag canonical form` commit
  (W2fu4 H10) for the full retrospective.

- **Signing.** Sign-offs / GPG signing are not used.
- **Atomicity.** One logical change per commit. Mechanical
  reformat or rename passes go in their own commit so the
  human-reviewable diff stays small.

## Disk hygiene contract

- Test eval expressions MUST resolve the flake via `git+file://$ROOT`
  (use the `nl_flake_ref` helper in `tests/lib.sh`), **never**
  `builtins.getFlake (toString $ROOT)`. A bare path makes Nix use the
  `path:` fetcher, which copies the ENTIRE working tree into the store —
  including the multi-GiB `packages/target` cargo artifacts (measured:
  ~36 GB / 5+ min per cold eval, re-triggered every time a cargo build
  churns `target/`). `git+file://` copies only git-tracked files
  (`target/` is gitignored), turning a 5-minute eval into <1 s. Caveats:
  (a) `nix eval` is pure by default and needs `--impure` with git+file;
  `nix-instantiate --eval` is impure by default and needs no flag.
  (b) When a script captures eval output via `2>&1` into a variable it
  then parses (jq, etc.), add `--quiet --no-warn-dirty` so the git+file
  `fetching git input` / `Git tree is dirty` stderr diagnostics don't
  corrupt the parsed JSON. (c) git+file sees uncommitted edits to
  TRACKED files but NOT untracked files — identical to `nix flake check`,
  so "commit before building" still holds (see "Edit -> commit ->
  validate").
- Every test script that creates repo-local scratch state MUST use
  `nl_mktemp` from `tests/lib.sh`; do not call raw
  `mktemp -d -p "$ROOT"`.
- Per-process bookkeeping (`cleanups.<PID>`, `scratch-registry`)
  lives in `${NL_BOOKKEEPING_DIR:-${TMPDIR:-/tmp}/nixling-bookkeeping}`,
  NOT in `$ROOT`. Parallel-test timing log/status files live in
  `${TMPDIR:-/tmp}/nixling-static-timing.$$/`. Both moves are
  required so volatile files can't race
  `builtins.getFlake (toString $ROOT)` source-capture during
  flake-eval gates (W2fu4 H8/H9).
- Rust worktrees share `/home/paydro/.cache/nixling-cargo-target/`
  through the repo-local `.cargo/config.toml` files.
- The integrator MUST run `nix-collect-garbage` after each wave merge.
- For the operator host running heavy iteration: prune OLD
  NixOS system generations periodically:

  ```
  sudo nix-collect-garbage --delete-older-than 7d
  ```

  Old `/nix/var/nix/profiles/system-N-link` symlinks are auto-gcroots;
  each pins ~1-2 GiB of unique closure. Without periodic pruning a
  host doing frequent rebuilds (today's W2fu4 baseline: 383
  generations from 10 days of work, pinning 471 GiB) silently fills
  its disk. The gate's default post-`nix store gc` only removes
  unreferenced paths, never old generations.
- `tests/static.sh` can run an opt-in deep GC after the gate:

  ```
  NL_POST_GATE_DEEP_GC=1 bash tests/static.sh           # user gens only
  NL_POST_GATE_DEEP_GC=1 \
  NL_POST_GATE_DEEP_GC_SUDO=1 \
  bash tests/static.sh                                  # + system gens
  ```

  `NL_POST_GATE_DEEP_GC_SUDO=1` uses `sudo -n` and skips fail-open
  with a clear log if passwordless sudo isn't available. Threshold
  defaults to 7 days; override with `NL_POST_GATE_DEEP_GC_DAYS=N`.
  Off by default — this is operator policy, not gate policy.
- `NL_SKIP_WITH_ENTRA_ID=1` skips the per-example flake check for
  `examples/with-entra-id` when its pinned `vicondoa/entrablau.nix`
  input fails the per-example cargo fetch with a transient crates.io
  403 against `libhimmelblau-0.8.18` / `kanidm-hsm-crypto-0.3.6`.
  `tests/static.sh` performs one in-band retry before failing the
  example; the skip knob is an explicit, panel-justifiable W3
  carve-out used only after the retry also fails. Added with the W3
  integration merge; re-evaluate once the entra-id input bumps past
  the affected revision.
- Before `git worktree remove`, confirm the worktree's
  `packages/target/` is the shared-cache symlink (or absent), not a
  real per-worktree directory.
- `tests/preflight-disk-space.sh` fails the wave when free disk under
  `$ROOT` drops below 10 GiB. Runs after the orphan reapers but BEFORE
  the rust toolchain bootstrap so the fail-closed guard cannot be
  bypassed by disk-consuming setup (W2fu4 H2).
- `nix flake check` now builds real `cargo-deny` + `cargo-audit`
  derivations (via `checks.${system}.rust-deny` / `.rust-audit`).
  Each derivation fetches the pinned RustSec advisory DB snapshot
  from the Nix store (no network at build time) and runs cargo-deny /
  cargo-audit against both `packages/Cargo.lock` and
  `packages/nixling-priv-broker/Cargo.lock`. The advisory DB is a
  `fetchFromGitHub` pinned to a specific commit; update the rev + hash
  in `flake.nix` periodically to pick up new advisories. Wall-clock
  impact: seconds per check (no compilation, just lockfile analysis).

## Critical subsystems — handle with care

Touch these only with a clear plan and a corresponding test run.

| System                              | Where                                                                                  | Risk if broken                                                            |
| ----------------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Net VM networking / firewall        | `nixos-modules/net.nix` (the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus the per-env MTU/MSS and east-west wiring) | Net VM dual-stacks DHCP on its uplink, breaks NAT, or weakens same-env isolation unexpectedly. Validate with `tests/net-vm-network-eval.sh`. |
| Per-VM `/nix/store` hardlink farm   | `nixos-modules/store.nix`, `/var/lib/nixling/vms/<vm>/store{,-meta}/`, `nixos-modules/processes-json.nix` (`virtiofsdRunner` ro-store `--shared-dir`), daemon `StoreSync` op + broker `store_view_farm` | The guest's `/nix/store` MUST be the per-VM closure-only farm `/var/lib/nixling/vms/<vm>/store`, never the host's full `/nix/store`: virtiofsd-ro-store's `--shared-dir` points at that farm (the `share.source == "/nix/store"` string stays as the eval-time sentinel — do not "simplify" it back to serving `/nix/store`, that re-leaks the whole host store to every guest). Requires `/var/lib/nixling` and `/nix/store` on the **same filesystem** — hardlinks can't cross FS boundaries; if split, `nixling vm switch` refuses with a fatal error. The broker builds the farm inside a private mount namespace where `/nix/store` is lazily detached (NixOS bind-mounts `/nix/store` on itself, so a same-`st_dev` cross-vfsmount `link(2)` returns `EXDEV` — recoverable, distinct from a fatal different-filesystem `EXDEV`); a `link(2)` `EMLINK` on a `--optimise`d store's saturated empty-file inode falls back to a byte copy. The daemon owns the sync; there is no per-VM `store-sync` unit. |
| TPM persistence (per-VM swtpm)      | `/var/lib/nixling/vms/<vm>/swtpm/`; spawned via broker `SpawnRunner` from `packages/nixling-host/src/swtpm_argv.rs` and supervised by `nixlingd` as a child of the VM's DAG | Holds the per-VM TPM 2.0 NVRAM + EK seed. **Wiping it looks like device tampering to any IdP** (Entra ID, Intune, Bitlocker-style policies) and forces re-enrollment. Never zero it casually. The state directory's ACLs are asserted by `tests/smoke-eval-tpm.nix`. |
| USBIP passthrough                   | `nixos-modules/components/usbip.nix` (eval-time gating) + broker `UsbipBindFirewallRule` + `SpawnRunner` (per-busid attach process supervised by `nixlingd`) | Eval-time gating still scopes attach to opted-in envs (validated by `tests/usbip-gating-eval.sh`). At runtime, attach/detach runs through the broker — there is no per-env `nixling-sys-<env>-usbipd-*` socket. Misrouted attaches expose a YubiKey to the wrong env. |
| GPU sidecar (graphics VMs)          | `nixos-modules/components/graphics.nix` + broker `SpawnRunner` for cloud-hypervisor on graphics VMs; pidfd handed back via `OpenPidfd` and supervised by `nixlingd` | Graphics VMs run cloud-hypervisor with the GPU device attached. Restarting `nixlingd` no longer terminates CH — pidfd handoff means the child outlives a daemon reconnect — but the broker spawn path is the only audited place CH is launched. Bypassing it breaks the audit trail. Validate with `tests/video-sidecar-hardening-eval.sh`. |
| Video sidecar (graphics VMs)        | `nixos-modules/components/video/guest.nix`, `nixos-modules/processes-json.nix`, `pkgs/vhost-user-video/`, `packages/nixling-host/src/video_argv.rs`, broker `SpawnRunner{role: Video}` | `graphics.videoSidecar = true` is an explicit opt-in H264 decode path: guest `virtio_media` + patched Cloud Hypervisor `--vhost-user-media` + patched crosvm `device video-decoder --backend vaapi`. There is no per-VM video systemd unit, no stock crosvm/CH fallback, and no free-form video extra args. The video runner MUST use the dedicated `nixling-<vm>-video` principal, not `nixling-<vm>-gpu`, so broker/activation ACLs can deny host Wayland/PipeWire/Pulse sockets to video without breaking GPU cross-domain. The broker masks `/dev` for the video runner and exposes only the declared device allowlist: default `/dev/dri/renderD128`, plus `/dev/nvidiactl`, `/dev/nvidia0`, and `/dev/nvidia-uvm` only when `graphics.videoNvidiaDecode = true`. `virtio_media` is a guest module, not a host `/proc/modules` preflight requirement. Firefox/VA-API uses the separate experimental `graphics.virglVideo` GPU path; it is default-off and must not be treated as stable video-sidecar coverage. Validate with `tests/video-contract-eval.sh`, `make test-rust` (pinned `video_argv` tests), and `tests/minijail-validator-video.sh`. |
| Manifest contract                   | `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix`               | Version-pinned via `manifestVersion`. Adding, removing, or renaming a per-VM field requires bumping the version, updating the schema, and noting it in the CHANGELOG. The `static.sh` md↔json drift gate catches partial updates. |
| Manifest bundle — private artifacts | `docs/reference/manifest-bundle.md` + `docs/reference/schemas/v2/*.json` + `packages/nixling-core/src/{bundle,host,processes,privileges,closures,minijail_profile}.rs` + `nixos-modules/{bundle,host-json,processes-json,privileges-json,closures-json,minijail-profiles}.nix` + `packages/xtask/src/main.rs` (`gen-schemas`) | Sensitive bundle artifacts install at `root:nixlingd` 0640 and ground every broker/sandbox/runner behaviour. `nixling-core` DTOs are canonical; committed schemas under `docs/reference/schemas/v2/` ARE the contract and the `tests/bundle-drift.sh` gate enforces `xtask gen-schemas` + `git diff --exit-code`. Breaking the schema without an intentional `bundleVersion`/`schemaVersion` bump silently breaks every downstream consumer. |
| Control plane — `nixlingd` + `nixling-priv-broker` | `packages/nixling-ipc/**` + `packages/nixling-core/**` + `packages/nixlingd/**` + `packages/nixling-priv-broker/**` (sibling workspace; `unsafe_code = "deny"` with quarantined `src/sys.rs` for fd-passing FFI) + `packages/nixling/**` + `docs/reference/{cli-contract,daemon-api,error-codes,privileges}.md` + the daemon Layer-1 gate set in `tests/static.sh` | The **only** persistent root surfaces the framework declares. `nixling-priv-broker.socket` is socket-activated: systemd creates/binds/listens/sets-ACL before the broker starts; the broker adopts fd 3 via `SD_LISTEN_FDS` and MUST NOT self-bind, self-fchmod, or self-fchown when `SD_LISTEN_FDS=1`. `nixlingd.service` carries `Wants=nixling-priv-broker.socket` (not `Requires=`) so the daemon keeps serving while the broker is idle. The broker reloads the current bundle resolver per accepted request so it does not dispatch stale runner intents after a switch. The broker drops to the `nixlingd` group and uses `SO_PEERCRED` at accept time for authz (launcher / admin / deny). Every host mutation flows through a typed broker op (cgroup v2 delegation, TAP/bridge lifecycle, `ApplyNftables`, `ApplyNmUnmanaged`, `ApplySysctl`, `UpdateHostsFile`, `ModprobeIfAllowed`, `UsbipBindFirewallRule`, `SpawnRunner`, `OpenPidfd`) and is recorded as an `OpAuditRecord` in `/var/lib/nixling/audit/broker-<utc-date>.jsonl` (root-owned `0640 root:nixlingd`, append-only `O_APPEND`, daily rotation, 14-day default retention overridable via `nixling.site.audit.retentionDays`). Relevant tests: `tests/broker-socket-activation-eval.sh`, `tests/broker-caps-eval.sh`, `tests/nixlingd-startup-smoke.sh`, `tests/legacy-unit-denylist-eval.sh`. See [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md). |
| Eval-time assertions                | `nixos-modules/assertions.nix`                                                          | These are the framework's contract with consumers. Loosening one silently turns a previously-rejected misconfig into runtime breakage. New assertions need a matching case in `tests/assertions-eval.sh`. |
| Guest-control exec session table    | `packages/nixlingd/src/{exec_session,exec_session_real}.rs`, `run_exec_owner` in `packages/nixlingd/src/lib.rs`, `packages/nixling/src/exec_client.rs`, `packages/nixling-ipc/src/public_wire.rs` (`ExecOp`/`ExecOpResponse`) | `nixling vm exec` (and the `vm konsole` wrapper) is **admin-only** and runs entirely in-process in `nixlingd`: an exec **session table** holds per-session workers that own one authenticated guest-control vsock client (W15 bridge), proxying typed exec ops to `guestd`. There is **no per-VM systemd unit, no new broker op, and no SSH** — the guest owns the PTY; the host only flips termios via an RAII raw-mode guard restored on every exit/error/panic. The admin `SO_PEERCRED` check runs BEFORE any session lookup/slot reservation/connect; old/non-guest-control generations fail closed (exit `70`) with no proxy and no SSH fallback. Session-table caps (global/per-UID/per-VM) are enforced before connect/auth; long-polls never hold the op-queue or sole client. Only one kind=critical session-establishment audit event is emitted (redacted: vm/peer_uid/tty); the opaque session handle, argv (hash-only), and stdio/env/cwd/paths never reach any Debug/trace/audit/metric surface. Detached reconnect is deferred — the owner handler rejects `detached=true`. Validate with the `exec_session`/`exec_client` hermetic test matrices. |
| Lifecycle permission group          | `nixos-modules/host-users.nix`                                                          | Membership in `nixling` + `SO_PEERCRED` at `public.sock` accept time is the **only** lifecycle authorisation surface. There is no polkit allowlist; wiring anything else into the group inverts the threat model. |
| SSH key generation / rotation       | `nixos-modules/host-keys.nix`, `host-activation.nix`                                    | The framework owns `${cfg.site.keysDir}/<vm>_ed25519`. `nixling keys rotate` MUST NOT touch consumer-supplied keys. |
| virtiofsd sandbox model             | `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles), `packages/nixling-priv-broker/src/sys.rs` (`clone3_spawn_runner` user-NS path), `nixos-modules/processes-json.nix` (argv emit) | virtiofsd profiles MUST declare zero host capabilities (`capabilities = []`), `requiresStartRoot = false`, and a `userNamespace` block mapping in-NS UID/GID 0 to the per-share principal. Normal VM shares map to `nixling-<vm>-runner`; the guest-control token share (`nl-gctl`) maps to the narrower `nixling-<vm>-gctlfs` principal. The broker pre-establishes the user namespace via `clone3(CLONE_NEWUSER)` + `pipe2` sync + `/proc/<pid>/uid_map` writes BEFORE virtiofsd's first instruction runs. virtiofsd argv MUST include `--sandbox=chroot --inode-file-handles=never` and `--readonly` for every `readOnly` share (`ro-store`, `nl-gctl`). Reintroducing host caps, `requiresStartRoot=true`, or `--sandbox=namespace` violates [ADR 0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md). Validate with `tests/minijail-validator-virtiofsd.sh` + `make test-rust` (pinned `virtiofsd_argv` tests). |

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
- **Don't reintroduce a per-VM systemd unit or a host-singleton
  framework service.** Every per-VM lifecycle step lives inside
  `nixlingd`'s DAG executor with privileged side effects routed
  through a typed `nixling-priv-broker` op (ADR 0015). The
  `tests/legacy-unit-denylist-eval.sh` and
  `tests/agents-md-rewrite-eval.sh` gates fail closed on
  regressions.
- **Don't reintroduce a bash CLI fallback or env-knob escape
  hatch.** The Rust CLI is the only operator surface;
  `NIXLING_LEGACY_BASH_OPT_IN`, `NIXLING_LEGACY_CLI`, and
  `NIXLING_NATIVE_ONLY` are no-ops.
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
  and feature-branch commits — never in shipped source comments,
  shipped docs prose, CLI help/error text, or released CHANGELOG
  sections. See [Versioning & changelog](#versioning--changelog).
  The functional `nixling.defaultSwitchReadiness.<wave>` option
  surface is the one deliberate exception.

## cgroup slice naming + ownership-marker conventions

The privileged broker's host-prepare dispatch (see the Control plane
row above) carries two operational conventions that ground every
broker op mutating host state.

### cgroup slice naming

- Single canonical slice: **`/sys/fs/cgroup/nixling.slice`** (no
  `system-` prefix, no `nixling-launcher.slice` parent). The broker
  creates it on `host prepare --apply` if absent.
- Per-VM directories live one level below the slice:
  `nixling.slice/<vm>/<role>/`. The VM layer is **process-free**; only
  the per-role leaves hold processes.
- Delegation: the broker `fchown`s the delegated subtree (the
  `nixling.slice` directory and every descendant) to the `nixlingd`
  system user. The host cgroup root is never chowned.
- Forbidden surfaces: writing `cpuset.cpus.partition` on
  nixling-owned cgroups (the cgroup v2 root and other ancestors
  are out of scope; nixling never reads/writes them), threaded
  cgroups, `cgroup.kill` on `nixling.slice` or any ancestor of
  a daemon-owned leaf, and **Phase B (post-delegation) runtime
  mutation while running as uid 0** (Phase A privileged setup
  — `+controllers` cascade, slice/leaf `mkdir`, `fchown` to
  `nixlingd`'s uid/gid — legitimately runs as root per ADR 0011
  Decision item 2; the uid != 0 invariant applies to the
  steady-state cgroup code path after privilege drop). See
  [`docs/reference/cgroup-delegation.md`](./docs/reference/cgroup-delegation.md)
  and ADR 0011 for the algorithm + audit shape.

### Ownership-marker conventions

The broker writes its host mutations inside greppable ownership
markers so foreign-rule preservation can be enforced fail-closed:

| Surface | Marker shape |
| --- | --- |
| nftables (`inet nixling` table) | every rule + chain carries `comment "nixling managed: <ownership-id>"`; foreign tables are never flushed |
| `/etc/hosts` | block delimited by `# nixling-managed begin` and `# nixling-managed end`; foreign lines outside the block are byte-preserved |
| NetworkManager unmanaged config | `/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf`, contents delimited by `# nixling-managed begin` / `# nixling-managed end` |
| systemd-networkd | detection-only; coexistence requires an operator-shipped configured-unmanaged file matching the `nl-`/`nlv-` prefix (no nixling write) |

Discovering a foreign ownership marker where nixling expects its own
is fail-closed (`path-safety-violation`,
`nm-managed-foreign-conflict`, `foreign-nft-rule-preserved`). See
[`docs/explanation/host-prepare.md`](./docs/explanation/host-prepare.md)
§ "NetworkManager / systemd-networkd coexistence" and ADR 0013 for
the rationale.

## Daemon-only end-state (P6 onward)

The framework declares **exactly three** root-visible units:
`nixlingd.service`, `nixling-priv-broker.socket`, and
`nixling-priv-broker.service`. The binding architectural decision
is recorded in
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md).

Agents working on the framework MUST treat the following as the
contract:

- The CLI is the Rust `nixling` binary, full stop. There is no bash
  fallback bridge; `NIXLING_LEGACY_BASH_OPT_IN`, `NIXLING_LEGACY_CLI`,
  and `NIXLING_NATIVE_ONLY` are no-ops.
- There are no framework-declared per-VM systemd units. The per-VM
  lifecycle DAG runs inside `nixlingd`; spawned runners
  (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound, USBIP
  attach) are launched by the broker's `SpawnRunner` op and handed
  back to `nixlingd` as pidfds via `OpenPidfd` / `SCM_RIGHTS`.
- There are no host-singleton framework services
  (`nixling-ch-exporter`, `nixling-otel-host-bridge`,
  `nixling-net-route-preflight`, `nixling-audit-check[.timer]`,
  `microvms.target`). Their work either moved into `nixlingd` or
  was retired with the metric / signal it produced.
- The `nixling.vms.<vm>.supervisor` option has been removed; setting
  it fails eval with a typed friendly message.
- The polkit allowlist for legacy launcher groups is retired.
  `nixling` group membership + `SO_PEERCRED` at
  `public.sock` accept time is the **only** lifecycle authorisation
  surface.
- The Rust CLI does not invoke bash.
  `packages/nixling-contract-tests/tests/policy_source.rs`
  (`no_bash_exec_check`) plus the AST-level `tests/tools/no-bash-ast-walker`
  step in `tests/rust-workspace-checks.sh` are the fail-closed gates
  ([ADR 0017](./docs/adr/0017-no-bash-fallbacks-invariant.md)).

### Verification gates

- `tests/legacy-unit-denylist-eval.sh` asserts that no example's
  `nixos-rebuild dry-build` output emits a retired unit name.
- `packages/nixling-contract-tests/tests/policy_lints.rs`
  (`adr_0015_present_with_header_and_cross_references`) asserts the ADR
  exists, carries the canonical header, and is cross-referenced from this
  file.
- `tests/agents-md-rewrite-eval.sh` asserts AGENTS.md itself does
  not mention the bash CLI or per-VM systemd templates as live
  surfaces (only as historical / retired context).
- Host exit criterion: on a deployed host,
  `systemctl list-units --no-pager --all | grep -E '^(nixling|microvm)' | wc -l`
  returns `3`.

## References

- [docs/adr/0015-daemon-only-clean-break.md](./docs/adr/0015-daemon-only-clean-break.md)
  — **the binding architectural decision** for the daemon-only
  end-state: `nixlingd` + `nixling-priv-broker` are the only
  persistent root surfaces.
- [docs/adr/0017-no-bash-fallbacks-invariant.md](./docs/adr/0017-no-bash-fallbacks-invariant.md)
  — the Rust CLI never invokes bash; CI gates enforce no new
  `Command::new("bash")` sites.
- [docs/adr/0018-microvm-nix-removal.md](./docs/adr/0018-microvm-nix-removal.md)
  — nixling owns its per-VM substrate via `vm-options.nix` +
  `vm-evaluator.nix`; the `microvm.nix` flake input is gone.
- [docs/adr/0021-broker-user-namespace-for-virtiofsd.md](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md)
  — broker pre-establishes a single-entry user namespace via
  `clone3(CLONE_NEWUSER)` so virtiofsd runs fake-root inside the
  NS while exposing **zero** host capabilities. Any change to the
  virtiofsd minijail profile or argv shape MUST preserve this
  contract.
- [README.md](./README.md) — consumer-facing intro, install,
  manual integration walkthrough.
- [CHANGELOG.md](./CHANGELOG.md) — Keep-a-Changelog, entries
  accumulate under `## Unreleased` until a tag cuts them.
- [SECURITY.md](./SECURITY.md) — disclosure path + scope.
- [docs/explanation/design.md](./docs/explanation/design.md) —
  threat model, defenses-in-depth list, *Why not X* FAQ.
- [docs/explanation/daemon-lifecycle.md](./docs/explanation/daemon-lifecycle.md)
  — daemon DAG executor, pidfd handoff, supervisor reconciliation.
- [docs/reference/privileges.md](./docs/reference/privileges.md) —
  authoritative broker op catalogue.
- [docs/reference/daemon-api.md](./docs/reference/daemon-api.md) —
  `public.sock` wire surface, audit format, retention.
- [docs/reference/manifest-schema.md](./docs/reference/manifest-schema.md)
  + [docs/reference/manifest-schema.json](./docs/reference/manifest-schema.json)
  — the manifest contract.
- [docs/reference/cli-contract.md](./docs/reference/cli-contract.md) —
  CLI lifecycle FSM, signal semantics, exit codes, JSON vs human
  output.
- [docs/how-to/migrate-nixling-v0-to-v1.md](./docs/how-to/migrate-nixling-v0-to-v1.md)
  — consumer migration guide for v0.x → v1.0.
- [docs/how-to/migrate-nixling-v1-0-to-v1-1.md](./docs/how-to/migrate-nixling-v1-0-to-v1-1.md)
  — consumer migration guide for v1.0 → v1.1.
- [docs/how-to/migrate-nixling-v1-1-to-v1-2.md](./docs/how-to/migrate-nixling-v1-1-to-v1-2.md)
  — consumer migration guide for v1.1 → v1.2, including the
  canonical `nixling` lifecycle group rename.
- [docs/how-to/migrating-from-microvm.md](./docs/how-to/migrating-from-microvm.md)
  — option mapping for users coming from raw microvm.nix
  (scoped to new installs).
- [tests/README.md](./tests/README.md) — full test layering,
  including Layer-2 integration tests.
- [LICENSE](./LICENSE) — Apache-2.0.
