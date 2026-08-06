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

Entry requires Gate 0 passed, no unresolved contention flag on this wave's destination paths,
the stack proposed against the exact named parent commit rather than a stale `v3`, a free
heavy-gate semaphore, and a green fast hermetic suite. If the predecessor is not yet merged,
implementation may start only after at least 5 of its 10 reviews return and integration is
green on its converged tree. Prior-wave `Merged` state is not entry evidence. It is checked at
the successor's panel request, seal, and merge-eligibility boundary, after the successor
rebases onto the merged predecessor.

### 1b. Reconcile `adr046w5` progress before implementation

T603 never infers task completion from code presence and never edits a feature artifact
directly. First freeze clean pre-validator base A and feature snapshot P0, run
`/speckit-analyze`, and obtain unanimous plan signoff at A/P0. That pair authorizes only
`packages/xtask/src/delivery/{mod.rs,resume.rs}`. Land validator-only commit V with sole
parent A, freeze B exactly at V, require P to remain byte-identical to P0, rerun analysis over
A..B plus the feature artifacts, and rerun the plan panel at B/P. Any finding or later
validator change invalidates B and requires both post-validator gates again.

Only the post-validator receipts permit the fd-anchored validator to write immutable
authorization R at
`.scratch/autopilot/adr046w5/reconciliation.json`, bound to repository identity, the
repository-relative feature path, clean resume base B and tree, exact 28-file pre-edit
snapshot P, validator-derived post-edit snapshot Q, post-validator analysis receipt,
ten-record post-validator plan panel, and one row for each T073-T218 obligation.

If any row is `open`, leave T603 and T073-T218 unchecked and stop. If all 146 rows are
`satisfied`, the post-validator analysis has no unresolved HIGH or CRITICAL finding, and the
post-validator unanimous plan signoff names B/P, route one explicit `/d2b-spec-edit` progress
batch. The editor recomputes R before making its only permitted feature changes: checking
T073-T218 and T603. The Wave 5
integrator stages only that diff and owns dedicated commit C with exact parent B. The
validator then finalizes `.scratch/autopilot/adr046w5/progress-editor-receipt.json`, binding
B, C, P, and Q. A retry converges only from exact B/P, B/Q, or C/Q. T589 refuses unless HEAD
is clean C, the finalized receipt validates, and all 147 checkboxes are checked. T602 later
validates C as an ancestor of separate final candidate F rather than requiring R to match F.

C1 is approved and fully assigned under Constitution 2.2.0. Run the pre-T603 analysis and
plan panel first. Implementation remains pending: after validator V, the post-T603 analysis
and plan panel must rerun before T603 may reconcile exactly T073-T218; T605 remains future
work after resume rather than a 147th receipt row.

### 2. Launch every ready, file-disjoint slice together

Anti-serialization is a positive obligation, not permission (FR-028). For W2 that means both
parallel groups start in the same cycle:

```bash
git worktree add -b adr046-w2-integrate  ../d2b-w2           v3
git worktree add -b adr046-w2-primitives ../d2b-w2-primitives adr046-w2-integrate
git worktree add -b adr046-w2-routing    ../d2b-w2-routing    adr046-w2-integrate
```

A ready slice left unlaunched without a recorded blocker fails wave entry.

For `adr046w5`, the exact implementation and close chain is
`T589 -> {T590,T591,T594}; T591 -> T592 -> T593 -> T605;
{T590,T592,T594,T605} -> T595 -> {T596,T597,T598,T599,T604} ->
T220 -> F -> {T600,T601} -> T602 -> T219`.
T595 may not start until both serialized branches and the other completion slices converge and consumes T605's
`SystemCoreHost` and `SystemCoreUser` variants. T220 reconciles generated manifests and every
remaining content change before F; T219 alone runs the binding panel, seal, and merge.

### 3. Inner loop while implementing

```bash
make check-tier0                 # fast; catches marker/dash/shellcheck violations
make test-rust                   # excludes the fixture-dependent contract crate
make test-fixture-contracts      # the enforcing lane for fixture-backed contracts
make check                       # full PR-equivalent Layer-1 gate
```

Read `tests/layer1-jobs.json` for the current enforcing-vs-advisory split rather than assuming
it. An advisory result is not validation evidence.

T605 alone regenerates compiler-derived public/private API snapshots, and only through the
pin target. Its focused loop is:

