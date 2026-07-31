# Development workflow

How work is organised, validated, and landed: worktrees for parallel agents,
the stacked-PR shape for large waves, the commit-then-validate rule, and the
disk hygiene contract that keeps concurrent worktrees from filling the host.

The binding one-line rules are in [`../../AGENTS.md`](../../AGENTS.md) under
"Development workflow". This file carries the detail and the rationale.

## Worktrees for parallel agents

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

## Finish-of-work invariant: merge back into the primary clone

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

## Stacked PR workflow for large waves

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

## Screenshot and visual artifact hygiene

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

## Local host validation after updating d2b

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

## Integrator-prep-first pattern (W3 onwards)

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

## Edit → commit → validate

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

## "Existing code is canon"

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

## Landing changes (PR workflow)

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
- **Never clear `RUSTC_WRAPPER` to make a command work.** Every
  `rustc-wrapper` line points at a repo-local `.cargo/rustc-wrapper.sh`
  that uses sccache when it is on PATH and plain rustc when it is not, so
  no environment needs the variable cleared in order to build. Naming
  `sccache` directly used to make it a hard requirement, and the resulting
  `RUSTC_WRAPPER=""` workaround spread into environments that *did* have
  sccache and silently disabled the compiler cache. Clearing it is reserved
  for a deliberate choice: running uncached (`D2B_NO_SCCACHE=1`, or CI
  without `D2B_CI_SCCACHE=1`), or the compile-fail seal fixtures, which
  clear every wrapper spelling because a caching wrapper that exits nonzero
  under concurrent cargo invocations is indistinguishable from the fixture
  failing for the wrong reason.
- Tests that shell out to `cargo` (the capability-seal guards in
  `packages/d2b-bus/` and `packages/d2b-controller-toolkit/`) cache their
  scratch trees between runs, keyed on a hash of `rustc -vV`. Compiled
  artifacts are not portable across compiler versions, and the gate's
  pinned toolchain routinely differs from a dev shell's, so an unkeyed
  cache lets one poison the other. Those caches live under `.scratch/` and
  are several GB per worktree; delete that directory to reclaim the space.
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

