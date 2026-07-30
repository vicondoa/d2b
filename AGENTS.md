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

## Build & validate

Use the top-level `Makefile` targets. The shell scripts under `tests/`
are implementation details unless a target or `tests/AGENTS.md` tells
you to run one directly.

```bash
# Focused Layer-1 jobs, in tests/layer1-jobs.json local phase order.
# Read each job's current enforcement classification from that manifest.
make check-tier0
make check-inventory
make test-lint
make test-changelog
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-runtime-ledger
make test-performance-budgets
make test-fixture-contracts

# Post-preflight Layer-1 development umbrella. This runs the manifest jobs
# outside its preflight phase; `make check` also runs the preflight jobs.
make test-unit

# PR-equivalent Layer-1 gate. Uses tests/layer1-jobs.json to run
# the current enforcing and advisory jobs with bounded parallelism.
make check

# Legacy/full-static monolithic gate retained for explicit use.
make check-static

# Local Layer 1 + container integration. Still run the explicit
# host/manual pre-PR targets below before opening an agent-owned PR.
make test
```

`tests/layer1-jobs.json` is authoritative for both the job list and its
classification. A job is enforcing unless it carries `"enforcement":
"advisory"`; an advisory entry pairs that field with `advisoryReason` explaining
why its successful result is not enforcing evidence. Advisory means the
command is still launched and a nonzero result still fails the run, but a
guarded skip is permitted. Therefore an advisory result must not be cited as
validation evidence for a change.

The manifest currently classifies `check-tier0`, `check-inventory`,
`test-lint`, `test-changelog`, `test-rust`, `test-proofs`, `test-flake`,
`test-nix-unit`, `test-policy`, `test-drift`, `test-runtime-ledger`, and
`test-fixture-contracts` as enforcing. It classifies
`test-performance-budgets` as advisory. Always re-read the manifest rather than
assuming this split is fixed.

The performance canary prints `SKIP` and enforces no latency budget unless
`D2B_PERF_STABLE=1`. Promoting it requires a pinned self-hosted runner, setting
that variable on the runner, and then removing the advisory classification and
reason from the manifest. The project does not currently have such a runner.

The fixture-contract lane runs the fixture-dependent `d2b-contract-tests`
crate and the CLI-contract cases against a built `D2B_FIXTURES` bundle. Both
the local and continuous-integration lanes set `D2B_ENABLE_FIXTURE_BUILD=1`, so
it executes and enforces; invoking it without that variable is a hard failure
rather than a silent skip. It acquires the heavy-gate semaphore before doing
Nix or Cargo work, and `packages/xtask/src/heavy_gate.rs` fails closed if that
guard is ever removed. `test-rust` explicitly excludes the fixture-dependent
`d2b-contract-tests` crate, so a green `test-rust` does not validate that
fixture-dependent contract and policy layer. Selected hermetic policy files
may still have separate enforcing entrypoints such as `test-policy`; inspect
the target driver before claiming coverage.

Before opening an agent-owned PR, run the host/manual integration
targets on the development host; do not rely on the PR pipeline for
them:

```bash
make test-integration       # Layer 2 container tests; needs podman
make test-host-integration  # runNixOSTest VM checks; NixOS + KVM host
```

`make test-host-integration` is x86_64-linux only and may fall back to
slow TCG if `/dev/kvm` is absent. Hardware and live-host tests remain
explicit manual tiers and require a host with the matching devices or
deployed d2b state.

`make test-runtime-ledger` is the hermetic execution-budget Layer-1 job
(also run by `make test-unit` / `make check` through
`tests/layer1-jobs.json`). After a warm build (so compilation is excluded
from measurement), it records per-test wall-clock p95s as advisory
diagnostics and enforces an aggregate process-CPU p95 budget for each pinned
crate. Process CPU excludes time descheduled behind unrelated machine load,
which is why it is the enforced timing basis. The closed census in
`tests/runtime-ledger-census.json` presently pins one crate and exactly 190
tests; a vanished or extra test, an incomplete or under-repeated run, or an
aggregate crate CPU p95 over budget fails the gate. A per-test diagnostic
threshold breach does not.

The gate holds no baseline and makes no historical-regression claim. When you
legitimately add, remove or rename a census test, regenerate the pin with
`make runtime-ledger-pin` and commit the result; the pin is a closed set, so
the gate fails until it matches. The `test-runtime-ledger check` output is
authoritative for the exact advisory-report formatting and selection.
Growing the census to a real multi-crate shard inventory (with a per-shard
budget) and adding a cross-machine reference baseline for a true
historical-regression gate is the named deferred follow-up
`runtime-ledger-full-census-and-real-shards`. If its shape here diverges from
the current `Makefile` target or `tests/layer1-jobs.json`, treat those as
authoritative and flag the drift for the integrator.

### Heavy lanes

Every Layer-2, host-integration, hardware, live, and perf-heavy command
runs through **one** semaphore, invoked from the repository root as `cargo
run --manifest-path packages/Cargo.toml -p xtask -- heavy-gate`. It grants
two slots per uid via open file description locks so concurrent heavy lanes
cannot oversubscribe the shared Nix store, cargo target directory, or KVM
device. Do not add a second lock file, sleep-and-retry loop, or per-crate
guard.

The slot namespace is fixed at `/run/d2b-heavy-gates/uid-<uid>/`. The root
and per-uid directory are root-owned and non-writable by unprivileged users;
the two `slot-*` files are pre-created for the target uid at mode `0600`.
There is no runtime-directory or temporary-directory fallback. The NixOS
module provisions the root with systemd-tmpfiles, then activation provisions
directories and slots for configured lifecycle users that NSS can resolve.
An unavailable network-backed user is deferred rather than failing
activation; after that user logs in, run `make heavy-gate-provision`. Use
the same target on a host that does not consume the module. Because `/run`
is a tmpfs, run it once per boot when the gate requests it. An absent or
malformed namespace is an environment error with that provisioning
remediation, never permission to create a weaker pool. In particular,
`/run/user/<uid>` is rejected because its owner can rename slot names or
their parent and create an independent pool.

The structure is public-lane-plus-guarded-internal:

- **Public lane targets** (`make test-integration`,
  `make test-host-integration`, `make test-hardware`, `make perf`) acquire
  a slot and then delegate to a guarded internal `heavy-lane-*` target.
  Run these.
- **Internal `heavy-lane-*` targets** hold the raw work and fail closed
  through `heavy-lane-guard` if invoked outside the gate (the gate exports
  `D2B_HEAVY_GATE` across its re-exec). Do not run them directly.
- **Convenience wrappers** `make heavy-check`, `make heavy-cargo-test`,
  `make heavy-flake-check`, and the `heavy-test-*` aliases run a Layer-1
  gate, the Rust suite, the building flake check, or a public lane under
  the same semaphore.

Run a heavy lane through its public target (or, for an arbitrary command,
`cargo run --manifest-path packages/Cargo.toml -p xtask -- heavy-gate --
<command>`) whenever another heavy lane might be running; the bare internal
targets stay available only for a serial console. Live-host and hardware
tests obey the same rule: use the gated live-VM smoke entrypoints (`make
pre-tag` for the full gate, `make smoke-lite` for the lite gate) or wrap a
raw live script as `cargo run --manifest-path packages/Cargo.toml -p xtask
-- heavy-gate -- env D2B_LIVE=1 bash tests/integration/live/<name>.sh`.

The `cargo run --manifest-path packages/Cargo.toml` form is deliberate:
there is no root cargo workspace, so the bare `cargo xtask` alias resolves
only when the working directory is `packages/`, and running it from the
repository root fails with `no such command: xtask`. Because cargo config
discovery is cwd-based, invoking `xtask` from the root via `--manifest-path`
silently drops the `sccache` configuration in `packages/.cargo/config.toml`;
that is immaterial for the gate itself. When it matters for a specific
command, `cd packages && cargo xtask <command>` is the equivalent form -
pick one per command and pass file arguments relative to the directory you
run from.

Invoking a live script directly is safe but not the documented path: each
one verifies the inherited slot and re-executes itself through the semaphore
exactly once when no genuine slot is held. A bare `D2B_HEAVY_GATE` value is
not trusted, so it cannot bypass the sole-use invariant.
**A new live, hardware, or performance entrypoint must carry that same
self-guard block**, or the fail-closed inventory guard
(`every_live_and_heavy_entrypoint_routes_through_the_gate`) rejects it.

### Spec-literal lint allowlist

The ADR 0046 spec-literal lints (`policy_adr046_spec_literals.rs`) enforce
three frozen decisions across `docs/specs/**`: D103 (the single 24-byte
`YYYY-MM-DDTHH:MM:SS.sssZ` datetime spelling), D104 (the single
`.d2bus.org.` ResourceType qualifier infix), and D108 (the integer
`retryAfterMs` retry-delay scalar superseding the old `retryAfter`
duration string). The allowlist is a pinned exact exemption, not an
author-suppressible marker: an inline `d2b-lint-allow` comment is
explicitly **not** honored and will not exempt a line - the lint rejects
that escape hatch by design, because a per-line marker would let any
future author silently suppress a real violation. The **only** exemption
is the decision-register table row that *defines* the rule (the `| <code> |`
row in `docs/specs/ADR-046-decision-register.md`), and that exemption is
pinned to that one file. Everywhere else, including a rejection
illustration, must be phrased so it does not embed the exact rejected
literal; correct the example rather than trying to silence the lint.

The same policy test checks the seven canonical feasibility measurements
against every Markdown and JSON document under `docs/**` plus `CHANGELOG.md`.
It inventories class-specific measurement signatures globally, including
run and group-commit denominators, the ChangeBatch comparison count, the
crash-boundary count phrase, RSS values with units, and each p95/p99 value
with its unit. Registered sites additionally pin their exact measurement or
qualitative outcome summary. The global scan deliberately does not match bare
numbers such as `13`, `20`, or `48`, because those are common in unrelated
prose. Consequently, a new copy that preserves a canonical number-and-unit,
denominator, or class phrase is rejected even in an unregistered document; a
free paraphrase that omits every inventoried signature remains a review
concern rather than something this lint claims to detect.

### Envelope policy lint (D116) negative-example marker

