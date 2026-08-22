# Development workflow

How work is organized, validated, reviewed, and landed for
[`../../AGENTS.md`](../../AGENTS.md). That file is the single operational
authority; this document carries the detail and rationale.

## Worktrees and task routing

Use a new isolated worktree by default for feature work. A worktree is also
required for each concurrent worker. Start from an owned feature or
integration branch, never directly from protected `main` or `v3`.

Every code change enters through Compound Engineering:

- A clear bounded change uses `ce-work` on the smallest sufficient route.
- An open-ended bug uses `ce-debug`, then `ce-work` after the failure is
  understood. Use `ce-plan` if scope or product intent remains unclear.
- Larger or product-ambiguous work uses `ce-brainstorm`, `ce-plan`, then
  `ce-work`.

Use these thresholds rather than treating every task as a heavyweight plan:

| Route | Signals |
| --- | --- |
| Clear bounded | One or two files, known behavior, no product decision, and a focused validation path. |
| Open-ended bug | A regression, failing test, error report, or unclear failure path that needs diagnosis before editing. |
| Larger or ambiguous | Cross-cutting work, roughly ten or more files, architecture/auth/migration impact, or unresolved product intent. |

Useful parallelism is limited to genuinely disjoint units. Different files do
not prove independence when units share APIs, generated artifacts, schemas,
lockfiles, stateful tools, or runtime resources. When overlap is uncertain,
work serially. Persisted prose uses normal repository style even when Caveman
keeps transient communication concise.

The d2b profile is:

```text
ce-work
ce-work mode:return-to-caller <plan-path>
ce-code-review mode:agent
ce-commit-push-pr branding:off babysit:off
ce-babysit-pr posture:target
```

Bare `ce-work` owns a bounded standalone change. Caller mode is only for an
outer workflow that supplies an implementation-ready plan and owns the
shipping tail.

Ponytail supplies minimal safe implementation discipline. Caveman is for
transient communication only. Advanced planning, orchestration, and review
prefer `gpt-5.6-sol` with xhigh reasoning and long context (`long_context`);
implementation prefers `gpt-5.6-luna` with xhigh reasoning. If unavailable,
use the strongest
native role-equivalent model and record the substitution only in a transient
handoff. Shipped prose never attributes a model or tool.

```bash
# From the primary clone, one worktree per concurrent scope:
git worktree add -b phase-<name> ../d2b-<name> <owned-feature-or-integration-branch>
```

Each agent commits inside its worktree on its `phase-<name>` branch. When scopes
are disjoint (different files or non-overlapping regions), the integrator
integrates them into the owned feature/integration branch:

```bash
git switch <owned-feature-or-integration-branch>
git merge --no-ff phase-a phase-b phase-c
```

If branches touch the same lines, use a sequential merge with conflict
resolution - octopus requires cleanly disjoint scopes. Do not merge a slice
directly into protected `main` or `v3`; the owned branch lands through the
required pull request flow.

## Finish-of-work invariant: merge back into the primary clone

A worktree is a workspace, not a destination. When an agent's scope is done -
implementation and tests green - the integrator merges
the slice into the owned feature/integration branch in the **primary clone
(`projects/d2b`)** before declaring the slice integrated. Finished side branch
work still "awaits integration", which the integrator owns. The owned branch is
then landed in protected `main` or `v3` only through a pull request.

Concretely, the agent that owns a worktree:

1. Verifies green on the worktree (`make check`, or the relevant focused Bazel
   labels).
2. From the primary clone (`/home/paydro/projects/d2b`), fast-forwards (or
   octopus-merges, per the rules above) the worktree's `phase-<name>` branch
   into the owned feature/integration branch.
3. If there is unrelated dirty WIP in the primary clone (operator
   was editing in place), stash it, do the merge, pop the stash,
   resolve any textual conflicts in a way that preserves both sets
   of changes, then leave the operator's WIP unstaged so they can
   commit it on their own terms.
4. Audits sibling worktrees (`git worktree list`) for branches
   whose tip is unmerged but represents abandoned/superseded work;
   flag those for the operator rather than silently dropping them.

Only after the merge lands does the agent call `task_complete`.


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

Do **not** commit unredacted screenshots to the repository. Reviewers may
inspect screenshots as part of validation evidence; the same redaction rules
apply when attaching screenshots to PR bodies.
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


## Edit -> commit -> validate

Commit before running `make check` / the smoke evals. Two reasons:

1. Untracked files are invisible to `nix flake check` (and to any
   eval that follows the same code path). Forgetting to `git add` a
   new module is the #1 "why doesn't my change apply?" pitfall.