```bash
make api-surface-pin
make test-rust-api-surface
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

The result must prove `SystemCoreHost`/`SystemCoreUser` kebab-case round-trip, exactly one
`Zone.status.handlers[]` record named `system-core-host` and one named `system-core-user`
with `phase` and `lastReconciledAt`, duplicate/missing/wrong-name rejection,
`ProviderLifecycle` non-substitution, current API snapshots, paired runtime reference text,
and byte-identical generated Zone desired schema. T605 does not wait for T595/T599 and does
not run the full drift gate. T595 owns the emitter, T599 owns downstream consumers, and T220
reconciles integrator-owned generated spec manifests and runs `make test-drift` before F.

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

Issue two separate native Copilot Task invocations **together in one
coordination cycle** before the panel: one reviewer lane and one rubber-duck
lane. Bind each lane explicitly to
`gpt-5.6-luna`, reasoning effort `max`, and context tier `long_context`; give
both the wave diff plus `spec.md`, `plan.md`, `tasks.md`, and the constitution,
and require read-only findings. Then run the actual Copilot panel skill,
`/d2b-panel-round work`, whose ten read-only seats are bound in its table to
`gpt-5.6-sol` at reasoning effort `xhigh` and context tier `default`.
There is no separate dotted verification or review command.

Clear every verification HIGH and CRITICAL, including constitution conflicts, before the
binding panel request. For `adr046w5`, a defect found here returns to T220 and reruns
T600-T602 against a new F before the binding panel is invoked.

### 5. Snapshot, validate, panel, seal

```bash
X="cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave"

$X snapshot --program ADR046 --wave W2 --repo d2b=$PWD \
    --base d2b=<base-oid> --pull-request d2b=<number>:<head-ref>
$X validate-import   # import local/host validator results for this exact snapshot
$X panel-request     # writes the candidate-bound 10-role request (gpt-5.6-sol/xhigh)
$X panel-attest      # validates exactly one 14-field record per role; rejects any record
                     # whose model does not match the pinned policy
$X seal              # requires all 10 unanimous + every wave item Merged
$X merge-target --seal <state>/W2/<candidate>/seal.json --target ./merge-target.json --repo d2b=$PWD
$X merge-eligibility
```

`history-proof` is **not** a separate subcommand; it runs inside `merge-eligibility`.

Panel lanes are 10 read-only subagents on `gpt-5.6-sol` at `xhigh`, dispatched
together in one message. They take no heavy-gate slot, so all 10 run
concurrently. They must not run tests or builds unless you explicitly ask a
specific lane to.

For ordinary waves, required CI, local/host validators, and panel lanes may run concurrently
against the snapshot. For `adr046w5`, T600/T601 evidence and T602's closed-set check complete
before T219 issues the single binding panel request.

### 6. Merge, rebase, and clean up

Merge the wave integration branch to `v3` only after `merge-eligibility` reports
eligible. Never a local octopus merge, never a direct push (FR-044).

```bash
make changelog-fold              # before snapshot/panel; T220 owns this for adr046w5
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

Any content change before the binding panel request invalidates validation evidence: converge,
re-snapshot, and rerun before requesting the panel. For `adr046w5`, no content or evidence
identity may change after T219's binding request and no second binding panel may run; such a
state fails closed for integrator escalation rather than silently re-attesting changed
content. The eligible integration-lineage merge may change history only while preserving F's
tree.

---

## Operator loop: prove the plane works

This is the loop that distinguishes a live control plane from a sealed wave. It becomes
runnable as W2-W5 land; before then it fails by design, because nothing is wired.

### Story 1 - declare and reconcile

The exact-candidate automated proof is T604. Its fixture-contract leg owns
`packages/d2b-contract-tests/tests/resource_operator_activation.rs`; its lowest feasible
production-boundary leg owns `packages/d2bd/tests/resource_operator_activation.rs`; and its
real activation/effect leg owns
`tests/host-integration/resource-operator-activation.nix`. Run only through the existing
public targets:

```bash
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
make test-rust
make test-host-integration
```

The host leg declares the representative Guest, Volume, Network, and Device, consumes the
emitted `zones/<zone>/resource-bundle.json`, activates on startup and public NixOS switches
through the production daemon, and requires a real owned effect and readiness for every one
of the four supported representative resources. Refusals are separate negative cases. It
then removes the Guest, switches the next generation without a manual daemon restart,
verifies dependency-safe cleanup, and proves the unrelated resources remain ready and intact.
Direct ResourceService calls, private reloads, and status-only effects do not satisfy T604.

```bash
# 1. Declare a Zone with a small resource set in the host config, then:
sudo nixos-rebuild switch --flake .#<host>

# 2. Every supported representative resource must reach its owned effect and ready state
d2b resource list
d2b resource inspect <Type>/<name>
```

**Expected**: the Guest, Volume, Network, and Device are each ready through their owned effect.
Actionable refusal coverage runs separately and cannot satisfy this positive proof.

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
