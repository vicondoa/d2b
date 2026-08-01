# Quickstart: Validating ADR-046 Delivery

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

Two audiences, two loops. The **integrator loop** validates and lands a wave. The **operator
loop** proves the control plane actually works. Both are needed: a wave that seals without an
operator-visible outcome is exactly the failure mode W0 and W1 produced.

This is a run guide. Implementation belongs in `tasks.md`.

---

## Prerequisites

- NixOS host, `x86_64-linux`, with `/dev/kvm` for Layer-2 VM checks
- The pinned toolchain resolves automatically from `packages/rust-toolchain.toml` (Rust 1.94.1)
- Heavy-gate slots provisioned for this boot. `/run` is a tmpfs, so run once per boot when the
  gate asks:

  ```bash
  make heavy-gate-provision
  ```

- Delivery state root outside any git tree (defaults to `~/.local/state/d2b/delivery`). The
  tooling refuses a root inside a working tree; do not override it into the repo.

**Commit before validating.** Untracked files are invisible to `nix flake check` and every
eval that follows the same path. A forgotten `git add` on a new module is the most common
"why didn't my change apply" failure.

---

## Integrator loop: land one wave

### 1. Confirm entry criteria

```bash
# Every prior wave's work items must be Merged. 14 of 545 today.
jq '[.workItems[] | select(.implementationState=="Merged")] | length' \
  docs/specs/ADR-046-work-items.json
```

Entry also requires: no unresolved contention flag on this wave's destination paths, the stack
proposed against the exact parent commit rather than a stale `v3`, a free heavy-gate semaphore,
and a green fast hermetic suite.

### 2. Launch every ready, file-disjoint slice together

Anti-serialization is a positive obligation, not permission (FR-028). For W2 that means both
parallel groups start in the same cycle:

```bash
git worktree add -b adr046-w2-integrate  ../d2b-w2           v3
git worktree add -b adr046-w2-primitives ../d2b-w2-primitives adr046-w2-integrate
git worktree add -b adr046-w2-routing    ../d2b-w2-routing    adr046-w2-integrate
```

A ready slice left unlaunched without a recorded blocker fails wave entry.

### 3. Inner loop while implementing

```bash
make check-tier0                 # fast; catches marker/dash/shellcheck violations
make test-rust                   # excludes the fixture-dependent contract crate
make test-fixture-contracts      # the enforcing lane for fixture-backed contracts
make check                       # full PR-equivalent Layer-1 gate
```

Read `tests/layer1-jobs.json` for the current enforcing-vs-advisory split rather than assuming
it. An advisory result is not validation evidence.

### 4. Heavy lanes, through the semaphore only

```bash
make test-integration            # Layer 2 containers; needs podman
make test-host-integration       # runNixOSTest; needs NixOS + KVM
```

Never invoke an internal `heavy-lane-*` target directly - it fails closed by design.

### 4b. Pre-panel gates (parallel, read-only)

Before any panel lane is dispatched, run both gates against **this wave scope**:

```bash
# Scope the review to the wave diff, NOT the repo default branch.
# detect-changed-files.sh resolves origin/HEAD -> main, but we integrate on v3.
BASE=$(git merge-base v3 adr046-w<N>-integrate)     # or the predecessor wave branch
git diff --name-only $BASE..adr046-w<N>-integrate
```

Dispatch two native Copilot Task lanes **in parallel** before the panel: one
reviewer lane and one rubber-duck lane. Bind each lane explicitly to
`gpt-5.6-luna`, reasoning effort `max`, and context tier `long_context`; give
both the wave diff plus `spec.md`, `plan.md`, `tasks.md`, and the constitution,
and require read-only findings. Then run the actual Copilot panel skill,
`/d2b-panel-round work`, whose ten read-only seats are bound in its table to
`gemini-3.1-pro-preview` at reasoning effort `high` and context tier `default`.
There is no separate dotted verification or review command.

Clear every verification CRITICAL, including constitution conflicts, before the panel.
A defect that reaches panel forces a content change, which invalidates the snapshot and
every record bound to it.

### 5. Snapshot, validate, panel, seal

```bash
X="cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave"

$X snapshot --program ADR046 --wave W2 --repo d2b=$PWD \
    --base d2b=<base-oid> --pull-request d2b=<number>:<head-ref>
$X validate-import   # import local/host validator results for this exact snapshot
$X panel-request     # writes the candidate-bound 10-role request (gemini-3.1-pro-preview)
$X panel-attest      # validates exactly one 14-field record per role; rejects any record
                     # whose model does not match the pinned policy
$X seal              # requires all 10 unanimous + every wave item Merged
$X merge-target --seal <state>/W2/<candidate>/seal.json --target ./merge-target.json --repo d2b=$PWD
$X merge-eligibility
```