2. Consumer hosts that vendor d2b tend to ship auto-backup
   tooling that catch-all-commits any dirty tree. That's a
   consumer-side concern, but the habit of committing-then-building
   is the right one to carry into framework work too.

## Existing code is canon

When the spec, plan, README, or any reference doc disagrees with the
**code that is actually committed and passing tests**, the code
wins. Document the drift, don't silently re-align the code to the
prose.

- Mention the drift in the commit message body
  (e.g. `Spec correction: docs/reference/cli-contract.md claimed
  exit code 3 for "VM not found"; code returns 2. Kept code.`).

This rule applies to AGENTS.md too: if you change a load-bearing
behaviour described here, update this file in the same commit.

## Reviewed-head PR lifecycle

Every code diff gets an independent review in a separate clean context.
`ce-code-review` is report-only; the repository-owned caller applies
actionable fixes. After any review fix, CI fix, push, base update, or other
head-changing update, validate the new head and obtain fresh independent
review. Missing review evidence fails closed to fresh review. No actionable
finding remains at merge.

The Bazel graph is the single enforcing Layer-1 authority. Bare local
`make check` uses BuildBuddy for eligible actions. Protected `v3` CI runs
credential-bearing remote suites for eligible actions and keeps Nix, fixture,
hardware, and other local-only lanes local and credential-free.
Make remains a compatibility surface, not a scheduler. See [Bazel and
BuildBuddy](../reference/bazel-buildbuddy.md) for execution, cache, failure,
credential, and update contracts.

Review evidence is bound to the repository, PR, review head OID, observed base
ref and OID, and verdict. It is not reusable after the head changes. A review
that cannot prove those bindings is missing evidence, not a pass.

Use `ce-babysit-pr` to watch feedback, required checks, and head currency.
Immediately before merge, refresh the current reviewed head, required checks,
feedback, mergeability, and observed base. Reconcile any new feedback or
check result before proceeding.

After all required checks and feedback settle, merge with a normal squash and
an expected-head guard. Do not use admin, auto-merge, bypass, or a merge
queue. If a merge result is ambiguous, inspect current PR state and reconcile
before retrying. The workflow refreshes the base on a best-effort basis and
accepts the narrow non-atomic base race under current non-strict branch
settings; it does not change GitHub settings or claim atomic base binding.

`main` and `v3` remain protected: changes land through a pull request, not a
direct push. PR bodies record the change, validation evidence, and substantive
review outcomes only. Never include AI, tool, or model attribution.

## Gas City boundary

Gas City is separate managed contributor infrastructure. Do not modify
`nix/gas-city-contributor/**` or its managed authority as part of ordinary
repository work, and do not make claims about repo-skill visibility in managed
sessions. Its focused operating detail is
[`gas-city.md`](./gas-city.md).


## Disk hygiene contract

- Put every throwaway probe, one-off crate, parser experiment, and debugging
  artifact under the gitignored repository-root `.scratch/` directory.
  Never place an exploratory file beside production code or tests, where a
  catch-all `git add` can sweep it into a commit.