Unlike the spec-literal lints above - which honor no author-suppression
marker at all - the envelope policy lint (`policy_adr046_envelopes`)
recognizes exactly one deliberately narrow exemption. That lint enforces
D116 across `docs/specs/**`: a `Host` or `Guest` whose `allowedDomains`
admits the `user` domain must name a non-null, non-empty `defaultUserRef`
(D116 is frozen in `docs/specs/ADR-046-decision-register.md`). A block that
simply omits it is a real violation and must be corrected.

The one exception is an **intentional negative example**: a fenced example
(typically a Nix block) authored to *teach* the rule by demonstrating the
eval-time failure that omitting `defaultUserRef` produces. Deleting that
counter-example would lose correct teaching content, so the lint preserves
it - but only under three exact conditions it enforces together, not the
looser "names both `d2b-lint` and `d116`" shape earlier drafts of this
section described:

- **One exact, case-sensitive marker.** A comment line **inside the fence**
  whose text, after its `#` or `//` prefix is stripped, equals the marker
  string exactly. The current spelling is `# d2b-lint: expect-d116-eval-error`;
  the match is a whole-string, case-sensitive comparison, so a paraphrase or a
  comment that merely mentions the `d2b-lint` and `d116` tokens does not
  qualify.
- **One pinned file.** The marker is honoured only in the single documenting
  file the lint pins (currently `docs/specs/ADR-046-nix-configuration.md`).
  The same comment anywhere else exempts nothing and fails closed.
- **Exactly once.** The marker must appear a single time in that file. A
  second copy makes the exemption fail closed for the whole file, so every
  D116 block there is flagged again.

This is an unambiguous authoring signal for one intentional-rejection
example, never a general suppression switch. Never reach for it to silence a
D116 failure on a shape that is meant to be valid - correct the shape
instead. `policy_adr046_envelopes` is the authority for the exact spelling,
the pinned file, and the single-occurrence scope; a concurrent hardening may
tighten them further, so if you are adding a legitimate negative example take
the current requirement from that lint, not from this paragraph.

For where tests live, when to add or retire each kind of test, and
which pins/ledgers to update, read [`tests/AGENTS.md`](./tests/AGENTS.md).
[`tests/README.md`](./tests/README.md) is the human quick-start for the
same test model.

## Development workflow

## Changelog & Releases

Every PR that changes code **must** ship release notes. The CI gate
enforces this and accepts either form: an entry in `CHANGELOG.md`, or a
changelog fragment under `changelog.d/`.

### Format

