# AGENTS.md

Operating manual for AI coding agents (Copilot CLI, GitHub Copilot,
Cursor, …) and human contributors working on **`vicondoa/nixling`
itself**. If you are *consuming* nixling in your own NixOS host
config, start at [README.md](./README.md) instead — this file is for
people changing the framework.

## What this is

Nixling is an opinionated NixOS desktop microVM framework built on
top of [microvm.nix]. **From v1.0 the control plane is daemon-only:
`nixlingd` supervises every per-VM DAG and `nixling-priv-broker`
dispatches every audited host mutation.** Per-VM systemd templates,
host-singleton framework services, and the bash CLI are removed
wholesale at the v0.4.x → v1.0.0 boundary (see
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md)).

What the framework still provides: per-env isolated networks with an
auto-declared NAT/DHCP "net VM", a per-VM `/nix/store` hardlink farm,
toggleable per-VM components (graphics, TPM, USBIP, audio), and the
versioned bundle/manifest contract that grounds the broker dispatcher.
See [README.md](./README.md) and
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

Four tiers cover the static gate. Pick the one that matches your
intent.

```bash
# 1. Flake-level eval, both systems we support.
nix flake check --no-build --all-systems

# 2. Tier-0 fast static gate. Shell syntax + shellcheck on the
#    repo's bash entrypoints in under a minute. Run it before a
#    broader review loop and after doc/shell-only rebases.
bash tests/static-fast-tier0.sh

# 3. Fast PR-loop gate (W3a-3). Catches parse / shellcheck / flake
#    check / rust workspace / W1 bundle invariants / W3 host-prepare
#    canaries / cross-cutting drift in ~13 min cold (~2 min warm),
#    ~520 G peak /nix/store. Run before every commit and after every
#    rebase. Does NOT exercise the eval gates, mid-tier consumer-config
#    evals, manifest contract, W2 broker daemons, per-example
#    flake-check, or audio component — those land in tier (4) below.
bash tests/static-fast.sh

# 4. Full panel/wave-exit gate (the canonical Layer 1 set). Adds
#    smoke-eval, assertions-eval, observability-eval, mid-tier evals,
#    manifest contract, W2 control-plane gates, per-example flake-
#    check, cli-contract-coverage, cli-json-drift. ~30-90 min cold,
#    peak disk capped at ~400 G via per-phase nix store gc (W3a-4).
#    Set NL_GATE_DISK_BUDGET_GIB=300 to fail-closed at the phase
#    boundary if free disk drops below the budget.
bash tests/static.sh

# 5. Optional focused checks (called transitively by static.sh, also
#    useful standalone while iterating):
bash tests/assertions-eval.sh        # consolidated batch eval +
                                     # fallback for the 3 throw cases
                                     # (W3a-1): ~13 min cold (was 32)
nix-instantiate --eval --strict \
  -E 'let f = import ./tests/smoke-eval-tpm.nix; r = f {}; in r.drvPath' \
  >/dev/null
bash tests/net-vm-network-eval.sh    # net VM networkd config invariants
bash tests/usbip-gating-eval.sh      # host-side USBIP gating + env scoping
bash tests/cli-json.sh               # daemon CLI JSON envelope contract (P6-rewritten; daemon-only)
bash tests/legacy-unit-denylist-eval.sh  # P6 fail-closed gate (ADR 0015)
bash tests/agents-md-rewrite-eval.sh # P6 docs invariant (this file's end-state)
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

From v1.0 the framework declares **exactly three** root-visible
units. There is no `nixling@<vm>`-style per-VM unit; `nixlingd`
supervises every per-VM DAG in-process and hands fds to spawned
runners via the broker's `SpawnRunner` / `OpenPidfd` ops.

| Resource                                | Pattern                                |
| --------------------------------------- | -------------------------------------- |
| Public daemon (supervisor)              | `nixlingd.service`                     |
| Privileged broker socket                | `nixling-priv-broker.socket`           |
| Privileged broker service               | `nixling-priv-broker.service`          |
| Lifecycle permission group              | `nixling-launchers` (singleton)        |

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

### VM lifecycle (daemon-supervised)

`nixlingd` is the sole supervisor for every per-VM lifecycle DAG.
There are no framework-declared per-VM systemd units: child
processes (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound,
USBIP attach) are spawned by the broker via `SpawnRunner`, handed
back to `nixlingd` over `SCM_RIGHTS` as pidfds, and reconciled
against the persisted DAG state under
`/var/lib/nixling/supervisor/state.json`.

The `restartIfChanged = false` invariant from v0.1.5 is no longer
needed on per-VM units (none are emitted). It still applies to the
two daemon units themselves:

- `nixlingd.service` carries `restartIfChanged = false`; rebuilds
  do not cycle a running daemon. Operators apply pending daemon
  changes explicitly via `nixling host doctor` (reports the
  pending-restart state) and `systemctl restart nixlingd` once the
  supervisor reconciliation window is acceptable.
- `nixling-priv-broker.service` is socket-activated and idle-exits;
  it picks up new code on the next dispatch and never holds
  in-flight session state across requests.

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

Canonical precedent: the v0.2.0 observability Wave-1 panel returned
0/8 sign-offs with 11 HIGH findings. `tests/static.sh` caught none of
them. This is the canonical "you can't test your way out of needing a
panel" data point.

### Default observability panel

| Engineer          | Focus |
|-------------------|-------|
| `software`        | Shell + Nix shape of every new module, `cli.nix` instrumentation, idempotency of socat sidecars, error handling in `nixling-ch-exporter`. |
| `test`            | Coverage of new option schema, vsock CID collision cases, restart-policy gate extension, manifest schema drift, what could regress invisibly. |
| `nixos`           | Module wiring, `lib.mkForce` / `lib.mkDefault` correctness, option declarations, systemd unit composition, `restartIfChanged = false` invariant on every new sidecar. |
| `networking`      | Reviews that vsock genuinely removes IP from the data path, sanity-checks that `network.nix` is **unchanged**, audits firewall posture across all envs, confirms no DHCP/DNS regression in the obs env. |
| `security`        | Vsock attack surface vs the existing virtio devices, host-relay trust posture, capability sets / syscall filters on every new unit, telemetry-label PII review, retention defaults. |
| `product`         | Default-off opt-in shape, naming surface across `nixling.observability.*`, operator UX (Grafana URL discovery, lite-mode story), deprecation policy for the eventual `socat → nixling-otel-relay` Rust binary swap. |
| `docs`            | Diataxis adherence in `docs/{reference,how-to,explanation}/`, CHANGELOG entries, manifest-schema md↔json drift, AGENTS.md updates landing in the same commit as the load-bearing changes they describe. |
| `observability`   | Cardinality of metric labels, span attribute hygiene (no secrets/cmd output/store paths), Loki label-set sizing, Tempo retention vs disk budget, Grafana datasource provisioning correctness. |

This is the default composition for the in-progress v0.2.0
observability track.

### Historical security-hardening panel

The original panel for the v0.1.x security-hardening work had six
engineers: `nixos`, `rust`, `software`, `test`, `networking`, and
`security`. That context helps when older commits or
[CHANGELOG.md](CHANGELOG.md) entries refer to a panel sweep.

The currently canonical roster files live in the host-local
`/etc/nixos/scripts/panel-engineers-security.txt` and
`/etc/nixos/scripts/panel-engineers-observability.txt`.

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
| `tests/static-fast.sh`                | **Fast PR-loop gate.** Parse / `shellcheck` / `flake check` / rust workspace / bundle invariants / host-prepare canaries / cross-cutting drift without the full wave-exit panel set. |
| `tests/static.sh`                     | **Top-level Layer-1 gate.** Parse, `flake check`, smoke evals, assertion/observability/USBIP/autostart/restart-policy eval gates, manifest contract, per-example flake checks. Runs from repo root; override with `ROOT=<path>`. |
| `tests/smoke-eval.nix`                | Workload smoke: minimal consumer-style nixosSystem, builder's native system.                 |
| `tests/smoke-eval-graphics.nix`       | Same shape, with `graphics.enable = true`. x86_64-only.                                      |
| `tests/smoke-eval-tpm.nix`            | TPM host-surface regression gate: swtpm parent-dir ACLs, swtpm flush helper, runner socket ACLs, and ownership-migration invariants. |
| `tests/smoke-eval-aarch64.nix`        | Headless smoke cross-evaluated on aarch64-linux (multi-arch eval-graph regression gate).     |
| `tests/assertions-eval.sh`            | 10 negative cases: CIDR overlap, platform gate, missing `waylandUser`, etc. Each must fail eval with the expected message. |
| `tests/usbip-gating-eval.sh`          | Host-side USBIP gating: absent until both host + enabled-VM opt-ins are set, and scoped to opted-in envs only. |
| `tests/assertions-eval.sh`            | 10 negative cases: CIDR overlap, platform gate, missing `waylandUser`, etc. Each must fail eval with the expected message. |
| `tests/assertions-eval.sh`            | Negative assertion cases: CIDR overlap, naming invariants, platform gate, missing `waylandUser`, etc. Each must fail eval with the expected message. |
| `tests/net-vm-network-eval.sh`        | Net VM networkd-config invariants — most importantly the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`. |
| `tests/video-sidecar-hardening-eval.sh` | Eval-time hardening gate for the broker `SpawnRunner` video runner descriptor (`AF_UNIX` only, syscall filter, empty capability sets). |
| `tests/bridge-isolation-runtime.sh`   | Hermetic runtime bridge-isolation test: net-VM port stays reachable, workload taps stay isolated even after peer-style MAC spoofing. |
| `tests/assertions-eval.sh`            | 10 negative cases: CIDR overlap, platform gate, missing `waylandUser`, etc. Each must fail eval with the expected message. |
| `tests/net-vm-network-eval.sh`        | Net VM networkd-config invariants — most importantly the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`. |
| `tests/assertions-eval.sh`            | Negative assertion cases plus reserved-path invariants (`stateDir`, `store.stateDir`) and the boot-cleaned `tmpDir` rule. Each bad case must fail eval with the expected message. |
| `tests/net-vm-network-eval.sh`        | Net VM networkd + nftables invariants — most importantly the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus per-env MTU/MSS, cross-env drops, and east-west toggles. |
| `tests/legacy-unit-denylist-eval.sh`  | P6 fail-closed gate: no example's `nixos-rebuild dry-build` output emits a retired per-VM systemd template or host-singleton framework service (ADR 0015). |
| `tests/adr-0015-presence-eval.sh`     | Asserts the daemon-only ADR exists, carries the canonical header, and is cross-referenced from `AGENTS.md`. |
| `tests/agents-md-rewrite-eval.sh`     | Asserts `AGENTS.md` does not describe the bash CLI or per-VM systemd templates as live framework surfaces (P6 docs invariant). |
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
because it depends on the sibling `nixos-entra-id` input, which the
core flake does not (and should not) pull in. Its own flake is
still eval-checked by `tests/static.sh` during the per-example
iteration step, and CI also runs
`.github/workflows/eval-with-entra-id.yml` to execute
`nix flake check --no-build --all-systems --no-write-lock-file`
inside the example directory without coupling the root flake to the
sibling input.

## Commit conventions

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
  `examples/with-entra-id` when its pinned `vicondoa/nixos-entra-id`
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
| Per-VM `/nix/store` hardlink farm   | `nixos-modules/store.nix`, `/var/lib/nixling/vms/<vm>/store{,-meta}/`, daemon `StoreSync` op | Requires `/var/lib/nixling` and `/nix/store` on the **same filesystem** — hardlinks can't cross FS boundaries. If they end up split, `nixling vm switch` refuses with a fatal error. The daemon owns the sync; there is no per-VM `store-sync` unit. |
| TPM persistence (per-VM swtpm)      | `/var/lib/nixling/vms/<vm>/swtpm/`; spawned via broker `SpawnRunner` from `packages/nixling-host/src/swtpm_argv.rs` and supervised by `nixlingd` as a child of the VM's DAG | Holds the per-VM TPM 2.0 NVRAM + EK seed. **Wiping it looks like device tampering to any IdP** (Entra ID, Intune, Bitlocker-style policies) and forces re-enrollment. Never zero it casually. The state directory's ACLs are asserted by `tests/smoke-eval-tpm.nix`. |
| USBIP passthrough                   | `nixos-modules/components/usbip.nix` (eval-time gating) + broker `UsbipBindFirewallRule` + `SpawnRunner` (per-busid attach process supervised by `nixlingd`) | Eval-time gating still scopes attach to opted-in envs (validated by `tests/usbip-gating-eval.sh`). At runtime, attach/detach runs through the broker — there is no per-env `nixling-sys-<env>-usbipd-*` socket. Misrouted attaches expose a YubiKey to the wrong env. |
| GPU sidecar (graphics VMs)          | `nixos-modules/components/graphics.nix` + broker `SpawnRunner` for cloud-hypervisor on graphics VMs; pidfd handed back via `OpenPidfd` and supervised by `nixlingd` | Graphics VMs run cloud-hypervisor with the GPU device attached. Restarting `nixlingd` no longer terminates CH — pidfd handoff means the child outlives a daemon reconnect — but the broker spawn path is the only audited place CH is launched. Bypassing it breaks the audit trail. Validate with `tests/video-sidecar-hardening-eval.sh`. |
| Manifest contract                   | `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix`               | Version-pinned (`manifestVersion`; bumped to 3 in P2 for the daemon-only end-state). Adding, removing, or renaming a per-VM field requires bumping the version, updating the schema, and noting it in the CHANGELOG. The `static.sh` md↔json drift gate catches partial updates. |
| Manifest bundle — private artifacts | `docs/reference/manifest-bundle.md` + `docs/reference/schemas/v2/*.json` + `packages/nixling-core/src/{bundle,host,processes,privileges,closures,minijail_profile}.rs` + `nixos-modules/{bundle,host-json,processes-json,privileges-json,closures-json,minijail-profiles}.nix` + `packages/xtask/src/main.rs` (`gen-schemas`) | Sensitive bundle artifacts install at `root:nixlingd` 0640 and ground every broker/sandbox/runner behaviour. `nixling-core` DTOs are canonical; committed schemas under `docs/reference/schemas/v2/` ARE the contract and the `tests/bundle-drift.sh` gate enforces `xtask gen-schemas` + `git diff --exit-code`. Breaking the schema without an intentional `bundleVersion`/`schemaVersion` bump silently breaks every downstream consumer. |
| Control plane — `nixlingd` + `nixling-priv-broker` | `packages/nixling-ipc/**` + `packages/nixling-core/**` + `packages/nixlingd/**` + `packages/nixling-priv-broker/**` (sibling workspace; `unsafe_code = "deny"` with quarantined `src/sys.rs` for fd-passing FFI) + `packages/nixling/**` + `docs/reference/{cli-contract,daemon-api,error-codes,privileges}.md` + the daemon Layer-1 gate set in `tests/static.sh` | The **only** persistent root surfaces the framework declares. `nixling-priv-broker.socket` is socket-activated: systemd creates/binds/listens/sets-ACL before the broker starts; the broker adopts fd 3 via `SD_LISTEN_FDS` and MUST NOT self-bind, self-fchmod, or self-fchown when `SD_LISTEN_FDS=1`. `nixlingd.service` carries `Wants=nixling-priv-broker.socket` (not `Requires=`) so the daemon keeps serving while the broker is idle. The broker drops to the `nixlingd` group and uses `SO_PEERCRED` at accept time for authz (launcher / admin / deny). Every host mutation flows through a typed broker op (cgroup v2 delegation, TAP/bridge lifecycle, `ApplyNftables`, `ApplyNmUnmanaged`, `ApplySysctl`, `UpdateHostsFile`, `ModprobeIfAllowed`, `UsbipBindFirewallRule`, `SpawnRunner`, `OpenPidfd`) and is recorded as an `OpAuditRecord` in `/var/lib/nixling/audit/broker-<utc-date>.jsonl` (root-owned `0640 root:nixlingd`, append-only `O_APPEND`, daily rotation, 14-day default retention overridable via `nixling.site.audit.retentionDays`). Relevant tests: `tests/broker-socket-activation-eval.sh`, `tests/broker-caps-eval.sh`, `tests/nixlingd-startup-smoke.sh`, `tests/legacy-unit-denylist-eval.sh`. See [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md). |
| Eval-time assertions                | `nixos-modules/assertions.nix`                                                          | These are the framework's contract with consumers. Loosening one silently turns a previously-rejected misconfig into runtime breakage. New assertions need a matching case in `tests/assertions-eval.sh`. The planned `ph6-p6-supervisor-removed-assertion` (eval-time rejection of the retired `nixling.vms.<vm>.supervisor` option) was deferred to v1.1 backlog; v1.0 retains the option for backward-compat with consumer flakes pinning pre-v1.0 manifests (see ADR 0015 § Decision). |
| Lifecycle permission group          | `nixos-modules/host-users.nix`                                                          | Membership in `nixling-launchers` + `SO_PEERCRED` at `public.sock` accept time is the **only** lifecycle authorisation surface. The polkit allowlist that used to grant per-VM start/stop is retired (ADR 0015); wiring anything else into the group inverts the threat model. |
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
- **Don't reintroduce a per-VM systemd unit or a host-singleton
  framework service.** Every per-VM lifecycle step lives inside
  `nixlingd`'s DAG executor with privileged side effects routed
  through a typed `nixling-priv-broker` op (ADR 0015). The
  `tests/legacy-unit-denylist-eval.sh` and
  `tests/agents-md-rewrite-eval.sh` gates fail closed on
  regressions.
- **Don't reintroduce a bash CLI fallback or env-knob escape
  hatch.** `NIXLING_LEGACY_BASH_OPT_IN` and `NIXLING_LEGACY_CLI`
  were retired in P6; `NIXLING_NATIVE_ONLY` is a no-op only
  because P4 cli-up documented it.
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

## W3 cgroup slice naming + ownership-marker conventions

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
- Forbidden surfaces: writing `cpuset.cpus.partition` anywhere in the
  subtree, threaded cgroups, `cgroup.kill` on `nixling.slice` or any
  ancestor of a daemon-owned leaf, and delegation while the broker is
  uid 0. See [`docs/reference/cgroup-delegation.md`](./docs/reference/cgroup-delegation.md)
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

From v1.0.0, the framework declares **exactly three** root-visible
units: `nixlingd.service`, `nixling-priv-broker.socket`, and
`nixling-priv-broker.service`. The binding architectural decision
is recorded in
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md); this section
is the operating manual's pointer to the end-state and the cleanup
commits that produced it.

### What this section is for

Agents working on the framework after P6 land **must** treat the
following as the contract:

- The CLI is the Rust `nixling` binary, full stop. There is no bash
  fallback bridge, no `NIXLING_LEGACY_BASH_OPT_IN`, and no
  `NIXLING_LEGACY_CLI`. `NIXLING_NATIVE_ONLY` remains as a no-op
  documented in P4 cli-up.
- There are no framework-declared per-VM systemd units. The
  per-VM lifecycle DAG runs inside `nixlingd`; spawned runners
  (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound, USBIP
  attach) are launched by the broker's `SpawnRunner` op and handed
  back to `nixlingd` as pidfds via `OpenPidfd` / `SCM_RIGHTS`.
- There are no host-singleton framework services
  (`nixling-ch-exporter`, `nixling-otel-host-bridge`,
  `nixling-net-route-preflight`, `nixling-audit-check[.timer]`,
  `microvms.target`). Their work either moved into `nixlingd` or
  was retired with the metric / signal it produced.
- The `nixling.vms.<vm>.supervisor` option is retained in v1.0 source
  for backward-compat with consumer flakes pinning pre-v1.0 manifests
  (the framework's Tier 0 detection still branches on it); the
  v1.0-intended hard removal + eval-time rejection assertion is
  deferred to v1.1 backlog. See ADR 0015 § Decision for details.
  Setting `supervisor = "nixlingd"` requires
  `nixling.daemonExperimental.enable = true`.
- The polkit allowlist for `nixling-launcher` is retired.
  `nixling-launchers` group membership + `SO_PEERCRED` at
  `public.sock` accept time is the **only** lifecycle authorisation
  surface.
- There is no `v0 → v1` manifest auto-rewriter. Consumers follow
  [`docs/how-to/migrate-nixling-v0-to-v1.md`](./docs/how-to/migrate-nixling-v0-to-v1.md)
  (landed in P7) to regenerate their `configuration.nix`.

### Cleanup commits

The P6 cleanup landed across a sequence of focused commits whose
canonical tags are `( P6 )` and `( P6 <slice> )`:

- `ph6-p6-cli-nix-migrations` — relocates every `cli.nix` consumer
  (host-audit, options surface, store activation, desktop wrappers,
  audio state helper, observability store-sync references) ahead of
  the deletion sweep.
- `ph6-remove-systemd-emission` — deletes `host-wrapper.nix`,
  `host-sidecars.nix`, `components/audio/host.nix`,
  `components/video/host.nix`, `host-ch-exporter.nix`,
  `host-otel-relay-acl.nix`, `cli.nix`, and the bash `scripts/`
  entrypoints. The AGENTS.md rewrite (this file) is folded into the
  same commit by the integrator.
- `ph6-p6-supervisor-removed-assertion` — was planned to add the
  eval-time rejection of `nixling.vms.<vm>.supervisor`; deferred
  to v1.1 backlog (per ADR 0015 § Decision). The option remains
  in v1.0 source for backward-compat with consumer flakes pinning
  pre-v1.0 manifests; the v1.0-intended hard removal will land in
  v1.1.
- `ph6-p6-polkit-retire` — retires the `nixling-launcher` polkit
  rules; the group itself remains declared for permission-boundary
  continuity.
- `ph6-p6-unit-denylist-gate` — adds
  [`tests/legacy-unit-denylist-eval.sh`](./tests/legacy-unit-denylist-eval.sh),
  the fail-closed regression gate enumerating every retired unit
  pattern.
- `ph6-p6-default-switch-doc`,
  `ph6-p6-doc-blast-radius`,
  `ph6-p6-privileges-doc-final`,
  `ph6-p6-adr-0015` — rewrite the doc tree (Diataxis quadrants) for
  the daemon-only end-state.

### Verification

- `tests/legacy-unit-denylist-eval.sh` asserts that no example's
  `nixos-rebuild dry-build` output emits a retired unit name.
- `tests/adr-0015-presence-eval.sh` asserts the ADR exists,
  carries the canonical header, and is cross-referenced from this
  file.
- `tests/agents-md-rewrite-eval.sh` asserts AGENTS.md itself does
  not mention the bash CLI or per-VM systemd templates as live
  surfaces (only as historical / retired context).
- P6 host exit criterion: on a v1.0 test host,
  `systemctl list-units --no-pager --all | grep -E '^(nixling|microvm)' | wc -l`
  returns `3`.

## References

- [docs/adr/0015-daemon-only-clean-break.md](./docs/adr/0015-daemon-only-clean-break.md)
  — **the binding architectural decision** for the v1.0
  daemon-only end-state: `nixlingd` + `nixling-priv-broker` are the
  only persistent root surfaces; per-VM systemd templates, host
  singletons, and the bash CLI are deleted in P6 with no
  deprecation window. Supersedes the migration-mode plumbing in
  [ADR 0007](./docs/adr/0007-bash-coexistence-and-migration.md).
  Every section of this AGENTS.md rewrite resolves to ADR 0015 as
  the source of truth.
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
  authoritative broker op catalogue (daemon-only end-state).
- [docs/reference/daemon-api.md](./docs/reference/daemon-api.md) —
  `public.sock` wire surface, audit format, retention.
- [docs/reference/manifest-schema.md](./docs/reference/manifest-schema.md)
  + [docs/reference/manifest-schema.json](./docs/reference/manifest-schema.json)
  — the manifest contract (v3 from P2 onward).
- [docs/reference/cli-contract.md](./docs/reference/cli-contract.md) —
  CLI lifecycle FSM, signal semantics, exit codes, JSON vs human
  output.
- [docs/how-to/migrate-nixling-v0-to-v1.md](./docs/how-to/migrate-nixling-v0-to-v1.md)
  — consumer migration guide for v0.x → v1.0 (landed in P7).
- [docs/how-to/migrating-from-microvm.md](./docs/how-to/migrating-from-microvm.md)
  — option mapping for users coming from raw microvm.nix
  (scoped to new installs).
- [tests/README.md](./tests/README.md) — full test layering,
  including Layer-2 integration tests.
- [LICENSE](./LICENSE) — Apache-2.0.