`history-proof` is **not** a separate subcommand; it runs inside `merge-eligibility`.

Panel lanes are 10 read-only subagents on `gemini-3.1-pro-preview`, dispatched together in
one message. They take no heavy-gate slot, so all 10 run concurrently. They must not run
tests or builds unless you explicitly ask a specific lane to.

Three lanes run **concurrently** against the snapshot and never gate each other: required CI,
local/host validators, and the panel.

### 6. Merge, rebase, and clean up

Merge the wave integration branch to `v3` only after `merge-eligibility` reports
eligible. Never a local octopus merge, never a direct push (FR-044).

```bash
make changelog-fold              # integrator folds changelog.d/ fragments at wave close
git -C ../d2b-w<N+1> rebase v3   # re-point the stacked next wave onto the updated v3

# Cleanup, in this order - target dir first or removal reclaims nothing
rm -rf ../d2b-w<N>-<slice>/packages/target
git worktree remove ../d2b-w<N>-<slice>
git branch -d adr046-w<N>-<slice> adr046-w<N>-integrate
git push origin --delete adr046-w<N>-<slice> adr046-w<N>-integrate
nix-collect-garbage
git worktree list && git branch -a | grep adr046-   # must show no residue
```

A rebase changes history: panel records survive only if the byte-identical history
proof passes, and required CI reruns regardless.

### If content changes after snapshotting

Any content change invalidates **both** the validation and panel evidence. Re-snapshot and
rerun. A history-only rebase may reuse the *panel* record only if the canonical proof shows
byte-identical content, generated artifacts, dependency diff, and repository set - and CI still
reruns regardless.

---

## Operator loop: prove the plane works

This is the loop that distinguishes a live control plane from a sealed wave. It becomes
runnable as W2-W5 land; before then it fails by design, because nothing is wired.

### Story 1 - declare and reconcile

```bash
# 1. Declare a Zone with a small resource set in the host config, then:
sudo nixos-rebuild switch --flake .#<host>
sudo systemctl restart d2bd.service     # notify-ready; confirm active before validating

# 2. Every declared resource should reach ready, or name a specific cause
d2b resource list
d2b resource inspect <Type>/<name>
```

**Expected**: each resource ready or reporting an actionable failure; a dependency that is not
ready causes its dependent to *wait with a stated reason*, not fail permanently.

### Story 1 - retire and restart

```bash
# Remove one resource from config, reactivate
sudo nixos-rebuild switch --flake .#<host>
d2b resource list          # retired in dependency-safe order, cleanup visible, others intact

sudo reboot
d2b resource list          # state recovered, live resources re-adopted, nothing recreated
```

### Story 2 - Provider ownership

```bash
d2b resource inspect <Type>/<name>   # names the owning Provider and its reported state
```

**Expected**: a failing Provider is attributed by name and does not cascade to unrelated
resources.

### Story 3 - cutover rehearsal

```bash
d2b host cutover plan                 # MUST modify nothing
```

**Expected**: a complete plan listing every affected artifact with its disposition, the
preserved set, the rollback boundary, the consent text, and the recovery-point obligation.

Do not run apply without a real recovery point. Validation runs on the daily-driver host by
decision, so the attestation gate is the actual safety net, not a formality.

---

## Release validation (W8)

All six release-gate conditions, evaluated against the **final** candidate:

1. The five closing specs Accepted with evidence imported
2. Every DELETE and REPLACE row's removal proof passing **on the shipping tree**
3. The complete test matrix including manual hardware, live-host, and cloud tiers with
   recorded external evidence, plus the reset and cutover scenarios
4. Unanimous ten-role panel, seal, and merge-eligibility on the W8 snapshot
5. A new `CHANGELOG.md` version header, summarized by version, with every wave and finding
   marker stripped
6. Every prior wave's cleanup performed - no dangling worktrees or branches

Plus this program's own additions:

```bash
# Companion verification - exercise each against the release candidate on a live host
# d2b-toolkit, d2b-wlterm, d2b-wlcontrol, d2b-clip-picker  (weezterm consumes no contract)
```

**Expected**: every companion works, or the release holds (FR-039, SC-024).

---

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| Eval ignores your change | Untracked file | `git add` and commit, then re-run |
| Eval copies gigabytes and takes minutes | Bare path flake ref pulling in `packages/target` | Use `git+file://$ROOT` via the `d2b_flake_ref` helper |
| Heavy lane refuses to start | Missing slot namespace after reboot | `make heavy-gate-provision` |
| `seal` refuses | A wave work item is not `Merged`, a panel record is missing, or a lane did not report | The error names the item and required transition |
| Drift gate fails | A generated artifact was hand-edited or not regenerated | Re-run the matching `xtask gen-*` and commit |
| Disk fills during a wave | Old system generations pinning closures | `sudo nix-collect-garbage --delete-older-than 7d` |