- Test eval expressions MUST resolve the flake via `git+file://$ROOT`
  (use the `d2b_flake_ref` helper in `tests/lib.sh`), **never**
  `builtins.getFlake (toString $ROOT)`. A bare path makes Nix use the
  `path:` fetcher, which copies the ENTIRE working tree into the store -
  including the multi-GiB build artifacts (measured:
  ~36 GB / 5+ min per cold eval, re-triggered every time a build
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
  `builtins.getFlake (toString $ROOT)` source-capture during flake-eval gates.
- Rust worktrees keep separate Bazel output trees. Compiled-output dedup
  across worktrees comes from the shared `sccache` directory
  (`$SCCACHE_DIR`, default `~/.cache/d2b-sccache`) when the active Bazel
  profile permits it; do not point multiple worktrees at one mutable output
  directory.
- **No linker and no alternative codegen backend are configured, and that is
  a measured decision rather than an oversight.** Both were tried on this
  tree and neither earned its place. mold, wired through
  `target.<triple>.linker` and compared against separately warmed target
  directories, came out at 6.3 s against 6.7 s on a relink-heavy incremental
  build and 90 s against 93 s on a warm one - inside the run-to-run noise.
  The reason is that `[profile.dev] debug = "line-tables-only"` already
  removed the debug information that makes linking expensive, so the cost
  mold targets has largely been paid already. Cranelift, over five
  incremental pairs against a nightly LLVM control, ran 5.8 s against 7.0 s:
  a real 17% but 1.2 s in absolute terms, and it cannot enter the gate at
  all, because `rust-toolchain.toml` pins an exact stable release
  that the pinned Rust toolchain enforces, so it would mean installing and
  caching a second toolchain in every Rust job. Reopen either only with a
  measurement, and note the trap: the Rust compatibility checks export `RUSTFLAGS`,
  and that environment variable **replaces** `build.rustflags` rather than
  merging with it, so a linker configured through `rustflags` is silently
  dead there.
- The persistent-shell helper is intentionally excluded from the main
- The persistent-shell helper is intentionally excluded from the main
  product workspace at `packages/d2b-guest-shell-runner/`. Its feature
  variant remains a direct Bazel target, and the Rust, supply-chain, and
  guest-runner aliases wire it explicitly like the broker workspace.
- Run `nix-collect-garbage` after integrating a completed change when disk
reclamation is needed.
- For the operator host running heavy iteration: prune OLD
  NixOS system generations periodically:

  ```
  sudo nix-collect-garbage --delete-older-than 7d
  ```

  Old `/nix/var/nix/profiles/system-N-link` symlinks are auto-gcroots;
  each pins ~1-2 GiB of unique closure. Without periodic pruning a
  host doing frequent rebuilds (today's historical baseline: 383
  generations from 10 days of work, pinning 471 GiB) silently fills
  its disk. The gate's default post-`nix store gc` only removes
  unreferenced paths, never old generations.
- Run an opt-in deep GC separately from the Layer-1 gate:

  ```
  D2B_POST_GATE_DEEP_GC=1 nix-collect-garbage --delete-older-than 7d
  D2B_POST_GATE_DEEP_GC=1 \
  D2B_POST_GATE_DEEP_GC_SUDO=1 \
  sudo -n nix-collect-garbage --delete-older-than 7d   # + system gens
  ```

  The sudo form uses `sudo -n` and skips fail-open
  with a clear log if passwordless sudo isn't available. Threshold
  defaults to 7 days; override with `D2B_POST_GATE_DEEP_GC_DAYS=N`.
  Off by default - this is operator policy, not gate policy.
- `D2B_SKIP_WITH_ENTRA_ID=1` skips the per-example flake check for
  `examples/with-entra-id` when its pinned `vicondoa/entrablau.nix`
  input fails a per-example Nix fetch with a transient upstream 403.
  The skip knob is an explicit, reviewable carve-out used when the example
  input is unavailable. Added with the integration merge; re-evaluate once the
  entra-id input bumps past
  the affected revision.
- Before `git worktree remove`, delete the worktree's real
  `target/` (every worktree has one; there is no shared-cache
  symlink) so the removal reclaims its multi-GiB build artifacts.
  Rebuilds in a fresh worktree stay cheap because sccache retains the
  compiled outputs.
- `make clean` does that sweep for the current worktree: every build output
directory, the `.scratch/` tree, then `nix-collect-garbage`. It
  keeps `$SCCACHE_DIR` for the reason above, and deletes no file outside
  the worktree, because sibling worktrees own their artifacts and may
  have work in flight - the store collection is the one step with
  user-wide reach, and it only reclaims paths nothing still references.
  A directory is removed only when it lies inside the worktree and holds
  no git-tracked file, so an unexpected match fails closed instead of
  deleting committed content. Use `D2B_CLEAN_DRY_RUN=1` to see the sweep
  first; `D2B_CLEAN_SKIP_GC=1` and `D2B_CLEAN_KEEP_SCRATCH=1` narrow it.
  Collecting old *system* generations still needs the operator-policy
  `sudo` form above.
- `tests/tools/preflight-disk-space.sh` fails when free disk under
  `$ROOT` drops below 10 GiB. Runs after the orphan reapers but BEFORE
  the rust toolchain bootstrap so the fail-closed guard cannot be
  bypassed by disk-consuming setup before toolchain bootstrap.
- `nix flake check` now builds real `cargo-deny` + `cargo-audit`
  derivations (via `checks.${system}.rust-deny` / `.rust-audit`).
  Each derivation fetches the pinned RustSec advisory DB snapshot
  from the Nix store (no network at build time). `rust-deny` checks the root
  `Cargo.lock`; `rust-audit` checks generated context policy locks derived
  from it plus the reduced `packages/Cargo.guest.lock` for guest-static. The
  advisory DB is a
  `fetchFromGitHub` pinned to a specific commit; update the rev + hash
  in `flake.nix` periodically to pick up new advisories. Wall-clock
  impact: seconds per check (no compilation, just lockfile analysis).