[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Add entries under
`## [Unreleased]`. When ready to release, rename the section to
`## [X.Y.Z] - YYYY-MM-DD`.

### Fragments (`changelog.d/`)

When more than one branch is in flight, do **not** edit `CHANGELOG.md` -
every branch appending to the same `## [Unreleased]` block is a guaranteed
merge conflict. Write one `changelog.d/<branch-name>.md` fragment instead,
holding the same `### <Section>` headings and entries you would have added
to the block. Two branches never write the same file.

The integrator folds the fragments at merge time with
`make changelog-fold` (`cargo run --manifest-path packages/Cargo.toml -p
xtask -- changelog-fold`): entries collate by
section into `## [Unreleased]` in Keep a Changelog order, released
versions are untouched, and the consumed fragments are deleted. A
fragment with an unknown heading, a repeated heading, an empty section, or
content outside a section fails the fold rather than losing the entry. See
[`changelog.d/README.md`](./changelog.d/README.md).

### Auto-release

Merging to `v3` with a new version header in `CHANGELOG.md` triggers:
1. Auto-creation of git tag `vX.Y.Z`
2. Build of all host binaries (`d2bd`, `d2b`, `d2b-priv-broker`,
   `d2b-wayland-proxy`, `d2b-activation-helper`)
3. GitHub Release with changelog notes + binary tarballs + `SHA256SUMS`

`v3` is the clean-break integration lineage and never merges to `main`, so
the release path cuts from `v3`, not `main` (see
[`docs/specs/ADR-046-validation-and-delivery.md`](./docs/specs/ADR-046-validation-and-delivery.md)
"Only after all six hold").

Consumers can fetch pre-built binaries from the release instead of
building from source.

### Versioning

Follow semver. The version in `CHANGELOG.md` is the single source of truth.

### Worktrees for parallel agents

When several agents (or several humans, or a mix) work on disjoint
scopes concurrently, use git worktrees instead of branching in
place. One worktree per agent keeps each context isolated and makes
the final merge trivial.

```bash
# From the primary clone, one worktree per concurrent scope:
git worktree add -b phase-<name> ../d2b-<name> main
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
sequential merge with conflict resolution - octopus only works for
clean disjoint scopes.

#### Finish-of-work invariant: merge back into the primary clone

A worktree is a workspace, not a destination. When an agent's scope
is done - implementation green, tests green, panel signed off - the
agent merges the worktree branch back into `main` in the **primary
clone (`projects/d2b`)** before declaring the task complete.
Finished work sitting on a side worktree branch is not done; it is
"awaiting integration", which is a state the agent owns, not a state
the agent leaves for the operator.

Concretely, the agent that owns a worktree:

1. Verifies green on the worktree (`cargo test --workspace`, the
   relevant `tests/*.sh` gates, panel signoff for plan-driven work).
2. From the primary clone (`/home/paydro/projects/d2b`),
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

### Stacked PR workflow for large waves

Large realm/control-plane waves that are not file-disjoint by default land
through a private stacked-PR workflow, not by direct local merges to `main`.
This is the default for ADR-scale work where one branch defines contracts that
later branches consume.

Use this shape:

1. Open one private branch/worktree per independently reviewable slice. Branch
   names should describe the wave and scope, for example
   `realm-workloads-w13-adr`, `realm-workloads-w14-options`, or
   `realm-workloads-w17-wlcontrol`.
2. Stack only when necessary. A later branch may target an earlier PR branch
   while it consumes new DTOs, schemas, or option contracts. Branches that do
   not depend on each other target `main` directly.
3. Open PRs for every slice. Do not merge locally into `main`, and do not push
   directly to `main`. The integrator merges only through GitHub PR flow after
   local validation, CI, and required panel/review gates pass.
4. PR bodies must list the change, validation evidence, and any substantive
   panel/review outcomes. Do not include AI/tool/model attribution.
5. Review and panel agents inspect code, docs, plans, screenshots, and supplied
   validation evidence. They must not run tests or long gates unless the
   integrator explicitly asks that reviewer to do so.
6. The integrator owns CI babysitting, retargeting, rebasing, conflict
   resolution, merge order, and branch deletion. If a lower PR merges, retarget
   or rebase dependent PRs promptly and rerun the smallest relevant validation.
7. When a stack updates host inputs, update `/etc/nixos` only after the upstream
   PRs are merged and validated. Then switch the host, restart `d2bd`, verify
   runtime/desktop behavior, and commit the host lock/config change separately.
8. If helper scripts are added for stack status, retarget/rebase, or
   wait-and-merge behavior, they must use `gh`, avoid direct main merges, and
   fail closed on dirty worktrees, failed checks, ambiguous merge state, or
   missing validation evidence.

For stacks that require panel gates, the first PR in the stack usually carries
the contract/ADR/plan update. Do not dispatch implementation PRs for later
waves until the plan/ADR panel returns unanimous signoff.

### Screenshot and visual artifact hygiene

Screenshots and other visual artifacts submitted as validation evidence or
committed to the repository must be redacted before use:

- Remove or black out all secrets, credentials, API keys, and tokens visible in
  any terminal, browser, or UI window.
- Remove or replace personally identifiable information (PII): real names, email
  addresses, employee ids, user ids, and similar identifiers.
- Replace or black out sensitive command output: stack traces with host paths,
  raw error messages with internal node names or realm principals, clipboard
  content, and any window title or app metadata that names a real person or
  organization.
- Use generic placeholder names (e.g., `alice`, `corp-vm`, `work`) matching the
  conventions in the Don'ts section above.

Do **not** commit unredacted screenshots to the repository. Panel and review
agents may inspect screenshots as part of validation evidence; the same
redaction rules apply when attaching screenshots to PR bodies or panel prompts.
If a screenshot cannot be adequately redacted without losing the information
being demonstrated, use a text description or a synthetic reproduction instead.

### Local host validation after updating d2b

When a host configuration switches to a new d2b checkout (for
example a local `path:/home/paydro/projects/d2b` input), the host
switch updates `/etc/d2b/*` and the system packages and may restart
`d2bd`. That daemon restart is a continuation event: VMs must stay
running, protected by `KillMode=process`, and the restarted daemon
re-adopts their runner pidfds. Before runtime validation, make sure the
notify-ready daemon is active on the updated generation:

```bash
sudo systemctl restart d2bd.service
```

Then restart affected VMs with the normal lifecycle commands (on this
host, prefer `d2b down <vm> --apply` followed by
`d2b up <vm> --apply`; `d2b switch <vm>` is not reliable here).

#### Integrator-prep-first pattern (W3 onwards)

For waves whose thematic scopes are NOT file-disjoint by default -
W3 host-prepare is the canonical example, with scopes s1-s5
naturally sharing `packages/d2b-contracts`, `packages/d2b-core`
DTOs, schemas, and `Cargo.toml` workspace pins - the wave is
preceded by an **integrator API/contract prep commit landed
directly on `main`** before any scope worktree is opened. That
prep commit:

- adds every shared crate, DTO module, broker enum variant,
  privileges row, schema regeneration, and `Cargo.toml`
  workspace-dep change the parallel scope commits will read;
- carries the canonical trailing tag `( W3 )` (no scope label
  inside the parens - scope labels are subject-prefix only,
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
2. Consumer hosts that vendor d2b tend to ship auto-backup
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
is no `d2b@<vm>`-style per-VM unit; `d2bd` supervises every
per-VM DAG in-process and hands fds to spawned runners via the
broker's `SpawnRunner` / `OpenPidfd` ops.

| Resource                                | Pattern                                |
| --------------------------------------- | -------------------------------------- |
| Public daemon (supervisor)              | `d2bd.service`                     |
| Privileged broker socket                | `d2b-priv-broker.socket`           |
| Privileged broker service               | `d2b-priv-broker.service`          |
| Lifecycle permission group              | `d2b` (singleton)                  |

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

- [`vicondoa/entrablau.nix`][entrablau] - Microsoft Entra ID
  joins (Himmelblau + TPM-bound machine credential).

Optional **desktop companion** pieces also live in sibling flakes:

- `vicondoa/d2b-toolkit` - shared Rust/Nix client DTOs, public-socket
  framing, redaction wrappers, Wayland color parsing, and Waybar helpers for
  desktop integrations.
- `vicondoa/d2b-wlterm` - Home Manager module and user-session launcher for
  persistent guest shells.
- `vicondoa/weezterm` - WeezTerm package/provider integration used by the
  terminal launcher when a d2b-aware terminal build is desired.

Consumer flakes that combine these pieces keep a single nixpkgs and toolkit
revision by using `inputs.d2b.inputs.nixpkgs.follows = "nixpkgs"`,
`inputs.d2b-toolkit.inputs.nixpkgs.follows = "nixpkgs"`, and
`inputs.d2b-wlterm.inputs.d2b-toolkit.follows = "d2b-toolkit"`. WeezTerm
follows only `nixpkgs`; its flake does not expose a toolkit input. The exact
copy-paste boilerplate lives in
[`docs/how-to/configure-desktop-terminal-integration.md`](./docs/how-to/configure-desktop-terminal-integration.md).

The composition pattern is intentionally one-way: d2b core does not import
identity, workload, or desktop companion flakes. Identity/workload flakes can
stay d2b-agnostic; desktop companions consume only d2b's public CLI/socket
contracts. Consumers compose workload modules on a specific VM:

```nix
d2b.vms.work.config.imports = [
  inputs.entrablau.nixosModules.default
];
```

If you're tempted to add a new sibling-shaped concern (e.g. a
specific desktop environment, a particular dev-shell flavour) to
the core framework, consider whether it belongs in its own flake
instead. The bar for landing it in core is: "every d2b user
plausibly wants this, and the framework cannot do the right thing
without it."

[entrablau]: https://github.com/vicondoa/entrablau.nix

### VM lifecycle (daemon-supervised)

`d2bd` is the sole supervisor for every per-VM lifecycle DAG.
There are no framework-declared per-VM systemd units: child
processes (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound,
USBIP attach) are spawned by the broker via `SpawnRunner`, handed
back to `d2bd` over `SCM_RIGHTS` as pidfds, and reconciled
against the persisted DAG state under
`/var/lib/d2b/supervisor/state.json`.

Stop is provider-aware for local primary VMM runners. Normal
`d2b vm stop` asks Cloud Hypervisor guests to shut down via the CH
API and qemu-media guests via broker-mediated QMP before pidfd signal
cleanup. `--force` is an explicit operator override that skips only
that graceful guest wait and then uses the standard SIGTERM/SIGKILL
cleanup path. `d2b.daemon.lifecycle.gracefulShutdown.*` and
`d2b.vms.<vm>.lifecycle.gracefulShutdown.*` configure the bounded
wait; disabled VMs bypass the graceful phase without being marked
degraded.

The restart policy applies differently to the two daemon units (no
per-VM units are emitted):

- `d2bd.service` is `Type=notify` and may restart on switch/update.
  Systemd does not report it ready until the public socket is bound and
  the daemon has completed startup/adoption. `KillMode=process` ensures a
  daemon restart kills only the daemon main PID, not VM runner
  descendants; the restarted daemon re-adopts existing runners. The
  existing guarded `ExecStop` host-shutdown hook remains the all-VM
  teardown path and runs only when the system manager is stopping.
- `d2b-priv-broker.service` is socket-activated. It reloads the
  current bundle resolver for each accepted request so a running broker
  does not dispatch stale runner intents after a switch, and it never
  holds in-flight session state across requests.

Drift detection moves from per-VM symlinks into the daemon's
state file. `d2b vm list` flags any VM where the running
closure differs from the latest declared closure with
`[pending restart]`; `d2b vm status <vm>` prints both store
paths and the exact remediation command (`d2b vm restart <vm>`
for a clean down+up, `d2b vm switch <vm>` for a per-VM closure
rebuild + live activation).

#### Adding new per-VM behaviour

New per-VM work belongs **inside the daemon's DAG executor**
(`packages/d2bd/src/supervisor/`), with any privileged side
effects routed through a typed `d2b-priv-broker` op declared
in `packages/d2b-contracts/` and audited in
`/var/lib/d2b/audit/broker-<utc-date>.jsonl`. Do not introduce
a new `systemd.services.*` declaration in `nixos-modules/` for
per-VM work. The denylist coverage lives in
`packages/d2b-contract-tests/tests/policy_units.rs`; run the enabled
fixture-contract lane when changing this surface. See
[`docs/explanation/daemon-lifecycle.md`](./docs/explanation/daemon-lifecycle.md)
for the DAG node taxonomy and
[`docs/reference/privileges.md`](./docs/reference/privileges.md) for
the broker op catalogue.

Adding or reclassifying a spawned runner `ProcessRole` also requires
matching process-builder and runner-matrix coverage: add/extend the
typed Rust argv builder in `packages/d2b-host/src/*_argv.rs` and
the role coverage policy/contract tests under
`packages/d2b-contract-tests/tests/` in the same change.

## Panel review

### Phase gate

Multi-phase plans MUST pass a panel sign-off gate at each phase
boundary. The integrator MUST NOT begin the next phase until every
reviewer on the selected roster returns `signoff: true` (N/N for the
plan's panel size; the default roster below is 10).

For plan-driven work, a "phase" is usually one wave from the plan's
parallelization graph (`Wave 0`, `Wave 1`, ...). For tiny plans that
touch fewer than three files, a single phase covering the whole plan is
acceptable.

For each phase:

1. **Plan review** - panel reviews the plan; iterate until N/N
   sign-off. The integrator may not dispatch implementation subagents
   until this gate passes.
2. **Implementation** - dispatch subagents in parallel per the
   dependency graph.
3. **Integration** - integrator merges subagent output.
4. **Work review** - panel reviews the integrated diff; iterate via
   fix-subagents until N/N sign-off.
5. **Advance** - only now may the integrator begin the next phase's
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

- **Swarm-driven work** satisfies the per-round gate with swarm's
  five-seat phase council instead of a ten-role panel round. See
  [Running the panel under swarm](#running-the-panel-under-swarm). The
  substitution covers only the per-round gate; the binding wave panel is
  untouched.
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

### Running the panel under swarm

There are three review surfaces in this repository and they are strictly
ranked. Read this ordering before wiring any harness.

1. **The binding ten-role panel** - `cargo run --manifest-path
   packages/Cargo.toml -p xtask -- delivery wave panel-request` /
   `panel-attest` / `seal`. This is the authority for an ADR 0046 wave.
   It runs **once, at wave close**, against the wave's one immutable
   snapshot, and it is enforced in code by
   `packages/xtask/src/delivery/panel.rs`: exactly one record per role
   for all ten roles, `signoff` true iff `recommendations` is `[]`,
   unanimous ten of ten, every record bound to the same
   `candidate_id`/`content_id`/`snapshot_sha256`, and provider/model/
   reasoning effort pinned to `github-copilot` /
   `gemini-3.1-pro-preview` / `high`. The panel model is deliberately
   not the coding model, so a lane cannot both author a change and
   attest to it. There is no override, no force flag, and no partial
   pass.
   See [`docs/specs/ADR-046-validation-and-delivery.md`](./docs/specs/ADR-046-validation-and-delivery.md)
   section 12.3.
2. **The per-round phase panel** - the [Phase gate](#phase-gate) rule
   above. Where ADR 0046 restricts the *binding* panel to one per wave,
   this rule allows a panel per implementation round. This is the loop
   swarm automates.
3. **Swarm's five-seat phase council** - the per-round gate whenever
   swarm drives the work. It stands in for surface 2 and has no bearing
   on surface 1.

**Swarm runs surface 2, not surface 1.** Under swarm the five-seat
council is the per-round gate: no ten-role panel round is required
between implementation rounds, which is the whole point of running the
harness. Surface 1 is unchanged, because ADR 0046 section 12.3 already
restricts the binding panel to exactly one run at wave close and never
per implementation round. A green phase council is therefore not a
sealed wave, and `phase_complete` passing is not `delivery wave seal`
passing.

**The 10 roles at wave close.** The ten-role roster is no longer run
every round. It runs once, at wave close, to produce the records
surface 1 consumes: dispatch one read-only lane per roster role via
`dispatch_lanes_async`, seeded with that role's focus cell from the
table above plus the integrator's validation evidence. Lanes are
read-only by contract, which keeps them off the shared Nix store, cargo
target directory, and heavy gate semaphore. Lane ids are free-form, so
all 10 roles vote independently and each lane's verdict maps one-to-one
onto a `panel-attest` record.

To keep those records attestable, the reviewing agents must run on the
pinned panel binding. The `panel` entry under `agent` in
`.opencode/opencode.json` pins them to
`github-copilot/gemini-3.1-pro-preview` at reasoning effort `high` and
denies the write, edit, patch, and bash tools, matching the read-only
lane contract above. A lane on any other model produces a record
`panel-attest` will reject, so do not let model fallback silently
downgrade a panel lane, and do not dispatch a panel lane through the
`general` agent - that one is pinned to the coding model
`github-copilot/gpt-5.6-sol` and its records are rejected by design.

**The per-round council, and what it costs.**
`submit_phase_council_verdicts` has a closed five-member roster
(`critic`, `reviewer`, `sme`, `test_engineer`, `explorer`) and
deduplicates by member, so ten distinct votes cannot be cast against it.
Each seat carries the concerns of the roster roles nearest it:

| Seat            | Covers                          |
|-----------------|---------------------------------|
| `reviewer`      | `software`, `rust`              |
| `test_engineer` | `test`                          |
| `sme`           | `nixos`, `networking`, `kernel` |
| `critic`        | `security`, `product`           |
| `explorer`      | `docs`, `observability`         |

A seat MUST NOT return `APPROVE` while any concern it covers is open.
Accept the tradeoff knowingly: five synthesizers can agree where ten
independent reviewers would have dissented, and the observability
precedent above is exactly that failure shape. That is why this council
gates a round and not a wave, and why the ten-role panel still runs
before the seal.

**Verdict rule.** Swarm's default is more permissive than this file: a
`CONCERNS` verdict carrying only MEDIUM/LOW findings still passes. The
repository rule, and the rule `panel.rs` enforces, is `signoff: true`
iff `recommendations` is `[]`. Set
`council.phaseConcernsAllowComplete: false` so `CONCERNS` blocks like
`REJECT`; that is a required part of the project config.

**Gate wiring.** Enable the gates before the QA profile locks
(`set_qa_gates` is ratchet-tighter and rejects all writes once critic
approval or drift evidence locks it):

```
phase_council, final_council, drift_check,
hallucination_guard, critic_pre_plan, sme_enabled
```

`phase_complete` then refuses to close a phase without
`.swarm/evidence/<phase>/phase-council.json`.

**Plan review.** Swarm has no gate that blocks dispatch on a
phase-scoped plan panel; `critic_pre_plan` is a single critic, once,
project-wide. Encode the plan gate as work instead: make task `N.1` of
every phase the plan-review task, declare the plan itself as its
acceptance criteria via `declare_council_criteria`, and give every
implementation task in that phase a `depends` edge on it. Per-task
council then enforces the plan gate before any coder is dispatched.

**Waves and file ownership.** `epic_decide_phase` followed by
`epic_plan_waves` is the direct implementation of the parallelization
graph, and a `declare_scope` call per task is the file-ownership map
described in [Integrator-prep-first pattern](#integrator-prep-first-pattern-w3-onwards).
Record `epic_record_divergence` after each task completes; declared
scope versus files actually touched is calibration data the manual
process never captured.

### Unattended multi-day runs

Long plans are expected to run for days with the operator away. Two
things make that work, and one thing makes "zero interaction"
unachievable.

**Removing the routine prompts.** Set `execution_profile.auto_proceed:
true` on the plan to drop the phase-boundary confirmation, and enable
Full-Auto (`full_auto.enabled: true`, `mode: "supervised"`) so safe
in-scope operations stop asking. Writes to protected paths still route
through the read-only `critic_oversight` agent rather than blocking.

**Escalation is a pause, not a stop-the-world.** Keep
`full_auto.escalation_mode: "pause"` and `full_auto.denials.on_limit:
"pause"`. `terminate` kills a multi-day run outright; `pause` parks it
recoverably, and `.swarm/` state survives process restarts.

**Zero user interaction is not achievable, by design on both sides.**
`full_auto.escalation_mode` admits only `pause` and `terminate`, there
is no autonomous mode, and `council.escalateOnMaxRounds` is declared
but not implemented - exhausting `council.maxRounds` without an
`APPROVE` surfaces a message for the operator and refuses to
auto-advance. Surface 1 is stricter still: a wave cannot seal without
ten human-attested records, so the binding panel is a deliberate
human-in-the-loop stop that no configuration removes. That matches this
file's own rule that green tests never waive the gate. Plan for
**batched escalation**: the run parks on unresolved disagreement,
`/swarm status` reports why, and the operator services the queue when
convenient. Raising `council.maxRounds` to 5 lets more disagreements
self-resolve before parking; it does not remove the park.

**Context.** A days-long session will cross the context budget's
critical threshold. Treat phase boundaries as the handoff points rather
than fighting the guard mid-phase.

**Heavy lanes.** Advisory panel lanes are read-only and take no heavy
gate slot. Any reviewer explicitly asked to run a validation is subject
to the normal two-slot semaphore in [Heavy lanes](#heavy-lanes), and an
unattended run must not exceed it.

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
use the compact form `(W2fu4 H10)` for readability - that's
shorthand for citing a commit, not the literal trailing tag
that the commit subject must end with. The trailing-tag form
in the commit subject itself always uses the spaced canonical
form (e.g. `... ( W2fu4 H10 )`).

### Tooling note

The panel contract is implementation-neutral: any harness that
preserves the roster, the unanimity rule, the no-rerun discipline, and
the two gates per phase is acceptable.

The in-repo reference implementation is `.opencode/opencode.json`. Its
`agent` table is the tracked, reviewable surface for panel behaviour:
`panel` carries the reviewing binding and the read-only tool set, while
`general` and `explore` carry the coding binding. Change that file in
the same commit as any change to this section.

The ADR 0046 program does not run swarm. Where this section describes
swarm's five-seat council, treat it as documenting an available harness
rather than the configuration in use; the per-round gate is run
directly, and the binding wave panel is dispatched as ten read-only
`panel` lanes.

A second, host-local implementation lives in
`/etc/nixos/scripts/panel-review.{md,sh}` and
`/etc/nixos/scripts/panel-aggregate.sh`. That tooling is paydro's
host-specific implementation, not an upstream d2b dependency. In it the
roster is selected per plan via `ENGINEERS_FILE` and each engineer's
focus file comes from `panel-roles/<engineer>.md`.

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

## Versioning & changelog

The project follows [Semantic Versioning](https://semver.org/) and
[Keep a Changelog](https://keepachangelog.com/). The CHANGELOG is
organised **by version**, never by development phase.

### Changelog lifecycle

- **While a version is in development**, entries accumulate under the
  top `## [Unreleased]` block. It remains consumer-facing and follows
  the same process-marker ban as released sections; wave, phase,
  follow-up, round, panel, and finding bookkeeping stays in plans,
  commits, and PR descriptions.
- **When a version is cut**, the `[Unreleased]` block is renamed to
  `## [X.Y.Z] - YYYY-MM-DD` and its contents are **summarised by
  version**:
  - Collapse any per-wave/per-phase substructure into the standard
    Keep-a-Changelog groups (`Added`, `Changed`, `Fixed`,
    `Deprecated`, `Removed`, `Security`). There are no
    `### Added (W6)`-style subsection headers in a released section.
  - Strip every internal process marker - wave/phase/revision/
    follow-up/panel/round/finding tags such as `W3`, `W4-fu`,
    `( W1fu3 H20 )`, `P6`, `D5/P2.3` - from the released prose.
  - Each released section reads as a coherent, consumer-facing
    summary of what changed, not as a log of how the work was
    organised internally.
- A fresh empty `## [Unreleased]` block is left at the top after a
  cut. `manifestVersion` / `bundleVersion` bumps and breaking
  changes always get an explicit released entry.

### Process markers stay out of shipped artifacts

Internal development bookkeeping - wave tags (`W3`, `W4-fu`,
`W2-followup`), phase tags (`P0`-`P7`, `v1.1-P4`, `ph6-…`),
decision codes (`D5/P2.3`), follow-up/round/finding refs
(`fu3`, `H20`, `(rust-1)`) - is for organising work, not for
shipping. Do **not** introduce these markers into:

- source comments in `nixos-modules/`, `pkgs/`, `packages/`, or `proofs/`;
- shipped docs prose under `docs/{reference,how-to,explanation}/`,
  `proofs/**/*.md`, `README.md`, `SECURITY.md`, or example READMEs;
- any user-facing CLI surface (`clap` `about`/`help`/`long_help`
  text, error/observed-state messages, JSON envelope fields);
- CI workflow names, job names, step names, and test output that a
  contributor sees in GitHub Actions logs. CI labels should describe
  the behavior being validated (for example, "ADR index coverage
  guard" or "host validate dry-run"), not historical phase/process
  codes;
- every CHANGELOG section, including `[Unreleased]`.

These markers are still expected and welcome in the contexts where
they are load-bearing:

- planning artifacts (a session `plan.md`, the wave/parallelization
  graph);
- this file and the other process docs (Panel review, Commit
  conventions, `## Daemon-only end-state (P6 onward)`) that
  *document* the methodology;
- `docs/adr/**` - ADRs are dated historical records and may name the
  wave/phase that produced a decision;
- commit messages and PR descriptions on in-development feature
  branches (see Commit conventions).

The ban is mechanically enforced by `scan_process_markers` in
`tests/tools/tier0-first-pass.sh`, which runs as part of
`make check-tier0`. That script is authoritative for the governed
paths, marker patterns, narrow functional exceptions, exact diagnostics,
and use of the active exemption set. The pin's typed schema and frozen
universe are independently checked by
`packages/xtask/src/process_marker_pin.rs`; consult both implementations
when changing the ratchet.

Existing violations are recorded in
`tests/golden/pinned/process-marker-legacy-paths.json`. Its
`activePaths` array is the current exemption set and `retiredPaths`
records cleaned paths. Both arrays must be sorted and disjoint, every
entry must be a normalized relative path, and their combined path
universe must match the fixed SHA-256 digest embedded in both checkers.
The digest freezes the combined universe; there is no editable count
budget and no permitted swap that adds a different path.

An active path is exempt only while the scanner still finds a violation
there. Cleaning that path makes the gate fail with a `STALE:` line; move
the path from `activePaths` to `retiredPaths` in the same change, preserving
the frozen universe. A retired path is not exempt, so a marker there is
reported as a new violation. Handle the contributor-facing failure modes
as follows:

- For a new violation outside the allow-list, remove or reword the
  marker. If it is a genuine functional identifier, add a narrowly
  scoped scanner exception with policy review rather than growing
  legacy debt.
- For a stale active entry, move it to `retiredPaths`; do not delete it
  from the frozen universe.
- For a pin validation failure, restore sorted, unique, normalized arrays
  whose disjoint union matches the embedded digest. Do not add, delete, or
  replace a frozen path.

The exact scanner failure text may evolve;
`tests/tools/tier0-first-pass.sh` remains the authority for it, while
`packages/xtask/src/process_marker_pin.rs` is authoritative for typed pin
validation.

There are two deliberate functional exceptions. The consumer-facing
`d2b.defaultSwitchReadiness.<wave>` option namespace (keys
`w4Fu`…`p7`), its `readinessWaveSpecs` schema, and the
`/var/lib/d2b/validated/<wave>.json` evidence contract use
`wave`/phase tokens as **functional identifiers**. Those are part of
the public option/schema surface and are not bookkeeping; leave them.

`packages/xtask/src/delivery/` also has a narrow exception for the
delivery tool's closed `W0` through `W8` namespace. These exact tokens
identify CLI values and state-path segments rather than development
bookkeeping. The exception applies only inside that delivery
implementation; suffixed bookkeeping forms remain violations.

### Landing changes (PR workflow)

`main` is protected: changes land via pull requests, not direct
pushes. Develop on a feature branch (or worktree), validate locally
against the gates above, open a PR, let CI run, then squash-merge. The
detailed wave-tag commit convention in
[Commit conventions](#commit-conventions) applies to in-development
commits on those feature branches; `main` itself is maintained as a
by-release history.

PR bodies record the change, validation evidence, and substantive
review outcomes only. Do **not** tag or list the AI agent, assistant, or
model used to author or review a change, and do not add PR-template
fields that request panel, agent, or model metadata.

## Commit conventions

> The trailing wave-tag scheme below applies to in-development
> commits on feature branches / worktrees, where wave/phase tags are
> load-bearing planning context. It does not license process markers
> in shipped code, docs, or any CHANGELOG section - see
> [Versioning & changelog](#versioning--changelog).

- **Subject.** Short, imperative, prefixed with the touched
  area: `net: fix 10-eth-dhcp neutralization`,
  `manifest: bump manifestVersion to 2`,
  `cli: tighten exit-code table`.
- **Body.** Wrap at ~72 cols. Explain *why*, not what - the diff
  shows the what.
- **Traceability - canonical tag form (forward, W2fu4+).**
  Every commit subject MUST end with a trailing parenthesized
  tag in one of these exact forms:

  - `( W<N> )` - wave-N implementer work (no finding ref)
  - `( W<N>fu<M> )` - wave-N follow-up round M integrator
    merge (no finding ref); merge-shape suffixes like
    `octopus` are NOT permitted in the tag
  - `( W<N>fu<M> <S><N> )` - single finding fixed in
    follow-up round M. The finding-tag is `<S><N>` where
    `<S>` is the severity letter from the reviewer JSON
    (`C` = critical, `H` = high, `M` = medium, `L` = low)
    and `<N>` is the ordinal within that severity. Example:
    `( W2fu1 H3 )` = wave 2, follow-up 1, HIGH-3.
  - `( W<N>fu<M> <S1><N1> <S2><N2> ... )` - multi-finding
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
  - `( W<N> <S><N> )` - single finding fixed inside the
    wave itself (rare; usually findings come during follow-ups)
  - `( W<N>a-<H> )` or `( W<N>a H<H> )` - post-wave **opening
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
  parenthesized tag - those go in prose. Every commit
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
- **Typography.** Only the ASCII hyphen `-` may spell a dash in the
  subject or the body. See the Don'ts entry for the repository-wide rule
  and the banned codepoint list.
- **AI/tool attribution.** Do not tag or list the AI agent, assistant,
  or model used in commit subjects, commit bodies, PR descriptions,
  changelog entries, or shipped docs. Do not add `Co-authored-by`
  trailers for AI tools unless the human explicitly requests one for
  that change.
- **Atomicity.** One logical change per commit. Mechanical
  reformat or rename passes go in their own commit so the
  human-reviewable diff stays small.

## Disk hygiene contract

- Put every throwaway probe, one-off crate, parser experiment, and debugging
  artifact under the gitignored repository-root `.scratch/` directory.
  Never place an exploratory file beside production code or tests, where a
  catch-all `git add` can sweep it into a commit.
- Test eval expressions MUST resolve the flake via `git+file://$ROOT`
  (use the `d2b_flake_ref` helper in `tests/lib.sh`), **never**
  `builtins.getFlake (toString $ROOT)`. A bare path makes Nix use the
  `path:` fetcher, which copies the ENTIRE working tree into the store -
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
  TRACKED files but NOT untracked files - identical to `nix flake check`,
  so "commit before building" still holds (see "Edit -> commit ->
  validate").
- Every test script that creates repo-local scratch state MUST use
  `d2b_mktemp` from `tests/lib.sh`; do not call raw
  `mktemp -d -p "$ROOT"`.
- Per-process bookkeeping (`cleanups.<PID>`, `scratch-registry`)
  lives in `${D2B_BOOKKEEPING_DIR:-${TMPDIR:-/tmp}/d2b-bookkeeping}`,
  NOT in `$ROOT`. Parallel-test timing log/status files live in
  `${TMPDIR:-/tmp}/d2b-static-timing.$$/`. Both moves are
  required so volatile files can't race
  `builtins.getFlake (toString $ROOT)` source-capture during
  flake-eval gates (W2fu4 H8/H9).
- Rust worktrees do NOT share a cargo target directory. Each worktree
  keeps its own `packages/target/`; compiled-output dedup across
  worktrees comes from `sccache` (`$SCCACHE_DIR`, default
  `~/.cache/d2b-sccache`), wired by the `[build] rustc-wrapper` lines in
  `packages/.cargo/config.toml` and the sibling-workspace configs under
  `packages/d2b-priv-broker/`, `packages/d2b-guest-shell-runner/`, and
  `packages/d2b-core/fuzz/`. A shared target dir is deliberately
  avoided: cargo's target-dir lock is workspace-wide, so two worktrees
  building concurrently at different SHAs would serialize pessimistically
  and stomp each other's incremental caches. To bypass sccache locally
  (e.g. when bisecting a compiler issue), set `RUSTC_WRAPPER=` or
  `CARGO_BUILD_RUSTC_WRAPPER=` explicitly.
- The persistent-shell helper is intentionally excluded from the main
  Rust workspace at `packages/d2b-guest-shell-runner/`. Run it by
  manifest path (and with `--features real-libshpool` when checking the
  real shpool bridge); the top-level Rust/static/supply-chain gates wire
  it explicitly like the broker workspace.
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
  D2B_POST_GATE_DEEP_GC=1 bash tests/static.sh           # user gens only
  D2B_POST_GATE_DEEP_GC=1 \
  D2B_POST_GATE_DEEP_GC_SUDO=1 \
  bash tests/static.sh                                  # + system gens
  ```

  `D2B_POST_GATE_DEEP_GC_SUDO=1` uses `sudo -n` and skips fail-open
  with a clear log if passwordless sudo isn't available. Threshold
  defaults to 7 days; override with `D2B_POST_GATE_DEEP_GC_DAYS=N`.
  Off by default - this is operator policy, not gate policy.
- `D2B_SKIP_WITH_ENTRA_ID=1` skips the per-example flake check for
  `examples/with-entra-id` when its pinned `vicondoa/entrablau.nix`
  input fails the per-example cargo fetch with a transient crates.io
  403 against `libhimmelblau-0.8.18` / `kanidm-hsm-crypto-0.3.6`.
  `tests/static.sh` performs one in-band retry before failing the
  example; the skip knob is an explicit, panel-justifiable W3
  carve-out used only after the retry also fails. Added with the W3
  integration merge; re-evaluate once the entra-id input bumps past
  the affected revision.
- Before `git worktree remove`, delete the worktree's real
  `packages/target/` (every worktree has one; there is no shared-cache
  symlink) so the removal reclaims its multi-GiB build artifacts.
  Rebuilds in a fresh worktree stay cheap because sccache retains the
  compiled outputs.
- `tests/tools/preflight-disk-space.sh` fails the wave when free disk under
  `$ROOT` drops below 10 GiB. Runs after the orphan reapers but BEFORE
  the rust toolchain bootstrap so the fail-closed guard cannot be
  bypassed by disk-consuming setup (W2fu4 H2).
- `nix flake check` now builds real `cargo-deny` + `cargo-audit`
  derivations (via `checks.${system}.rust-deny` / `.rust-audit`).
  Each derivation fetches the pinned RustSec advisory DB snapshot
  from the Nix store (no network at build time) and runs cargo-deny /
  cargo-audit against both `packages/Cargo.lock` and
  `packages/d2b-priv-broker/Cargo.lock`. The advisory DB is a
  `fetchFromGitHub` pinned to a specific commit; update the rev + hash
  in `flake.nix` periodically to pick up new advisories. Wall-clock
  impact: seconds per check (no compilation, just lockfile analysis).

## Critical subsystems - handle with care

Touch these only with a clear plan and a corresponding test run.

| System                              | Where                                                                                  | Risk if broken                                                            |
| ----------------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Net VM networking / firewall        | `nixos-modules/net.nix` (the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus the per-env MTU/MSS and east-west wiring) | Net VM dual-stacks DHCP on its uplink, breaks NAT, or weakens same-env isolation unexpectedly. Validate with `tests/unit/nix/cases/net-vm-network.nix`. |
| Per-VM `/nix/store` hardlink farm   | `nixos-modules/store.nix`, `/var/lib/d2b/vms/<vm>/store{,-meta}/`, `nixos-modules/processes-json.nix` (`virtiofsdRunner` ro-store `--shared-dir`), daemon `StoreSync` op + broker `store_view_farm` | The guest's `/nix/store` MUST be the per-VM closure-only farm `/var/lib/d2b/vms/<vm>/store`, never the host's full `/nix/store`: virtiofsd-ro-store's `--shared-dir` points at that farm (the `share.source == "/nix/store"` string stays as the eval-time sentinel - do not "simplify" it back to serving `/nix/store`, that re-leaks the whole host store to every guest). Requires `/var/lib/d2b` and `/nix/store` on the **same filesystem** - hardlinks can't cross FS boundaries; if split, `d2b vm switch` refuses with a fatal error. The broker builds the farm inside a private mount namespace where `/nix/store` is lazily detached (NixOS bind-mounts `/nix/store` on itself, so a same-`st_dev` cross-vfsmount `link(2)` returns `EXDEV` - recoverable, distinct from a fatal different-filesystem `EXDEV`); a `link(2)` `EMLINK` on a `--optimise`d store's saturated empty-file inode falls back to a byte copy. The daemon owns the sync; there is no per-VM `store-sync` unit. |
| TPM persistence (per-VM swtpm)      | `/var/lib/d2b/vms/<vm>/swtpm/`; spawned via broker `SpawnRunner` from `packages/d2b-host/src/swtpm_argv.rs` and supervised by `d2bd` as a child of the VM's DAG. The broker **provisions + hardens** this dir on first start (`packages/d2b-priv-broker/src/ops/swtpm_dir.rs`, gated on `seccomp_policy_ref == "w1-swtpm"`): fd-safe create (owner `d2b-<vm>-swtpm`, mode 0700, inherited ACLs cleared), reconcile-in-place on a correct-owner existing dir, fail-closed on owner/type/symlink mismatch, ancestor `--x` traverse ACL, stale `tpm.sock` unlink - emitting the path-free `PrepareSwtpmDir` audit op. | Holds the per-VM TPM 2.0 NVRAM + EK seed. **Wiping it looks like device tampering to any IdP** (Entra ID, Intune, Bitlocker-style policies) and forces re-enrollment. Never zero it casually. The per-VM state root is `3770` (setgid **+ sticky**) so a non-owner role UID cannot rename/replace the `swtpm/` entry; an identity-bound, root-owned marker at `/var/lib/d2b/swtpm-markers/<vm>` makes a *previously-provisioned-then-missing/replaced* dir **fail the VM start closed** (`previously-provisioned-swtpm-state-missing`) rather than silently re-creating an empty TPM. The state directory's ACLs are asserted by `tests/unit/smoke/smoke-eval-tpm.nix`; the broker hardening by `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` tests. |
| USBIP passthrough                   | `nixos-modules/components/usbip.nix` (eval-time gating) + broker `UsbipBindFirewallRule` + `SpawnRunner` (per-busid attach process supervised by `d2bd`) | Eval-time gating still scopes attach to opted-in envs (validated by `tests/unit/nix/cases/usbip-gating.nix`). At runtime, attach/detach runs through the broker - there is no per-env `d2b-sys-<env>-usbipd-*` socket. Misrouted attaches expose a YubiKey to the wrong env. |
| GPU sidecar (graphics VMs)          | `nixos-modules/components/graphics.nix` + broker `SpawnRunner` for cloud-hypervisor on graphics VMs; pidfd handed back via `OpenPidfd` and supervised by `d2bd` | Graphics VMs run cloud-hypervisor with the GPU device attached. Restarting `d2bd` no longer terminates CH - pidfd handoff means the child outlives a daemon reconnect - but the broker spawn path is the only audited place CH is launched. Bypassing it breaks the audit trail. Validate the evaluated graphics shape with `tests/unit/nix/cases/video-contract.nix`. |
| Video sidecar (graphics VMs)        | `nixos-modules/components/video/guest.nix`, `nixos-modules/processes-json.nix`, `pkgs/vhost-user-video/`, `packages/d2b-host/src/video_argv.rs`, broker `SpawnRunner{role: Video}` | `graphics.videoSidecar = true` is an explicit opt-in H264 decode path: guest `virtio_media` + patched Cloud Hypervisor `--vhost-user-media` + patched crosvm `device video-decoder --backend vaapi`. There is no per-VM video systemd unit, no stock crosvm/CH fallback, and no free-form video extra args. The video runner MUST use the dedicated `d2b-<vm>-video` principal, not `d2b-<vm>-gpu`, so broker/activation ACLs can deny host Wayland/PipeWire/Pulse sockets to video without breaking GPU cross-domain. The broker masks `/dev` for the video runner and exposes only the declared device allowlist: default `/dev/dri/renderD128`, plus `/dev/nvidiactl`, `/dev/nvidia0`, and `/dev/nvidia-uvm` only when `graphics.videoNvidiaDecode = true`. `virtio_media` is a guest module, not a host `/proc/modules` preflight requirement. Firefox/VA-API uses the separate experimental `graphics.virglVideo` GPU path; it is default-off and must not be treated as stable video-sidecar coverage. Validate evaluated shape with `tests/unit/nix/cases/video-contract.nix`; rendered argv and sandbox coverage lives in `packages/d2b-contract-tests/tests/minijail_swtpm_video.rs` and is advisory until the fixture lane is enabled. |
| UI color contract / niri backend    | `nixos-modules/ui-colors.nix`, `nixos-modules/niri-vm-borders.nix`, `docs/reference/ui-colors.{md,json}`, `tests/unit/nix/cases/niri-vm-borders.nix`, and sibling consumers such as `vicondoa/d2b-wlcontrol` | The compositor-agnostic `d2b.site.ui` / `d2b.envs.<env>.ui` / `d2b.vms.<vm>.ui` color model is the source of truth for host/env/VM/state colors. Generated `/etc/d2b/ui-colors.json` and `/etc/d2b/ui-colors.css` are public presentation metadata, not authz or policy inputs. Niri-specific settings belong only under `d2b.site.ui.compositors.niri`; do not add compositor-specific color source options. Keep the JSON schema, reference docs, GTK CSS `@define-color` names, and nix-unit artifact-shape tests in sync. Downstream tools must fail visibly but remain usable when the artifact is missing or malformed, without reading root-owned d2b state directly. |
| ComponentSession capability boundary | `packages/d2b-contracts/src/v3/component_session.rs`, `packages/d2b-session/`, `packages/d2b-session-unix/` | Authenticated transport evidence and attachment credits are consumed into a private single session owner; do not add a clone/accessor that lets callers reuse admission evidence. **`SessionAuthority` is sealed** by a private supertrait in a private module (`admission.rs`), so no crate outside `d2b-session` can implement it - that seal is load-bearing, because a foreign authority implementation is a direct path to minting a genuine admission. Prove exact Zone equality before every capability mint, and never expose a store path, socket, or handle through the session. These crates are tested but deliberately unwired from production listeners until the full authenticated registration path lands. |
| Zone message bus boundary | `packages/d2b-bus/src/{router,registry,authorization,streams,operations}.rs`, `packages/d2b-resource-api/src/adapter.rs` | Registration consumes the single-owner capability admission; comparing a clonable token is insufficient. Every route is exact, subject-bound, revision-bound, and Zone-checked before minting authority. There is no wildcard pub/sub and no direct store handle. `UnregisteredBusAdapter` is a deliberate unreachable seam and must remain unregistered until authenticated ComponentSession, the Zone bus, and Zone registration land together. |
| Authoritative subject resolution | `packages/d2b-bus/src/router.rs` (`ZoneRegistrar`), `packages/d2b-session-unix/src/subject.rs` | `ZoneRegistrar` **exclusively owns and consumes** subject resolution: a peer is mapped to a subject from registrar-private state using verified peer evidence. There is no public subject-configuration type and no raw-claim registration path, and there must not be one - caller-supplied `subject_ref`/`subject_uid` are exactly how a component would name itself something it is not. Production currently fails closed because no authoritative resolver is wired, which is the intended state until the Zone runtime supplies one; do not "fix" that by accepting claims from the caller. This boundary moved several times before it closed, each time by reappearing as a public constructor or registrar mutator somewhere the guard was not looking, so it is enforced by the type-based mint-surface inventory and a compile-fail fixture rather than by convention. |
| Capability mint surface allowlist | `packages/d2b-bus/tests/public_mint_surface.rs`, its four approved API snapshots, the mutations under `packages/d2b-bus/tests/ui/`, and the capability definitions in `packages/{d2b-bus,d2b-session,d2b-session-unix}/src/` | The **enforcing compiler leg** uses stable trait-solver ambiguity assertions in the defining crates. It rejects the enumerated `Clone`, `Copy`, `Default`, and `From` implementations for `ComponentSessionAdmission`, `VerifiedUnixPeer`, `SessionAcceptor<C>`, and `AuthenticatedComponentSession<C>` in every compiled configuration. Generic assertions catch unconditional blanket implementations; separate assertions cover `C = ()` and the workspace's `C = ComponentSessionAdmission` uses. They do not enumerate every bounded or downstream `From<X>` implementation, so private construction fields, sealed traits, instance identity, and consumed authority remain the primary boundary. The external-seals tests require `error[E0283]` plus `CapabilityMustNotImplementCloneCopyDefaultOrFrom`; fabrication fixtures require the construction diagnostic that proves private fields remain closed. The **best-effort source leg** inventories explicit workspace impl and derive forms and compares them with `approved-capability-trait-impls.txt`. Module aliases and module-level globs resolve monotonically over a finite universe: parsed alias names form the binding universe, declared local module paths form the only target universe, explicit bindings shadow glob imports, conflicting glob results are ambiguous, and separate target/visibility and taint budgets bound the two fixed points. Capability propagation resolves every glob target through the completed module-alias fixed point, including renamed targets; a multiple-target result is ambiguous and fails closed. A target can never acquire a path outside that finite module set, so glob cycles cannot grow indefinitely. Capability relevance propagates through resolved aliases to every descendant module containing a discovered capability binding. Unknown glob destinations taint their importing module; that taint propagates through later glob re-exports and makes otherwise unclassified impl self types fail closed. Roots matching Cargo-declared dependency names are classified as external and import no local capability binding, so ordinary dependency globs remain accepted. Unresolved alias bindings imported by a glob remain tainted bindings and fail closed when used as an impl prefix. Block-local globs and impls carry lexical scope identities. The scanner accepts a same-scope direct module alias only when its target is resolved and no capability or tainted descendant is reachable; capability-relevant, ambiguous, unresolved, or otherwise unmodelled block-local glob aliases fail closed. This is intentionally not a claim of complete Rust glob resolution. Regression fixtures pin the terminating `a`/`b` glob cycle with explicit shadowing, nested re-export through glob, rejecting direct and grouped renamed glob targets, unresolved and two-hop glob taint, rejecting direct and grouped block-local capability globs, and accepting non-capability block-local and renamed-target globs. Existing direct, renamed, chained, cfg, raw-identifier, path-loaded, symlink, attribute, and duplicate-logical-module fixtures remain covered. The source leg also fails closed on generic or cfg-gated declared type aliases, cfg-gated renamed imports, unsupported aliases, lexically scoped capability aliases, unresolvable external modules, missing selected module files, and unrecognised module attributes. It does not perform general Rust name resolution, macro expansion, or `include!` expansion, and implementations outside the scanned workspace remain outside its claim. Approved snapshots retain rendered signatures for exact comparison; failure output uses fixed operation or syntax labels, package or crate identity, exit status, and crate-relative logical locations. Raw Cargo or rustdoc stderr, signature tokens, source text, attribute tokens, absolute scratch paths, and attacker-authored path literals are not emitted. The separate capability API inventory still propagates from fixed capability and claim identities through private field types. Widening any compiler seal or approved snapshot is a deliberate trust-boundary change requiring a stated reason. |
| Resource controller effects boundary | `packages/d2b-controller-toolkit/src/{runner,queue,context,result,owner_hints}.rs`, `packages/d2b-core-controller/src/{hints,dependencies,owner_reconcile}.rs` | Controller and core-reconciliation engines are test-only and unwired from the absent production store/watch dispatcher. An EffectPort call is permitted only after durable resource commit and consumption of the matching `CommittedRevisionProof`; abort, conflict, stale proof, or restart ambiguity cannot release an effect. Preserve per-resource single flight, bounded fair admission, deterministic owner/dependency propagation, and restart-safe idempotency when wiring the production path. |
| Unsafe-local provider, launcher, and persistent-shell helper | `nixos-modules/options-realms-workloads.nix`, `nixos-modules/unsafe-local-workloads-json.nix`, `packages/d2b-core/src/unsafe_local_workloads.rs`, `packages/d2b-contracts/src/unsafe_local_wire.rs`, `packages/d2b-unsafe-local-helper/src/{shell_runtime,shell_supervisor,shell_socket,output_ring,tty_exec}.rs`, and `docs/reference/unsafe-local-provider.md` | `unsafe-local` is explicit and default-denied. It runs only as the exact authenticated requesting uid and provides no isolation boundary. Public metadata never carries configured argv or shell policy; those come only from the integrity-pinned private bundle. A persistent-shell supervisor in a verified transient USER scope - not the reconnectable helper or d2bd - owns the login-shell PTY, bounded merged-output ring, attachment, and private same-UID listener. Ledger adoption preserves ambiguous sessions as degraded; teardown closes the PTY and signals only the exact re-verified scope. The helper-wide ring reservation is bounded, terminal responses transfer exactly one CLOEXEC stream fd, and shell names, supervisor ids, paths, environment, process/unit identity, and bytes stay out of Debug/errors/audit. Do not add cross-uid execution, a direct compositor fallback, VM state/network/device semantics, a root service, per-VM unit, broker op, free-form shell command, or broad same-UID cleanup. |
| Manifest contract                   | `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix`               | Version-pinned via `manifestVersion`. Adding, removing, or renaming a per-VM field requires bumping the version, updating the schema, and noting it in the CHANGELOG. The `static.sh` md↔json drift gate catches partial updates. |
| Manifest bundle - private artifacts | `docs/reference/manifest-bundle.md` + `docs/reference/schemas/v2/*.json` + `packages/d2b-core/src/{bundle,host,processes,privileges,closures,minijail_profile}.rs` + `nixos-modules/{bundle,bundle-artifacts,host-json,processes-json,privileges-json,closures-json,minijail-profiles}.nix` + `packages/xtask/src/main.rs` (`gen-schemas`) | Sensitive bundle artifacts install at `root:d2bd` 0640 and ground every broker/sandbox/runner behaviour. `d2b-core` DTOs are canonical; `d2b._bundle` is the typed internal artifact table that owns JSON data, install names, classifications, and `/etc/d2b` materialization for every bundle artifact. Add new bundle artifacts through `nixos-modules/bundle-artifacts.nix` instead of hand-writing parallel install logic in each emitter. Committed schemas under `docs/reference/schemas/v2/` ARE the contract and the `tests/unit/gates/drift-check.sh` gate enforces `xtask gen-schemas` + `git diff --exit-code` through `make test-drift`. Breaking the schema without an intentional `bundleVersion`/`schemaVersion` bump silently breaks every downstream consumer. |
| Control plane - `d2bd` + `d2b-priv-broker` | `packages/d2b-contracts/**` + `packages/d2b-core/**` + `packages/d2bd/**` + `packages/d2b-priv-broker/**` (sibling workspace; `unsafe_code = "deny"` with quarantined `src/sys.rs` for fd-passing FFI) + `packages/d2b/**` + `docs/reference/{cli-contract,daemon-api,error-codes,privileges}.md` + the daemon Layer-1 gate set in `tests/static.sh` | The **only** persistent root surfaces the framework declares. `d2b-priv-broker.socket` is socket-activated: systemd creates/binds/listens/sets-ACL before the broker starts; the broker adopts fd 3 via `SD_LISTEN_FDS` and MUST NOT self-bind, self-fchmod, or self-fchown when `SD_LISTEN_FDS=1`. `d2bd.service` carries `Wants=d2b-priv-broker.socket` (not `Requires=`) so the daemon keeps serving while the broker is idle. The broker reloads the current bundle resolver per accepted request so it does not dispatch stale runner intents after a switch. The broker drops to the `d2bd` group and uses `SO_PEERCRED` at accept time for authz (launcher / admin / deny). Every host mutation flows through a typed broker op (cgroup v2 delegation, TAP/bridge lifecycle, `ApplyNftables`, `ApplyNmUnmanaged`, `ApplySysctl`, `UpdateHostsFile`, `ModprobeIfAllowed`, `UsbipBindFirewallRule`, `SpawnRunner`, `OpenPidfd`) and is recorded as an `OpAuditRecord` in `/var/lib/d2b/audit/broker-<utc-date>.jsonl` (root-owned `0640 root:d2bd`, append-only `O_APPEND`, daily rotation, 14-day default retention overridable via `d2b.site.audit.retentionDays`). Relevant enforcing coverage includes `tests/unit/nix/cases/broker-socket-activation.nix`, `tests/unit/nix/cases/broker-caps.nix`, and daemon startup integration tests under `packages/d2bd/tests/`. The legacy-unit policy lives in `packages/d2b-contract-tests/tests/policy_units.rs` and remains advisory until the fixture lane is enabled. See [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md). |
| Storage lifecycle / restart / synchronization | Planned generated contracts in `d2b-core::{storage,process_restart,sync}` + Nix emitters, broker storage/sync ops, daemon lifecycle DAG integration, and docs [ADR 0034](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md) / [`docs/explanation/storage-lifecycle.md`](./docs/explanation/storage-lifecycle.md) | Managed paths, restart adoption, locks, leases, cleanup, and degraded-state reporting are control-plane contracts. Normal daemon restarts are continuation events: do not broad-sweep `/run/d2b`; first re-discover adoptable runners from declared cgroup leaves, open fresh pidfds, verify identity, and quarantine/degrade ambiguity. Pidfds are not persisted. New advisory locks use OFD locks with `O_CLOEXEC`, explicit fd transfer only, and total acquisition order. The broker resolves storage/lock mutations from opaque bundle ids through anchored `openat2`/fd-relative path walking; daemon-owned ledgers are diagnostics, never repair authority. |
| Eval-time assertions                | `nixos-modules/assertions.nix`                                                          | These are the framework's contract with consumers. Loosening one silently turns a previously-rejected misconfig into runtime breakage. New assertions need a matching case in `tests/unit/nix/cases/assertions.nix`. |
| Guest-control exec session table    | `packages/d2bd/src/{exec_session,exec_session_real}.rs`, `run_exec_owner` in `packages/d2bd/src/lib.rs`, `packages/d2b/src/exec_client.rs`, `packages/d2b-contracts/src/public_wire.rs` (`ExecOp`/`ExecOpResponse`) | Arbitrary `d2b vm exec` is **admin-only**; configured `d2b launch` local-VM items may use the same detached guest-control backend with launcher authority because argv is resolved exclusively from the hash-verified private bundle. Both run through `d2bd` plus authenticated guest-control vsock to `guestd`. Attached exec uses the daemon's in-process **session table**: per-session workers own one authenticated guest-control client and proxy typed exec ops. **guestd runs every exec as the VM's workload user (`ssh.user`) inside a real PAM login session (`systemd-run --property=PAMName=login --uid=<user>`) - never as root; the wire `user` field is ignored and the target user is host-fixed, bare `argv[0]` is resolved by the workload user's login `PATH`, and each attached exec runs in a process-unique named transient unit (`d2b-exec-<…>.service`) that teardown stops via `systemctl kill` so a quiet command cannot outlive owner-disconnect, cancel, or the runtime ceiling. Operators elevate with `sudo` inside the session.** Detached non-TTY exec is enabled with `d2b vm exec -d <vm> -- <cmd>` and managed through VM-first verbs (`d2b vm exec <vm> list`, `logs <id>`, `status <id>`, `kill <id>`); command forms always require `--`, so those verb words remain valid VM names. Detached jobs and configured local-VM launches also run as the workload user, never root: the root detached runner only owns trusted slot/log files, re-validates the non-root uid before spawning the workload unit, and fails terminally rather than falling back to direct root execution. Guestd reconciles detached runner/workload units on startup, cleans orphaned workloads, and runs a periodic reaper for terminal records and retained logs; `kill` maps to idempotent two-phase `ExecCancel` (SIGTERM/grace/SIGKILL). There is **no per-VM systemd unit, no new broker op, and no SSH** - the guest owns the PTY; the host only flips termios for attached TTY via an RAII raw-mode guard restored on every exit/error/panic. The admin `SO_PEERCRED` check runs before arbitrary exec session setup; configured launch instead requires local launcher/admin authority and a trusted configured item. Old/non-guest-control generations fail closed (exit `70`) with no proxy and no SSH fallback. Session-table caps (global/per-UID/per-VM), detached slot/log quotas, and rate limits are enforced before connect/auth or create. Attached audit emits one redacted kind=critical session-establishment event (vm/peer_uid/tty); detached create/kill daemon audit carries only vm/peer_uid/action/result/exec_id, while configured-launch audit adds target/item/operation correlation without execution details. Opaque session handles, argv, stdio, env, cwd, and paths never reach any Debug/trace/audit/metric surface. Validate with the `exec_session`/`exec_client` hermetic test matrices. |
| Unsafe-local persistent shells | `packages/d2bd/src/{workload_dispatch,unsafe_local_helper,unsafe_local_terminal,shell_backend}.rs`, shell owner dispatch in `packages/d2bd/src/lib.rs`, `packages/d2b-unsafe-local-helper/src/{shell_runtime,shell_supervisor}.rs`, and `tests/host-integration/unsafe-local-helper.nix` | `d2b shell` remains **admin-only** for every provider. Unsafe-local target identity and `defaultName`/`maxSessions` come only from the hash-verified private bundle; public `ShellOp` keeps protocol v3 and carries no policy, uid, argv, env, cwd, or path. The daemon dispatches helper protocol v2 to the exact `SO_PEERCRED` uid, validates exactly one connected CLOEXEC stream fd, and multiplexes terminal protocol v1 behind a fresh opaque public handle. Disconnect/`CloseAttach` detach but never kill; `Kill` targets only the helper-verified transient user scope. Shells survive CLI, daemon, and helper reconnects while that scope and the non-lingering user manager live. User logout ends them by design. User scopes provide lifecycle ownership, **not containment from other processes with the same host uid**. There is no root unit, broker op, per-VM service, SSH path, host-shell fallback, direct-compositor fallback, or automatic replay after an ambiguous daemon timeout. Never log/audit/label shell names, supervisor ids, public handles, terminal bytes, helper diagnostics, PIDs, unit names, argv, env, cwd, or paths; audit may use configured target/peer uid and fixed digests, while metrics use closed provider/component/operation/outcome/error labels. |
| Lifecycle permission group          | `nixos-modules/host-users.nix`                                                          | Membership in `d2b` + `SO_PEERCRED` at `public.sock` accept time is the **only** lifecycle authorisation surface. There is no polkit allowlist; wiring anything else into the group inverts the threat model. **Exception:** the guarded `ExecStop` shutdown hook runs as uid 0 and receives the narrow `HostShutdown` role, which is permitted only for `vmStop` during host-shutdown teardown (see `packages/d2bd/src/admission.rs`). This exception is scoped strictly: all other admin-only operations (exec, USB attach, key rotation, host prepare, audit export) are denied for this role. The daemon-restart continuation guard is preserved: `Restart=on-failure` restarts never receive `HostShutdown` treatment because the restarting daemon re-adopts runners and the shutdown hook only runs under systemd stop with a live `stopping` system state check. |
| SSH key generation / rotation       | `nixos-modules/host-keys.nix`, `host-activation.nix`                                    | The framework owns `${cfg.site.keysDir}/<vm>_ed25519`. `d2b keys rotate` MUST NOT touch consumer-supplied keys. |
| virtiofsd sandbox model             | `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles), `packages/d2b-priv-broker/src/sys.rs` (`clone3_spawn_runner` user-NS path), `nixos-modules/processes-json.nix` (argv emit) | virtiofsd profiles MUST declare zero host capabilities (`capabilities = []`), `requiresStartRoot = false`, and a `userNamespace` block mapping in-NS UID/GID 0 to the per-share principal. Normal VM shares map to `d2b-<vm>-runner`; the guest-control token share (`d2b-gctl`) maps to the narrower `d2b-<vm>-gctlfs` principal. The broker pre-establishes the user namespace via `clone3(CLONE_NEWUSER)` + `pipe2` sync + `/proc/<pid>/uid_map` writes BEFORE virtiofsd's first instruction runs. virtiofsd argv MUST include `--sandbox=chroot --inode-file-handles=never` and `--readonly` for every `readOnly` share (`ro-store`, `d2b-gctl`). Reintroducing host caps, `requiresStartRoot=true`, or `--sandbox=namespace` violates [ADR 0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md). Rendered profile and argv coverage lives in `packages/d2b-contract-tests/tests/minijail_roles.rs` and is advisory until the fixture lane is enabled. |

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
  See [Versioning & changelog](#versioning--changelog).
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

## cgroup slice naming + ownership-marker conventions

The privileged broker's host-prepare dispatch (see the Control plane
row above) carries two operational conventions that ground every
broker op mutating host state.

### cgroup slice naming

- Single canonical slice: **`/sys/fs/cgroup/d2b.slice`** (no
  `system-` prefix, no `d2b-launcher.slice` parent). The broker
  creates it on `host prepare --apply` if absent.
- Per-VM directories live one level below the slice:
  `d2b.slice/<vm>/<role>/`. The VM layer is **process-free**; only
  the per-role leaves hold processes.
- Delegation: the broker `fchown`s the delegated subtree (the
  `d2b.slice` directory and every descendant) to the `d2bd`
  system user. The host cgroup root is never chowned.
- Forbidden surfaces: writing `cpuset.cpus.partition` on
  d2b-owned cgroups (the cgroup v2 root and other ancestors
  are out of scope; d2b never reads/writes them), threaded
  cgroups, `cgroup.kill` on `d2b.slice` or any ancestor of
  a daemon-owned leaf, and **Phase B (post-delegation) runtime
  mutation while running as uid 0** (Phase A privileged setup -
  `+controllers` cascade, slice/leaf `mkdir`, `fchown` to
  `d2bd`'s uid/gid - legitimately runs as root per ADR 0011
  Decision item 2; the uid != 0 invariant applies to the
  steady-state cgroup code path after privilege drop). See
  [`docs/reference/cgroup-delegation.md`](./docs/reference/cgroup-delegation.md)
  and ADR 0011 for the algorithm + audit shape.

### Ownership-marker conventions

The broker writes its host mutations inside greppable ownership
markers so foreign-rule preservation can be enforced fail-closed:

| Surface | Marker shape |
| --- | --- |
| nftables (`inet d2b` table) | every rule + chain carries `comment "d2b managed: <ownership-id>"`; foreign tables are never flushed |
| `/etc/hosts` | block delimited by `# d2b-managed begin` and `# d2b-managed end`; foreign lines outside the block are byte-preserved |
| NetworkManager unmanaged config | `/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf`, contents delimited by `# d2b-managed begin` / `# d2b-managed end` |
| systemd-networkd | detection-only; coexistence requires an operator-shipped configured-unmanaged file matching the `d2b-`/`d2bv-` prefix (no d2b write) |

Discovering a foreign ownership marker where d2b expects its own
is fail-closed (`path-safety-violation`,
`nm-managed-foreign-conflict`, `foreign-nft-rule-preserved`). See
[`docs/explanation/host-prepare.md`](./docs/explanation/host-prepare.md)
§ "NetworkManager / systemd-networkd coexistence" and ADR 0013 for
the rationale.

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
  is advisory until the fixture lane is enabled
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

- [docs/adr/0015-daemon-only-clean-break.md](./docs/adr/0015-daemon-only-clean-break.md) -
  **the binding architectural decision** for the daemon-only
  end-state: `d2bd` + `d2b-priv-broker` are the only
  persistent root surfaces.
- [docs/adr/0017-no-bash-fallbacks-invariant.md](./docs/adr/0017-no-bash-fallbacks-invariant.md) -
  the Rust CLI never invokes bash; CI gates enforce no new
  `Command::new("bash")` sites.
- [docs/adr/0018-microvm-nix-removal.md](./docs/adr/0018-microvm-nix-removal.md) -
  d2b owns its per-VM substrate via `vm-options.nix` +
  `vm-evaluator.nix`; the `microvm.nix` flake input is gone.
- [docs/adr/0021-broker-user-namespace-for-virtiofsd.md](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md) -
  broker pre-establishes a single-entry user namespace via
  `clone3(CLONE_NEWUSER)` so virtiofsd runs fake-root inside the
  NS while exposing **zero** host capabilities. Any change to the
  virtiofsd minijail profile or argv shape MUST preserve this
  contract.
- [docs/adr/0031-bare-command-and-detached-exec.md](./docs/adr/0031-bare-command-and-detached-exec.md) -
  bare command-name exec resolution and enabled detached
  workload-user exec with VM-first management verbs.
- [docs/adr/0032-d2b-v2-constellation-control-plane.md](./docs/adr/0032-d2b-v2-constellation-control-plane.md) -
  evolves `d2bd` into a transport-neutral constellation
  daemon. **Load-bearing invariant:** the host daemon/broker hold
  **no** realm relay/provider credentials, remote node registries,
  or realm audit (those live inside a per-realm gateway guest); and
  **relay identity is not local auth** - relay credentials
  authenticate relay/transport access only, are never mapped to a local
  lifecycle role, and `SO_PEERCRED` + `d2b` group membership remains
  the sole local lifecycle authz surface.
- [docs/adr/0034-storage-lifecycle-restart-and-synchronization.md](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md) -
  selected design for generated storage, restart/adoption, and
  synchronization contracts. **Load-bearing invariant:** normal daemon
  restarts are continuation events; recover/adopt/quarantine before
  cleanup, never persist pidfd authority, and route host storage/lock
  mutation through broker-resolved opaque ids.
- [README.md](./README.md) - consumer-facing intro, install,
  manual integration walkthrough.
- [CHANGELOG.md](./CHANGELOG.md) - Keep-a-Changelog, entries
  accumulate under `## Unreleased` until a tag cuts them.
- [SECURITY.md](./SECURITY.md) - disclosure path + scope.
- [docs/explanation/design.md](./docs/explanation/design.md) -
  threat model, defenses-in-depth list, *Why not X* FAQ.
- [docs/explanation/daemon-lifecycle.md](./docs/explanation/daemon-lifecycle.md) -
  daemon DAG executor, pidfd handoff, supervisor reconciliation.
- [docs/reference/privileges.md](./docs/reference/privileges.md) -
  authoritative broker op catalogue.
- [docs/reference/daemon-api.md](./docs/reference/daemon-api.md) -
  `public.sock` wire surface, audit format, retention.
- [docs/reference/manifest-schema.md](./docs/reference/manifest-schema.md)
  + [docs/reference/manifest-schema.json](./docs/reference/manifest-schema.json) -
  the manifest contract.
- [docs/reference/cli-contract.md](./docs/reference/cli-contract.md) -
  CLI lifecycle FSM, signal semantics, exit codes, JSON vs human
  output.
- [docs/reference/realm-policy.md](./docs/reference/realm-policy.md) -
  host-resident vs gateway-backed realm policy, default-deny
  cross-realm behavior, and `d2b realm list` / `inspect`
  inspection surfaces.
- [docs/reference/constellation-observability.md](./docs/reference/constellation-observability.md) -
  bounded `d2b op inspect`, TraceContext handling, degraded partial
  results, and telemetry redaction/cardinality constraints.
- [docs/how-to/configure-work-gateway.md](./docs/how-to/configure-work-gateway.md) -
  configure a dedicated work/provider realm gateway and verify the
  default-deny boundary.
- [docs/how-to/migrate-d2b-v0-to-v1.md](./docs/how-to/migrate-d2b-v0-to-v1.md) -
  consumer migration guide for v0.x → v1.0.
- [docs/how-to/migrate-d2b-v1-0-to-v1-1.md](./docs/how-to/migrate-d2b-v1-0-to-v1-1.md) -
  consumer migration guide for v1.0 → v1.1.
- [docs/how-to/migrate-d2b-v1-1-to-v1-2.md](./docs/how-to/migrate-d2b-v1-1-to-v1-2.md) -
  consumer migration guide for v1.1 → v1.2, including the
  canonical `d2b` lifecycle group rename.
- [docs/how-to/migrating-from-microvm.md](./docs/how-to/migrating-from-microvm.md) -
  option mapping for users coming from raw microvm.nix
  (scoped to new installs).
- [tests/README.md](./tests/README.md) - full test layering,
  including Layer-2 integration tests.
- [LICENSE](./LICENSE) - Apache-2.0.
