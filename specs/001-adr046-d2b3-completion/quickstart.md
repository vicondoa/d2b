# Quickstart: Validating ADR-046 Delivery

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

Two audiences, two loops. The **integrator loop** validates and lands a wave. The **operator
loop** proves the control plane actually works. Both are needed: a wave that seals without an
operator-visible outcome is exactly the failure mode W0 and W1 produced.

This is a run guide. Implementation belongs in `tasks.md`.

---

## Prerequisites

- NixOS host, `x86_64-linux`, with `/dev/kvm` for Layer-2 VM checks
- The pinned Rust toolchain resolves automatically from
  `packages/rust-toolchain.toml` (currently 1.97.0); the pin is authoritative.
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

Constitution 3.1.0 supplies a generic historical-process disposition with no ADR-046 detail.
This feature's exact delivery validator/tooling contract bounds it through merged Wave 5
commit `177235ed37188b3be87525e7f016fb43401574c5`. It creates no Wave 5 seal and authorizes no
Wave 5 recovery. For Wave 6, entry first requires the production historical-predecessor guard
against the fetched exact `origin/v3` base. After that guard, entry requires Gate 0 passed, no
unresolved contention flag on Wave 6 destinations, a free heavy-gate semaphore, and a green
fast hermetic suite. The ordinary T221 selected roster uses the current thirteen-seat role
domain and may only widen over fix deltas.

### 1b. Verify the Wave 5 to Wave 6 boundary

Do not run a Wave 5 recovery or close command. Fetch and bind the exact Wave 6 base, then run
the focused guard tests:

```bash
set -eu

REPOSITORY="github.com/vicondoa/d2b"
CHECKOUT_ROOT="$(git rev-parse --show-toplevel)"
TARGET_BRANCH="v3"
git fetch origin "$TARGET_BRANCH"
BASE_OID="$(git rev-parse "refs/remotes/origin/$TARGET_BRANCH")"
test "$(git rev-parse "refs/remotes/origin/$TARGET_BRANCH")" = "$BASE_OID"
git merge-base --is-ancestor \
  177235ed37188b3be87525e7f016fb43401574c5 "$BASE_OID"

cargo test --manifest-path packages/Cargo.toml -p xtask \
  delivery::work_item_state::tests
```

Create the Wave 6 entry snapshot through the production delivery command, using
`BASE_OID` as the exact base and the current draft Wave 6 PR. The snapshot command must
match candidate
`d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`,
snapshot identity `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`,
head `19b77dad63060bcadd41f1ef800978d2c53cc030`, retained request digest
`15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7`, zero attestations,
no seal, and every retained evidence filename and digest in `data-model.md`. It must also
identify the accepted first-parent integration commit after the Wave 5 merge whose tree
contains the exact generic Constitution 3.1.0 bytes. A missing, extra, or changed entry stops
here.

```bash
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/d2b/delivery"
PR_NUMBER="$(gh pr view --json number --jq .number)"
HEAD_REF="$(git symbolic-ref --short HEAD)"

cargo run --manifest-path packages/Cargo.toml -p xtask -- \
  delivery wave snapshot \
  --program ADR046 \
  --wave adr046w6 \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --base "$REPOSITORY=$BASE_OID" \
  --pull-request "$REPOSITORY=$PR_NUMBER:$HEAD_REF" \
  --state-dir "$STATE_DIR"
```

Only after that succeeds, run `/d2b-panel-round plan` against the exact base, entry snapshot,
and current feature snapshot. T221 remains incomplete until every selected seat signs off
with zero recommendations.

### 1c. Reject actionable retired-phase prose

Run this read-only check after any feature-artifact edit. Explicitly marked historical blocks
are allowed; the word `historical` alone is not suppression. Actionable retired-task
instructions, including multiline forms, and round-threshold deferral rules are not.

<!-- STALE-PROSE-CHECK-BEGIN -->

```bash
set -eu

FEATURE_DIR="specs/001-adr046-d2b3-completion"
RETIRED='T(219|220|589|590|591|592|593|594|595|596|597|598|599|600|601|602|603|605)'
ACTION="(?s)\\b${RETIRED}\\b([[:space:]]*(/|,|and)[[:space:]]*\\b${RETIRED}\\b)*[[:space:]]+(MUST[[:space:]]+)?(dispatch|implement|prepare|reconcile|fold|measure|consume|reject|seal|close|run|verify|import|freeze|refuse|require|revalidate|emit|reopen|block)(s|es|ed|ing|ation|ment)?\\b"
EDGE="(?s)\\b(before|until|depends[[:space:]]+on)[[:space:]]+\\b${RETIRED}\\b"

current_hits="$(
  while IFS= read -r -d '' file; do
    awk -v file="$file" '
      /RETIRED-W5-.*-BEGIN/ { retired = 1; next }
      /RETIRED-W5-.*-END/ { retired = 0; next }
      /STALE-PROSE-CHECK-BEGIN/ { retired = 1; next }
      /STALE-PROSE-CHECK-END/ { retired = 0; next }
      !retired { print file ":" FNR ":" $0 }
    ' "$file"
  done < <(find "$FEATURE_DIR" -type f -name '*.md' -print0) |
    rg -n -U -i "$ACTION|$EDGE" || true
)"
test -z "$current_hits" || {
  printf '%s\n' "$current_hits"
  exit 1
}

! rg -n -i \
  'round nine.*\bMAY\b|eight panel rounds.*\bMAY\b|^## Standing obligations$' \
  "$FEATURE_DIR/deferred-findings.md"
```

<!-- STALE-PROSE-CHECK-END -->

### 2. Launch every ready, file-disjoint slice together

Anti-serialization is a positive obligation, not permission (FR-028). For W2 that means both
parallel groups start in the same cycle:

```bash
git worktree add -b adr046-w2-integrate  ../d2b-w2           v3
git worktree add -b adr046-w2-primitives ../d2b-w2-primitives adr046-w2-integrate
git worktree add -b adr046-w2-routing    ../d2b-w2-routing    adr046-w2-integrate
```

A ready slice left unlaunched without a recorded blocker fails wave entry.

`adr046w5` has no executable implementation or close chain in this guide. Its retained state
is immutable history with zero attestations and no seal. T219 records that historical
disposition. Start only Wave 6 work selected by the T221 plan result, and launch every ready,
file-disjoint Wave 6 slice in the same coordination cycle.

### 3. Inner loop while implementing

```bash
make check-tier0                 # fast; catches marker/dash/shellcheck violations
make test-rust                   # excludes the fixture-dependent contract crate
make test-fixture-contracts      # the enforcing lane for fixture-backed contracts
make check                       # full PR-equivalent Layer-1 gate
```

Read `tests/layer1-jobs.json` for the current enforcing-vs-advisory split rather than assuming
it. An advisory result is not validation evidence.

The former T605/T595/T599/T220 Wave 5 loop is historical planning evidence. Do not rerun it to
reconstruct Wave 5 completion. Wave 6 implementers run only the validation owned by their
T221-selected tasks.

### 4. Heavy lanes, through the semaphore only

```bash
make test-integration            # Layer 2 containers; needs podman
make test-host-integration       # runNixOSTest; needs NixOS + KVM
```

Never invoke an internal `heavy-lane-*` target directly - it fails closed by design.

### 4b. Pre-panel gates (parallel, read-only)

**Historical `adr046w5` boundary:** do not dispatch any Wave 5 panel lane or run any Wave 5
delivery command. Its consumed request, zero attestations, and absent seal are immutable.
Proceed here only for Wave 6 after T221 passes.

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
`/d2b-panel-round work`, whose candidate-selected read-only seats come from the current
thirteen-seat role domain and are bound in its table to
`gpt-5.6-sol` at reasoning effort `xhigh` and context tier `default`.
There is no separate dotted verification or review command.

Clear every actionable content finding, at any severity, including constitution conflicts, before the
binding panel request on an ordinary unconsumed wave. For Wave 6, a defect found here returns to convergence and requires a replacement candidate
before the sole binding request.

### 5. Track A - approve, bind, merge, seal, and close

This procedure applies only to a wave whose binding request has not been consumed.
The historical `adr046w5` state MUST NOT execute any command in this subsection.

Enter only after the final nonbinding Discover-Fix-Verify lifecycle is unanimously approved
and no content-changing fix remains. The PR may already exist as a draft because `snapshot`
must bind its real number and head, but it stays unmergeable until the sole request, records,
and attestation below are complete. The changelog fold and every generated artifact must
already be part of the approved tree. The effective `v3` repository rules must configure a
nonempty set of required status checks for strict up-to-date enforcement. This requirement
applies whether or not a merge queue is enabled, so GitHub atomically refuses merge after the
expected base becomes stale. A merge queue is sufficient only when
`MERGE_GROUP_TREE_CHECK` names a required check triggered for `merge_group` that resolves the
actual merge-group head's tree, compares it with the snapshot-bound expected
`integration_tree_oid`, and refuses a mismatch. A queue without that required comparison, a
merely protected branch, a head-only `--match-head-commit` check, or the post-merge tree
comparison below is not the preventive base guard.

Immediately after the selected Task runs return, the same-user integrator must materialize
`$ROUND/task-runs.json` directly from the Task result envelopes. It is an object keyed by
every selected seat, with exactly `run_id` and `receipt_locator` for that seat. Do not copy
either value from a reviewer verdict or invent a replacement. The workflow below combines
those actual process values with the completion-bound dispatch binding and agent-definition
digests to create `$ROUND/observed.json` before `make-records`.

```bash
set -eu

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

CHECKOUT_ROOT="$(git rev-parse --show-toplevel)"
REPOSITORY="github.com/owner/repository"
GITHUB_REPOSITORY="${REPOSITORY#github.com/}"
TARGET_BRANCH="v3"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/d2b/delivery"
git fetch origin "$TARGET_BRANCH"
BASE_OID="$(git rev-parse "origin/$TARGET_BRANCH")"
git merge-base --is-ancestor "$BASE_OID" HEAD ||
  fail "HEAD is not based on current origin/$TARGET_BRANCH; update the integration branch and restart validation, selected-roster verification, snapshot, binding, and required checks"
HEAD_REF="$(git symbolic-ref --short HEAD)"
PR_NUMBER="$(gh pr view --json number --jq .number)"
PROGRAM="${PROGRAM:-ADR046}"
WAVE="${WAVE:?set WAVE to the current qualified wave}"
ROUND="${ROUND:?set ROUND to the final verification round directory}"
MERGE_GROUP_TREE_CHECK="${MERGE_GROUP_TREE_CHECK:-}"

PR_IDENTITY="$(gh pr view "$PR_NUMBER" \
  --json baseRefName,baseRefOid,headRefName,headRefOid)"
HEAD_OID="$(git rev-parse HEAD)"
jq -e \
  --arg base_ref "$TARGET_BRANCH" \
  --arg base_oid "$BASE_OID" \
  --arg head_ref "$HEAD_REF" \
  --arg head_oid "$HEAD_OID" '
  .baseRefName == $base_ref and
  .baseRefOid == $base_oid and
  .headRefName == $head_ref and
  .headRefOid == $head_oid
' <<<"$PR_IDENTITY" ||
  fail "PR identity or base changed; update the integration branch and restart validation, selected-roster verification, snapshot, binding, and required checks"

BRANCH_RULES="$(gh api \
  "repos/$GITHUB_REPOSITORY/rules/branches/$TARGET_BRANCH")"
MERGE_MODE="direct"
jq -e '
  any(.[];
    .type == "required_status_checks" and
    .parameters.strict_required_status_checks_policy == true and
    ((.parameters.required_status_checks // []) | length) > 0
  )
' <<<"$BRANCH_RULES" >/dev/null ||
  fail "configure effective v3 protection with nonempty strict up-to-date required status checks before binding"

if jq -e 'any(.[]; .type == "merge_queue")' \
  <<<"$BRANCH_RULES" >/dev/null; then
  MERGE_MODE="merge-queue"
  test -n "$MERGE_GROUP_TREE_CHECK" ||
    fail "v3 uses a merge queue; set MERGE_GROUP_TREE_CHECK to its required snapshot-bound integration-tree check"
  jq -e --arg context "$MERGE_GROUP_TREE_CHECK" '
    any(.[];
      .type == "required_status_checks" and
      .parameters.strict_required_status_checks_policy == true and
      any(.parameters.required_status_checks[]?;
        .context == $context
      )
    )
  ' <<<"$BRANCH_RULES" >/dev/null ||
    fail "the merge queue check is not required by effective strict v3 protection"
fi

SELECTION="$ROUND/selection.json"
LEDGER="$ROUND/discovery-ledger.json"
RESPONSES="$ROUND/responses.json"
VERIFICATION_RESULTS="$ROUND/verification-results.json"
APPROVAL="$ROUND/approval.json"
DISPATCH_BINDING="$ROUND/dispatch-binding.json"
COMPLETION="$ROUND/.complete"
TASK_RUNS="$ROUND/task-runs.json"
OBSERVED="$ROUND/observed.json"

jq -e '.approved == true' "$APPROVAL"

artifact_ref() {
  jq -er 'select(.status == "ok") | .artifact'
}

X=(cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave)

SNAPSHOT_RESULT="$("${X[@]}" snapshot \
  --program "$PROGRAM" \
  --wave "$WAVE" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --base "$REPOSITORY=$BASE_OID" \
  --pull-request "$REPOSITORY=$PR_NUMBER:$HEAD_REF" \
  --state-dir "$STATE_DIR")"
SNAPSHOT="$(printf '%s\n' "$SNAPSHOT_RESULT" | artifact_ref)"
EXPECTED_INTEGRATION_TREE_OID="$(
  jq -er --arg repository "$REPOSITORY" '
    .material.repository_set[]
    | select(.id == $repository)
    | .integration_tree_oid
  ' "$STATE_DIR/$SNAPSHOT"
)"
test "$EXPECTED_INTEGRATION_TREE_OID" = \
  "$(git rev-parse "${HEAD_OID}^{tree}")" ||
  fail "snapshot integration_tree_oid does not match the selected integration tree"

jq -e --slurpfile snapshot "$STATE_DIR/$SNAPSHOT" '
  .candidate_id == $snapshot[0].candidate_id and
  .content_id == $snapshot[0].content_id and
  .snapshot_sha256 == $snapshot[0].snapshot_sha256
' "$SELECTION"

PANEL_REQUEST_RESULT="$("${X[@]}" panel-request \
  --snapshot "$SNAPSHOT" \
  --selection "$SELECTION" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
PANEL_REQUEST="$(printf '%s\n' "$PANEL_REQUEST_RESULT" | artifact_ref)"

test -f "$TASK_RUNS" ||
  fail "$TASK_RUNS is missing; capture actual selected Task run metadata"
test "$(sha256sum "$DISPATCH_BINDING" | awk '{print $1}')" = \
  "$(jq -er '.artifact_sha256["dispatch-binding.json"]' "$COMPLETION")" ||
  fail "dispatch-binding.json is not bound by the round completion marker"
while IFS= read -r seat; do
  definition="agent-definitions/panel-$seat.agent.md"
  test "$(sha256sum "$ROUND/$definition" | awk '{print $1}')" = \
    "$(jq -er --arg definition "$definition" \
      '.artifact_sha256[$definition]' "$COMPLETION")" ||
    fail "$definition is not bound by the round completion marker"
done < <(jq -er '.roster[]' "$SELECTION")

jq -e -n \
  --slurpfile selection "$SELECTION" \
  --slurpfile dispatch "$DISPATCH_BINDING" \
  --slurpfile completion "$COMPLETION" \
  --slurpfile runs "$TASK_RUNS" '
  ($selection[0]) as $selection |
  ($dispatch[0]) as $dispatch |
  ($completion[0]) as $completion |
  ($runs[0]) as $runs |
  if
    $dispatch.roster != $selection.roster or
    (($runs | keys | sort) != ($selection.roster | sort))
  then
    error("task run, dispatch binding, and selected rosters disagree")
  elif
    (($selection.roster | map($runs[.].run_id) | unique | length) !=
      ($selection.roster | length)) or
    (($selection.roster | map($runs[.].receipt_locator) | unique | length) !=
      ($selection.roster | length))
  then
    error("selected Task run IDs and receipt locators must be unique")
  else
    [
      $selection.roster[] as $seat |
      ($dispatch.bindings[$seat]) as $binding |
      ($runs[$seat]) as $run |
      ($completion.artifact_sha256[
        "agent-definitions/panel-\($seat).agent.md"
      ]) as $definition_sha256 |
      if
        (($run | keys | sort) != ["receipt_locator", "run_id"]) or
        (($run.run_id | type) != "string") or
        ($run.run_id | length) == 0 or
        (($run.receipt_locator | type) != "string") or
        ($run.receipt_locator | startswith("github-copilot://") | not) or
        (($definition_sha256 | type) != "string") or
        ($definition_sha256 | test("^[0-9a-f]{64}$") | not)
      then
        error("invalid selected Task process metadata or definition digest for \($seat)")
      else
        {
          key: $seat,
          value: {
            provider: "github-copilot",
            model: $binding.model,
            reasoning_effort: $binding.reasoning_effort,
            context_tier: $binding.context_tier,
            communication: $binding.communication,
            agent_type: $binding.agent_type,
            agent_definition_sha256: $definition_sha256,
            run_id: $run.run_id,
            receipt_locator: $run.receipt_locator
          }
        }
      end
    ] | from_entries
  end
' >"$OBSERVED.tmp"
mv -- "$OBSERVED.tmp" "$OBSERVED"

node .github/skills/d2b-panel-round/scripts/make-records.mjs "$ROUND" \
  --selection "$SELECTION" \
  --ledger "$LEDGER" \
  --responses "$RESPONSES" \
  --verification-results "$VERIFICATION_RESULTS" \
  --approval "$APPROVAL"
RECORDS_DIR="$ROUND/records"

PANEL_ATTEST_RESULT="$("${X[@]}" panel-attest \
  --snapshot "$SNAPSHOT" \
  --records "$RECORDS_DIR" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
PANEL_RECORDS="$(printf '%s\n' "$PANEL_ATTEST_RESULT" | artifact_ref)"

# Wait for the protected PR's required checks, then import exact-snapshot
# validator results. A failed or empty required-check set stops here.
if [ "$(gh pr view "$PR_NUMBER" --json isDraft --jq .isDraft)" = "true" ]; then
  gh pr ready "$PR_NUMBER"
fi
gh pr checks "$PR_NUMBER" --required --watch
REQUIRED_CHECKS="$(gh pr checks "$PR_NUMBER" --required --json name,state)"
jq -e 'length > 0 and all(.state == "SUCCESS")' <<<"$REQUIRED_CHECKS"
CURRENT_BASE_OID="$(gh pr view "$PR_NUMBER" --json baseRefOid --jq .baseRefOid)"
test "$CURRENT_BASE_OID" = "$BASE_OID" ||
  fail "v3 changed; update the integration branch and restart validation, selected-roster verification, snapshot, binding, and required checks"

EVIDENCE_GITHUB_CI_RESULT="$("${X[@]}" validate-import \
  --snapshot "$SNAPSHOT" \
  --validation required-github-ci \
  --result passed \
  --lane github-ci \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
EVIDENCE_GITHUB_CI="$(printf '%s\n' "$EVIDENCE_GITHUB_CI_RESULT" | artifact_ref)"

EVIDENCE_LOCAL_HOST_RESULT="$("${X[@]}" validate-import \
  --snapshot "$SNAPSHOT" \
  --validation required-local-host \
  --result passed \
  --lane local-host \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
EVIDENCE_LOCAL_HOST="$(printf '%s\n' "$EVIDENCE_LOCAL_HOST_RESULT" | artifact_ref)"

# Capture the exact green pre-merge PR state. Registration remains post-merge
# because it consumes the seal that can exist only after the PR merge.
PR_STATE="$(gh pr view "$PR_NUMBER" \
  --json number,baseRefName,baseRefOid,headRefName,headRefOid)"
jq -e \
  --arg base_ref "$TARGET_BRANCH" \
  --arg base_oid "$BASE_OID" \
  --arg head "$HEAD_OID" '
  .baseRefName == $base_ref and
  .baseRefOid == $base_oid and
  .headRefOid == $head
' <<<"$PR_STATE" ||
  fail "PR base or head changed; update the integration branch and restart validation, selected-roster verification, snapshot, binding, and required checks"
MERGE_TARGET_INPUT="$(mktemp)"
trap 'rm -f "$MERGE_TARGET_INPUT"' EXIT
jq -n \
  --slurpfile snapshot "$STATE_DIR/$SNAPSHOT" \
  --arg repository "$REPOSITORY" \
  --argjson pr "$PR_STATE" \
  --argjson checks "$REQUIRED_CHECKS" '
  {
    artifact_kind: "d2b-delivery/merge-target",
    schema_version: $snapshot[0].schema_version,
    material: $snapshot[0].material,
    pull_requests: [
      {
        repository: $repository,
        number: $pr.number,
        base_ref: $pr.baseRefName,
        base_oid: $pr.baseRefOid,
        head_ref: $pr.headRefName,
        head_oid: $pr.headRefOid,
        required_checks: ($checks | map({
          name: .name,
          conclusion: (.state | ascii_downcase)
        }))
      }
    ]
  }
' >"$MERGE_TARGET_INPUT"

# This is the point of no return. GitHub's effective v3 rule is the preventive
# base guard. The post-merge tree comparison is defense in depth, not the guard.
IMMEDIATE_BASE_OID="$(gh pr view "$PR_NUMBER" --json baseRefOid --jq .baseRefOid)"
test "$IMMEDIATE_BASE_OID" = "$BASE_OID" ||
  fail "v3 changed immediately before merge; update the integration branch and restart validation, selected-roster verification, snapshot, binding, and required checks"
if [ "$MERGE_MODE" = "merge-queue" ]; then
  gh pr merge "$PR_NUMBER" --match-head-commit "$HEAD_OID"
  attempts=0
  while [ "$(gh pr view "$PR_NUMBER" --json state --jq .state)" = "OPEN" ]; do
    current_base="$(gh pr view "$PR_NUMBER" --json baseRefOid --jq .baseRefOid)"
    if [ "$current_base" != "$BASE_OID" ]; then
      gh pr merge "$PR_NUMBER" --disable-auto || true
      fail "v3 changed while queued; update the integration branch and restart validation, selected-roster verification, snapshot, binding, and required checks"
    fi
    attempts=$((attempts + 1))
    test "$attempts" -lt 120 ||
      fail "merge queue did not complete within the bounded wait"
    sleep 5
  done
else
  gh pr merge "$PR_NUMBER" --merge --match-head-commit "$HEAD_OID"
fi
test "$(gh pr view "$PR_NUMBER" --json state --jq .state)" = "MERGED"
MERGE_COMMIT="$(gh pr view "$PR_NUMBER" --json mergeCommit --jq .mergeCommit.oid)"
git fetch origin "$TARGET_BRANCH"
test "$(git rev-parse "${MERGE_COMMIT}^{tree}")" = \
  "$(git rev-parse "${HEAD_OID}^{tree}")"

SEAL_RESULT="$("${X[@]}" seal \
  --snapshot "$SNAPSHOT" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
SEAL="$(printf '%s\n' "$SEAL_RESULT" | artifact_ref)"

MERGE_TARGET_RESULT="$("${X[@]}" merge-target \
  --seal "$SEAL" \
  --target "$MERGE_TARGET_INPUT" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
MERGE_TARGET="$(printf '%s\n' "$MERGE_TARGET_RESULT" | artifact_ref)"

MERGE_ELIGIBILITY_RESULT="$("${X[@]}" merge-eligibility \
  --seal "$SEAL" \
  --target "$MERGE_TARGET" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
MERGE_ELIGIBILITY="$(printf '%s\n' "$MERGE_ELIGIBILITY_RESULT" | artifact_ref)"

printf '%s\n' \
  "$SNAPSHOT" \
  "$EVIDENCE_GITHUB_CI" \
  "$EVIDENCE_LOCAL_HOST" \
  "$PANEL_REQUEST" \
  "$PANEL_RECORDS" \
  "$SEAL" \
  "$MERGE_TARGET" \
  "$MERGE_ELIGIBILITY"
```

`validate-import` writes candidate-addressed evidence artifacts; later `seal` discovers them
through `--snapshot`, so the delivery CLI has no separate `--evidence` option. Every stage
above still captures its emitted artifact reference and repeats the same valid repository
mapping.

The order is deliberate: final nonbinding approval, final snapshot and selection, the sole
request, observed process metadata, records and attestation, protected PR checks, merge,
seal, merge-target registration, and merge eligibility. `seal` refuses until every
current-wave item is `Merged`; moving it before the PR merge recreates the cycle this
workflow is designed to prevent. The merge-target input is captured from the green PR
immediately before merge, then registered after the seal so the post-merge commands consume
the exact pre-merge head and check state.
R12 and R55 do not reorder those stages. The Wave 5 historical disposition does not relax
them for Wave 6.

Every seat in `observed.json` has exactly these required fields: `provider`, `model`,
`reasoning_effort`, `context_tier`, `communication`, `agent_type`,
`agent_definition_sha256`, `run_id`, and `receipt_locator`. The first six values come from
the completion-bound dispatch policy except for the fixed `github-copilot` provider; the
definition digest comes from the completion marker; and the final two values come from the
actual selected Task result envelope. This is same-user process metadata validated against
the completed packet for correlation and uniqueness. It is not authentication, an
authentication proof, or evidence that a particular definition executed.

If `v3` changes at any point after validation or selected-roster verification begins, the
atomic repository rule must refuse the merge. The operator then updates the integration
branch and restarts validation, selected-roster verification, snapshot creation, and
candidate binding, then reruns the required checks in the same Track A order. The old
snapshot, records, attestation, and CI evidence are ineligible. If the old attempt already
consumed the wave's sole binding request, the existing no-second-request rule requires an
accepted external disposition before that restart can establish another binding; it never
permits merging the stale attempt. When a merge queue is enabled, its required
`merge_group` check must use the actual merge-group integration tree and the same
snapshot-bound expected `integration_tree_oid`; the polling loop is defense in depth, not a
substitute for that atomic refusal.

`history-proof` is **not** a separate subcommand; it runs inside `merge-eligibility`.

Panel lanes are exactly the read-only seats and profiles in the lifecycle selection artifact, dispatched on their recorded bindings
together in one message. They take no heavy-gate slot, so all selected lanes run
concurrently. They must not run tests or builds unless you explicitly ask a
specific lane to.

For prospective waves, local/host validators may run against the snapshot while the final
records are assembled. Required PR checks must be green and imported before merge.

### 6. Rebase and clean up

Section 5 has already merged the protected PR, sealed the merged wave, registered the captured
merge target, and passed merge eligibility. Never substitute a local octopus merge or direct
push (FR-044).

```bash
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
re-snapshot, and rerun before requesting the panel. The retained Wave 5 request is historical
and is never re-attested, replaced, or used for successor admission.

### Historical SC-002 recovery plan

The former T589/T599/T220 recovery sequence is read-only historical planning evidence. It is
not an executable runbook and supplies no current command, gate, or refusal. Current work must
obtain any recovery action from an accepted current generated traceability row owned by a
prospective task after T221; nothing in the retired Wave 5 plan may be inferred or run.

## Operator loop: prove the plane works

This is the loop that distinguishes a live control plane from a sealed wave. Its exact
operator activation positive remains W6 acceptance after T221. T221 first requires the
accepted external Network contract/work-item amendment to remove every current-facing sole
Network-opt-in path and retain T336-T355 plus all four double-opt-in cases as authoritative
W6 work. T604 then consumes their merged implementation. A stale sole-opt-in contract makes
T221 fail closed; an unimplemented T336-T355 row blocks T479.

### Story 1 - declare and reconcile

The exact-F6 automated proof is T604. It remains W6 work under T221 and consumes the merged
T336-T355 result without moving those tasks into W5. Its fixture-contract leg owns
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

The host leg declares Zone `acceptance` with the exact W6 operator acceptance set -
`Volume/acceptance-state` through `Provider/volume-local`,
`Network/acceptance-net` through `Provider/network-local`, and
`Device/acceptance-tpm` through `Provider/device-tpm` - and consumes the emitted
`zones/<zone>/resource-bundle.json`, activates on startup and deployment-entrypoint
transitions through the production daemon, and requires a real owned effect and readiness for
every one of those three exact resources under the Provider/config/effect predicates in
`spec.md`. Refusals are separate negative cases. It then removes only
`Device/acceptance-tpm`, deploys the next generation without a manual daemon restart, verifies
exact swtpm/flush cleanup, an unresolvable Endpoint, finalizer clearance, and same-identity
TPM state-Volume preservation,
and proves `Volume/acceptance-state`, `Network/acceptance-net`, and unrelated resources remain
ready, identity-stable, intact, and unrecreated. The same candidate must also pass the
no-skip `vmChecks.x86_64-linux.daemon-restart-vm-survival` FR-075 continuity case. Guest
runtime-effect acceptance remains distinct Wave 6
`Provider/runtime-cloud-hypervisor` T384/T479/T480 work; Guest emission, status, or refusal
cannot satisfy T604. This host leg is W6 T604 operator acceptance, not Wave 5 evidence. Wave 5
retains only its production-plane prerequisites, the accepted double-opt-in contract
migration, and the settled T336-T355 W6 ownership. Full US1 completes only after
T479/T480 accept exact-F6 `Provider/runtime-cloud-hypervisor` evidence for a real Cloud
Hypervisor process effect, authenticated guest-control session, and ready Guest; missing,
skipped, status-only, fake-boundary, other-family, or refusal evidence leaves it incomplete.
Direct ResourceService calls, private reloads, and status-only effects do not satisfy T604.
After prospective T227 lands, the host configuration must set
`d2b.site.hostGenerationRebuildRef` to the exact `<flake-ref>#<configuration-name>` value. It
is required, has no default, and is limited to 2048 bytes. Use the real validated flake and
configuration values below; this procedure has no fixed illustrative target.

> **Blocked at this committed base.** The installed protocol-4 broker has no
> host-generation handoff operation, and the existing broker service cannot execute a
> target-closure compatibility binary before profile publication. Exact code-canon searches
> also find no `hostGenerationRebuildRef` option or carrier. Do not run migration or rollback
> until T221 passes and prospective T222/T227 merge.
>
> The source-floor schema, encoding, digest and signature rules, receipts, capability
> transitions, fixtures, poison registries, and transition matrices are owned solely by
> accepted Version 2 through `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and
> `VD2-SC002-TRACEABILITY`. T222 owns the typed handoff and T227 owns the option/carrier. A
> missing, stale, wrong-owner, or failing prospective row blocks T604. Retired T589/T592/T595
> text supplies no implementation or command.
>
After T222/T227 merge, the first 3/1-to-4/2 migration cannot read
the stable reference because only the target broker can publish it. The following is the
post-prerequisite operator contract named `host-generation-deploy-bootstrap-v1`, using the
deployment entrypoint from the explicit target configuration:

```bash
set -eu
LC_ALL=C
export LC_ALL
fail() { printf '%s\n' "$1" >&2; exit 2; }

[ -n "${D2B_HOST_FLAKE_REF:-}" ] ||
  fail 'set D2B_HOST_FLAKE_REF to the target flake ref without a # selector'
[ -n "${D2B_HOST_CONFIGURATION:-}" ] ||
  fail 'set D2B_HOST_CONFIGURATION to the target nixosConfigurations name'

case "$D2B_HOST_FLAKE_REF" in
  *[!A-Za-z0-9+._~:/?@%=\&,-]*|*'#'*)
    fail 'D2B_HOST_FLAKE_REF has invalid grammar'
    ;;
esac
case "$D2B_HOST_CONFIGURATION" in
  [!A-Za-z0-9]*|*[!A-Za-z0-9_-]*)
    fail 'D2B_HOST_CONFIGURATION has invalid grammar'
    ;;
esac
[ "${#D2B_HOST_CONFIGURATION}" -le 64 ] ||
  fail 'D2B_HOST_CONFIGURATION exceeds 64 bytes'
[ "$(id -u)" -ne 0 ] ||
  fail 'run authorization as the unprivileged d2b administrator, not root'

D2B_HOST_REBUILD_REF="${D2B_HOST_FLAKE_REF}#${D2B_HOST_CONFIGURATION}"
D2B_HOST_INSTALLABLE="${D2B_HOST_FLAKE_REF}#nixosConfigurations.${D2B_HOST_CONFIGURATION}.config.system.build.d2bHostGenerationDeploy"
[ "${#D2B_HOST_REBUILD_REF}" -le 2048 ] ||
  fail 'composed host generation rebuild reference exceeds 2048 bytes'

# Discard evaluator/build stderr. A flake may place secrets, credentials, host names,
# store paths, or arbitrary canaries there; this procedure never persists or prints it.

if ! D2B_EVALUATED_REF="$(nix eval --raw \
  "${D2B_HOST_FLAKE_REF}#nixosConfigurations.${D2B_HOST_CONFIGURATION}.config.d2b.site.hostGenerationRebuildRef" \
  2>/dev/null)"; then
  fail 'target hostGenerationRebuildRef evaluation failed; fix the target flake/configuration and retry'
fi
[ "$D2B_EVALUATED_REF" = "$D2B_HOST_REBUILD_REF" ] ||
  fail 'target hostGenerationRebuildRef does not match the validated parameters'

if ! D2B_DEPLOY_OUT="$(nix build --no-link --print-out-paths \
  "$D2B_HOST_INSTALLABLE" 2>/dev/null)"; then
  fail 'target deployment store object resolution failed; fix the target build and retry'
fi
case "$D2B_DEPLOY_OUT" in
  /nix/store/*) ;;
  *) fail 'target deployment store object resolution returned a non-store path' ;;
esac
case "$D2B_DEPLOY_OUT" in
  *'
'*|*[[:space:]]*)
    fail 'target deployment store object resolution returned more than one path'
    ;;
esac
D2B_TARGET_CLI="${D2B_DEPLOY_OUT}/bin/d2b"
[ -x "$D2B_TARGET_CLI" ] ||
  fail 'target deployment store object has no d2b executable'
D2B_INSTALLED_CLI="$(readlink -e \
  /run/current-system/sw/bin/d2b 2>/dev/null)" ||
  fail 'installed broker-managed d2b executable cannot be resolved'
case "$D2B_INSTALLED_CLI" in
  /nix/store/*/bin/d2b) ;;
  *) fail 'installed broker-managed d2b executable is not an immutable store object' ;;
esac

"$D2B_TARGET_CLI" host-generation authorize-handoff ||
  fail 'public-socket administrator authorization failed'
sudo -- "$D2B_INSTALLED_CLI" host-generation apply-authorized-handoff ||
  fail 'authorized host generation handoff failed'
```

`d2b host-generation apply-authorized-handoff` intentionally has no intent selector and no
authority token.
Every authorization/apply pair in this quickstart relies on one durable nonterminal intent
per source generation. Authorization takes the broker coordinator lock and refuses while an
authorized, claimed, mutating, recovery-pending, or transfer-pending intent exists. Apply
takes the same lock and atomically claims only the sole `authorized-pending` intent for the
accepted connection's kernel-derived peer pidfd/executable identity. Zero pending intents,
two pending intents, and a second concurrent apply connection refuse before mutation; there
is no oldest/newest fallback. A disconnect before the first mutation may release that exact
claim only after a durable zero-mutation proof. After any mutation, a replacement connection
is accepted only by coordinator replay of the same intent and the same pinned apply object
after the old peer is proven dead. Completion or rollback is terminal, so repeating apply
with no pending intent refuses and never reapplies.

Inspect the sole current-source handoff without an intent selector:

```bash
"$D2B_TARGET_CLI" host-generation inspect-authorized-handoff
```

The planned human result is the exact five-line
`HostGenerationHandoffStatusV1` projection from `data-model.md`; `--json` emits its closed
seven fields. It is serialized only from one of the exact validated tuples in
`data-model.md`, including separate source/target and active/failed broker variants. A failed
`transfer-pending` source owner projects `restart-existing-broker`, never the active wait
action. Active rollback projects `wait-for-broker-rollback`; failed rollback projects
`restart-existing-broker-for-rollback`. A valid `recovery-irreconcilable` state exists only
with a complete immutable-audit-backed rollback and advances only to `rolled-back`.

Selection uses the coordinator's authenticated current-intent pointer, not mtime or directory
order. `completed` and `rolled-back` remain selectable terminal projections until a new
authorization atomically installs the next `authorized-pending` pointer. Only an exact-empty
pointer census exits `3`; repairable absence and every invalid census exit `4`. Forbidden
syntax, selector/path/token input, an extra positional argument, or root
inspection exits `2`; invalid coordinator state or any incomplete rollback proof exits `4`
with zero mutation. The exact two-line human and four-field JSON error envelopes are in
`data-model.md`. Apply or broker recovery that races into a valid concurrent or terminal
state exits `4` with the same valid five-line or seven-field status through the shared
renderer. Neither projection exposes an intent, generation, pid, uid, store path, or
apply-peer identity.

A missing pointer is not one state. Exact-empty `clean-absence` exits `3` with
`action: begin-host-generation-deploy`; run the parameterized bootstrap above, named
`host-generation-deploy-bootstrap-v1`. Exactly one fully valid authenticated intent and
complete matrix with only its pointer absent is `repairable-absence`; inspect exits `4` with
`error: pointer-repair-required` and `action: repair-authorized-handoff`. Run the exact
selector-free unprivileged repair command:

```bash
"$D2B_TARGET_CLI" host-generation repair-authorized-handoff
```

It uses the existing public socket and broker coordinator. It may durably repair only a
uniquely reconstructible authenticated current-intent pointer and then prints the normal
five-line status. Competing, malformed, unauthenticated, orphaned, unknown, or incomplete
intent censuses are `invalid-coordinator`, exit `4`, and project
`action: preserve-and-escalate-invalid-coordinator`; do not run repair. The site security
authority owns the named external
`host-generation-invalid-coordinator-escalation-v1` procedure, which preserves the complete
coordinator/backup set and authorizes no mutation. Repair accepts only optional `--json`; any selector, path, token, root invocation,
extra positional argument, or `--force` exits `2` with
`action: repair-without-selectors` and zero mutation.

The broker records immutable `coordinator-pointer-repair` pre-mutation and outcome audit
members around direct-final no-replace pointer publication. A crash before the direct link
leaves the pointer absent; after it, restart accepts only absence or the exact complete
final and completes parent/audit durability. Repeating a completed repair is success with
zero write. A conflicting final is preserved and exits `4` with
`action: preserve-and-escalate-pointer-conflict`.

If one immutable rollback, audit, transition, or pointer-authentication member is absent or
mismatched, exit `4` identifies its bounded closed member and failure class with
`action: restore-immutable-audit-backup`.

1. Submit that exact fixed diagnostic to the disposition-pinned backup authority's named
   external `host-generation-immutable-audit-backup-acquisition-v1` procedure. It returns
   one canonical signed `HostGenerationImmutableAuditRestorationV1` as a regular single-link
   current-user mode-`0600` file no larger than 131,072 bytes. Do not edit or copy fields
   into a new JSON object.
2. Submit that artifact through the existing public socket as an unprivileged local Admin:

   ```bash
   "$D2B_TARGET_CLI" host-generation restore-immutable-audit-backup "$RESTORATION_ARTIFACT"
   ```

   The command accepts exactly one path and optional `--json`; it opens the file once
   no-follow and sends only its bounded canonical bytes. Launcher, workload, Zone,
   `HostShutdown`, root, nonmember, unauthenticated-local, direct-broker, and remote callers
   are denied. Signature validity does not authorize them.
3. Exit `0` prints the fixed restored/already-restored result and
   `action: rerun-repair-authorized-handoff`. Rerun the repair command above. Response-loss
   replay of the same artifact is `already-restored` with zero write. If the CLI loses the
   socket response entirely, it exits `4` and prints exactly
   `host generation handoff immutable audit restoration response lost` followed by
   `action: resubmit-same-restoration-artifact`; JSON is exactly
   `{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-response-lost","action":"resubmit-same-restoration-artifact"}`.
   This form has no failure class or settlement field. Immediately rerun the same command
   with the byte-identical artifact as the same unprivileged local Admin.

Zero/multiple paths or forbidden options use `restore-with-one-artifact` and the command
shape above. Root instead uses `use-unprivileged-local-admin-restoration-session`; the site
access administrator runs
`host-generation-unprivileged-local-admin-restoration-session-v1`, then the resulting
unprivileged local Admin reruns the same command with one artifact.
`use-local-admin-public-socket` maps to the site access administrator's named external
`host-generation-local-admin-session-v1` procedure. A closed artifact failure uses
`reacquire-immutable-audit-backup` and repeats step 1; a durable conflict uses the named
security escalation below. None authorizes a force or local file repair.

The broker appends restoration pre-mutation audit first, then publishes broker-private
non-observable preparatory signed evidence before digest/enum-only effective audit
provenance/restored-member and outcome/settlement records without
replacing an existing mismatched, unauthenticated, or noncontiguous member. Every append and
the repair/backup/dispatch/prune-audit records share one exact-final restart protocol.
Conflicting bytes are preserved. Invalid request, artifact, authorization, state-race, or
capacity refusals have zero coordinator/restoration/provenance mutation. Noncapacity
refusals and standing-reserve exhaustion also have zero audit mutation. An audited
retention-limit refusal writes only its capacity pre/outcome pair, leaves the reservation
ledger and covered restoration unchanged, and is the narrowly named
`RefusedZeroMutation`; standing-reserve exhaustion is a separate no-write pre-audit
admission refusal that keeps the same operation and generation for retry. An accepted
restoration attempt that later fails or conflicts records exactly one fixed outcome. A pre-only crash has no broker
response and remains blocked until the immediate byte-identical Admin resubmission above;
daemon restart does not settle it automatically and the operator does not wait for restart
settlement. A durable degraded settlement is nonterminal: after the named storage repair,
resubmitting the byte-identical artifact resumes the same operation and attempt and
converges to restored without duplicate provenance.

Restoration, durable-record, lifecycle, status, audit-edge, capacity, continuity,
redaction, and shrinkage fixture membership is owned only by generated
`VD2-SC002-REGISTRIES` and `VD2-SC002-TRACEABILITY` rows. This quickstart copies no ids,
counts, fixture contents, or transition matrix. Every assigned fixture remains independently
authored from production and no registry substitutes for another. Missing, stale,
runtime-derived, skipped, wrong-owner, non-ancestor, or failing coverage blocks the operation
with the generated remediation.

The Type-1 Nix
case proves only
rebuild-reference option grammar and cannot satisfy runtime recovery. The Type-10
`host-generation-handoff.nix` VM test alone proves real broker service failure/restart,
ownership transfer, mutation, rollback, and terminal selection.

Once the external prerequisite exists, the unprivileged invocation traverses the existing
public socket and its `SO_PEERCRED`/`d2b`-group Admin classification. It emits no authority
token. The installed source daemon forwards exactly one accepted-socket evidence fd over the
authenticated source channel only after both installed peers negotiate numeric protocol 4
plus the exact `source-handoff-v1` catalogue fingerprint, and the installed source broker
consumes it into one durably sealed nonfabricable handoff capability bound to the staged
intent. Bare protocol 4 or a source-peer fingerprint mismatch refuses.
It creates and retains the GC root and immutable identity for the exact target store object,
and separately pins the canonical identity and digest of `D2B_INSTALLED_CLI` from the
installed source generation. The caller-flake `D2B_TARGET_CLI` is executed only while
unprivileged.
The privileged command receives no flake URI, installable, reference path, target executable,
command, or argv to reevaluate; it can only ask the broker to resume the exact pinned intent.
Substituting either store executable, replacing the GC root, changing an installed symlink
after authorization, or replaying either executable for another intent refuses before
mutation. Effective uid 0, target-closure provenance, daemon identity, broker peer
credentials, and caller-supplied role claims never authorize independently.

The installed source compatibility broker reached through the existing
`d2b-priv-broker.service` and `d2b-priv-broker.socket` is the sole pre-transfer lifecycle
owner. Its ordinary
`serve` startup reopens the durable coordinator when the existing restart policy restarts it;
the entrypoint is never a supervisor. A capability-authorized source or target broker
performs every profile, service, 3/1 bootstrap, publication, and rollback mutation with
immutable pre-mutation and outcome audit. The broker durably owns the coordinator before the
first mutation. Coordinator ownership transfers exactly once to the authenticated target
broker before target daemon activation. The durable order is staged intent/capability,
target-object/GC-root/installed-apply-object pins, broker coordinator, source compatibility
actor, target broker, coordinator transfer, target daemon, Hello while unready,
phase-attenuated authenticated publication
request, broker-durable pointer/reference publication, daemon ingestion, then readiness.
Killing either entrypoint or the installed source compatibility actor cannot orphan the
coordinator.

The shell sends raw Nix stderr directly to `/dev/null` and emits only the fixed
stage-specific `fail` literals above; it creates no diagnostic file. The production
entrypoint instead permits at most 16,384 raw stderr bytes in memory, fails closed if that
ceiling is exceeded, drops all raw bytes before return, and emits only its fixed
identifier-free typed stage failure with remediation `rebuild-host-generation`. Neither path
forwards evaluator or builder stderr to human, JSON, wire, log, audit, metric, span, or
`Debug` output.

After the first successful publication, the installed entrypoint may use the stable reference:

```bash
set -eu
fail() { printf '%s\n' "$1" >&2; exit 2; }
[ "$(id -u)" -ne 0 ] || {
  printf '%s\n' 'run authorization as the unprivileged d2b administrator, not root' >&2
  exit 2
}
D2B_INSTALLED_CLI="$(readlink -e \
  /run/current-system/sw/bin/d2b 2>/dev/null)" ||
  fail 'installed d2b executable cannot be resolved'
case "$D2B_INSTALLED_CLI" in
  /nix/store/*/bin/d2b) ;;
  *) fail 'installed d2b executable is not an immutable store object' ;;
esac
"$D2B_INSTALLED_CLI" host-generation authorize-handoff \
  --from-reference /etc/d2b/host-generation-rebuild-ref ||
  fail 'stable reference validation or public-socket authorization failed; no privileged command was run'
sudo -- "$D2B_INSTALLED_CLI" host-generation apply-authorized-handoff ||
  fail 'authorized stable-reference handoff failed'

# Every exact acceptance resource must reach its owned effect and ready state
d2b resource list
d2b resource inspect Volume/acceptance-state
d2b resource inspect Network/acceptance-net
d2b resource inspect Device/acceptance-tpm
```

For every authorization/apply pair above and below, the unprivileged authorization command is
the complete preflight: it validates the flake/configuration or stable-reference grammar,
UTF-8 byte bounds, target identity, existence, immutable digest, and public-socket Admin
classification, then requires the broker to durably pin the exact executable store object
before returning success. The shell invokes `sudo` only after that success and only on the
exact immutable broker-managed apply object from the installed generation. Apply revalidates
both the target-object and apply-object pins and performs no evaluation or reference lookup.
The broker also binds the accepted apply connection's direct peer pidfd and live executable
identity to the apply-object pin immediately before each mutation; exit, exec, PID reuse,
start-identity mismatch, executable mismatch, or ambiguity refuses, and no pidfd is
persisted. Validation is the full cross-product: each of those six transitions is injected
in a fresh run before the first mutation and, after exactly the first mutation and its audit
become durable, immediately before each individual later mutation edge.

The mutation-edge, peer-transition, pre-start, unit-census, redaction, and source-floor
fixture sets are resolved only through the accepted generated `VD2-SC002-REGISTRIES` and
`VD2-SC002-TRACEABILITY` rows assigned prospectively to T222, T227, and T604. This quickstart does
not copy their ids, counts, ordering, or poison cases. The generated rows must name
independently authored expectations and enforcing gates; missing, duplicate, stale,
wrong-owner, non-ancestor, runtime-derived, skipped, or unvisited coverage fails closed.

Every apply connection still binds the live peer identity to the pinned apply object and
revalidates it before each mutation. A selected refusal leaves that mutation and all
successors unexecuted. Raw peer and executable identity remains absent from every observable
surface; only typed fixed correlation digests explicitly authorized by the generated contract
may survive, and metrics carry no peer-identity label.

**Expected**: all three exact resources are ready through their owned effects; removal of
`Device/acceptance-tpm` completes the pinned state-preserving cleanup; and FR-075 continuity
passes separately through T479 on the same candidate. Actionable refusal coverage runs
separately and cannot satisfy this positive proof. Guest passes through the distinct Wave 6
`Provider/runtime-cloud-hypervisor` T479/T480 exact-F6 acceptance. T604 remains W6
acceptance-only and consumes the Network implementation after authoritative T336-T355 merge.

This acceptance run fixes `isolation.allowEastWest = false`; it does not alone prove
Host/Network double opt-in. T221 requires the accepted Network contract/work-item amendment
on the exact fetched Wave 6 base. It must require
`effectiveEastWest = Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`,
default both inputs false, remove every current-facing sole Network-opt-in path, and
regenerate the manifest with T336-T355 retained as authoritative W6 implementation under
T221 and all four Network/Host production cases assigned there. T480 revalidates that
migration, implementation, and evidence before every Wave 6 close boundary. T604 and T479
require the merged W6 implementation and all four passing cases. Historical or current sole
opt-in cannot satisfy T221, T604, T479, or T480. Do not change feature status to bypass that
stop.

If migration rolls back to a 3/1 generation that had no stable reference, verified absence is
the correct restored state. The broker-owned durable coordinator resumes rollback after an
entrypoint crash. Before durable transfer only the matching externally installed source
broker may reopen it through the existing broker service; after transfer
the existing `d2b-priv-broker.service` reopens it even when target
daemon startup fails. No entrypoint process or extra unit must remain alive. Verify the
source state, then retry with the parameterized target command above; do not create the file
or copy the rolled-back target value into place. The census below queries the complete loaded
`d2b*`/`microvm*` namespace, excludes only canonical `d2b.slice`, and requires exactly the
three lifecycle units `d2bd.service`, `d2b-priv-broker.socket`, and
`d2b-priv-broker.service`. Code canon therefore produces four raw matched entries on a
conforming host - committed `d2b.slice` plus those three - despite the stale AGENTS.md
exit-criterion count of three. The comparison below is the canonical FR-075 predicate; this
feature batch does not edit AGENTS.md:

```bash
set -eu
LC_ALL=C
export LC_ALL
fail() { printf '%s\n' "$1" >&2; exit 2; }

systemctl is-active d2bd.service d2b-priv-broker.socket >/dev/null
D2B_EXPECTED_UNITS="$(printf '%s\n' \
  d2bd.service \
  d2b-priv-broker.service \
  d2b-priv-broker.socket | sort -u)"
if ! D2B_LOADED_UNITS="$(systemctl list-units --all --plain --no-legend --no-pager \
  'd2b*' 'microvm*')"; then
  fail 'failed to enumerate the loaded d2b/microvm unit namespace; repair systemd and retry'
fi
if ! D2B_FILTERED_UNITS="$(printf '%s\n' "$D2B_LOADED_UNITS" |
  awk '$1 != "d2b.slice" { print $1 }')"; then
  fail 'failed to filter the loaded d2b/microvm unit namespace'
fi
if ! D2B_ACTUAL_UNITS="$(printf '%s\n' "$D2B_FILTERED_UNITS" | sort -u)"; then
  fail 'failed to sort the loaded d2b/microvm unit namespace'
fi
[ "$D2B_ACTUAL_UNITS" = "$D2B_EXPECTED_UNITS" ] ||
  fail 'unexpected d2b/microvm lifecycle unit set after excluding d2b.slice; restore the required three-unit generation'
d2b vm status acceptance-vm
```

The host test repeats that exact census before VM start, after public start, after daemon
restart/adoption, and after public stop. Its fixture membership comes only from T604's
generated `VD2-SC002-REGISTRIES` and `VD2-SC002-TRACEABILITY` rows; this quickstart copies no
ids or counts. Every assigned injected unit survives the sole `d2b.slice` exclusion and fails
exact equality. Missing, runtime-derived, skipped, or unvisited coverage fails, so a transient
per-VM unit cannot hide between lifecycle observations.

To roll a successfully migrated host back to a prior validated configuration, set the prior
values explicitly and run that target through the same broker-owned path:

```bash
set -eu
LC_ALL=C
export LC_ALL
fail() { printf '%s\n' "$1" >&2; exit 2; }

[ -n "${D2B_ROLLBACK_FLAKE_REF:-}" ] ||
  fail 'set D2B_ROLLBACK_FLAKE_REF to the prior flake ref without a # selector'
[ -n "${D2B_ROLLBACK_CONFIGURATION:-}" ] ||
  fail 'set D2B_ROLLBACK_CONFIGURATION to the prior nixosConfigurations name'

case "$D2B_ROLLBACK_FLAKE_REF" in
  *[!A-Za-z0-9+._~:/?@%=\&,-]*|*'#'*)
    fail 'D2B_ROLLBACK_FLAKE_REF has invalid grammar'
    ;;
esac
case "$D2B_ROLLBACK_CONFIGURATION" in
  [!A-Za-z0-9]*|*[!A-Za-z0-9_-]*)
    fail 'D2B_ROLLBACK_CONFIGURATION has invalid grammar'
    ;;
esac
[ "${#D2B_ROLLBACK_CONFIGURATION}" -le 64 ] ||
  fail 'D2B_ROLLBACK_CONFIGURATION exceeds 64 bytes'
[ "$(id -u)" -ne 0 ] ||
  fail 'run authorization as the unprivileged d2b administrator, not root'
D2B_INSTALLED_CLI="$(readlink -e \
  /run/current-system/sw/bin/d2b 2>/dev/null)" ||
  fail 'installed broker-managed d2b executable cannot be resolved'
case "$D2B_INSTALLED_CLI" in
  /nix/store/*/bin/d2b) ;;
  *) fail 'installed broker-managed d2b executable is not an immutable store object' ;;
esac

D2B_ROLLBACK_REF="${D2B_ROLLBACK_FLAKE_REF}#${D2B_ROLLBACK_CONFIGURATION}"
D2B_ROLLBACK_INSTALLABLE="${D2B_ROLLBACK_FLAKE_REF}#nixosConfigurations.${D2B_ROLLBACK_CONFIGURATION}.config.system.build.d2bHostGenerationDeploy"
[ "${#D2B_ROLLBACK_REF}" -le 2048 ] ||
  fail 'composed rollback reference exceeds 2048 bytes'

if ! D2B_EVALUATED_ROLLBACK_REF="$(nix eval --raw \
  "${D2B_ROLLBACK_FLAKE_REF}#nixosConfigurations.${D2B_ROLLBACK_CONFIGURATION}.config.d2b.site.hostGenerationRebuildRef" \
  2>/dev/null)"; then
  fail 'prior hostGenerationRebuildRef evaluation failed; fix the prior flake/configuration and retry'
fi
[ "$D2B_EVALUATED_ROLLBACK_REF" = "$D2B_ROLLBACK_REF" ] ||
  fail 'prior hostGenerationRebuildRef does not match the validated parameters'

if ! D2B_ROLLBACK_OUT="$(nix build --no-link --print-out-paths \
  "$D2B_ROLLBACK_INSTALLABLE" 2>/dev/null)"; then
  fail 'prior deployment store object resolution failed; fix the prior build and retry'
fi
case "$D2B_ROLLBACK_OUT" in
  /nix/store/*) ;;
  *) fail 'prior deployment store object resolution returned a non-store path' ;;
esac
case "$D2B_ROLLBACK_OUT" in
  *'
'*|*[[:space:]]*)
    fail 'prior deployment store object resolution returned more than one path'
    ;;
esac
D2B_ROLLBACK_CLI="${D2B_ROLLBACK_OUT}/bin/d2b"
[ -x "$D2B_ROLLBACK_CLI" ] ||
  fail 'prior deployment store object has no d2b executable'

"$D2B_ROLLBACK_CLI" host-generation authorize-handoff ||
  fail 'public-socket rollback authorization failed'
sudo -- "$D2B_INSTALLED_CLI" host-generation apply-authorized-handoff ||
  fail 'authorized rollback handoff failed'
```

The host-integration acceptance executes the parameterized migration and rollback procedures,
rejects empty, malformed, over-bound, mismatched, and nonexistent flake/configuration inputs,
and rejects zero-output or multi-output target resolution
before public-socket authorization or `sudo`, kills the entrypoint at every post-staging
crash point, and requires the broker-owned coordinator to finish or roll back. It authorizes
one target store executable and one broker-managed installed apply executable, then
substitutes each independently, replaces the GC root, and changes the installed symlink
before apply; every substitution must refuse with no mutation, while the originally pinned
objects remain eligible. It injects target broker startup failure, target daemon
startup/reconciliation failure, entrypoint death, every installed source compatibility-actor crash
boundary, and both sides of durable ownership transfer. The existing
`d2b-priv-broker.service` must restart pre-transfer source-actor work and no systemd unit may be
added. Nix stderr canaries must be absent from every emitted diagnostic, log, audit, span,
wire response, and `Debug`; runnable shell examples discard raw Nix stderr without buffering
it, and production captures at most 16,384 raw bytes in memory, drops them before return, and
emits only the fixed error class and remediation.

### Story 1 - retire and restart

```bash
# Remove one resource from config, reactivate
set -eu
fail() { printf '%s\n' "$1" >&2; exit 2; }
[ "$(id -u)" -ne 0 ] || {
  printf '%s\n' 'run authorization as the unprivileged d2b administrator, not root' >&2
  exit 2
}
D2B_INSTALLED_CLI="$(readlink -e \
  /run/current-system/sw/bin/d2b 2>/dev/null)" ||
  fail 'installed d2b executable cannot be resolved'
case "$D2B_INSTALLED_CLI" in
  /nix/store/*/bin/d2b) ;;
  *) fail 'installed d2b executable is not an immutable store object' ;;
esac
"$D2B_INSTALLED_CLI" host-generation authorize-handoff \
  --from-reference /etc/d2b/host-generation-rebuild-ref ||
  fail 'stable reference validation or public-socket authorization failed; no privileged command was run'
sudo -- "$D2B_INSTALLED_CLI" host-generation apply-authorized-handoff ||
  fail 'authorized stable-reference handoff failed'
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

Do not triage or enter W8 until T556 has completed W7 merge, seal, ordered worktree/branch/
target/Nix-store cleanup, and the residue audit. T557 derives the terminal work set only from
that complete observed friction, and T558 starts from the resulting updated `v3` HEAD.

All six release-gate conditions, evaluated against the **final** candidate:

1. The five closing specs Accepted with evidence imported
2. Every DELETE and REPLACE row's removal proof passing **on the shipping tree**
3. The complete test matrix including manual hardware, live-host, and cloud tiers with
   recorded external evidence, plus the reset and cutover scenarios
4. Unanimous selected-roster panel, byte-identical PR merge, post-merge seal, and
   merge-eligibility on the W8 snapshot
5. A new `CHANGELOG.md` 3.0.0 version header, matching release-binary and flake package
   versions, with every wave and finding marker stripped; F8 contains either a complete
   matching prebuilt manifest or explicit `version: null`/`system: "x86_64-linux"`/
   empty-binaries source fallback,
   and the publication workflow is manual-only
6. Every prior wave's cleanup performed - no dangling worktrees or branches

Plus this program's own additions:

```bash
# Companion verification - exercise all four revision-2 rows on a live host
# d2b-toolkit, d2b-wlterm, d2b-wlcontrol, d2b-clip-picker
# weezterm is excluded by a recorded negative surface-consumption determination
```

After T561 merges without publishing, T573 resolves the current merged `v3` HEAD and proves
its tree equals the sealed F8 tree. It does not require the merged commit OID to equal the
sealed feature-tip commit OID. Publication binds to and builds from that merged `v3` HEAD;
the workflow repeats the merged-HEAD/sealed-tree, version, manifest/fallback, artifact-name,
embedded-version, and hash checks before tagging that HEAD. A push to `v3`, a mismatched
tree, or a post-tag manifest PR cannot publish or repair the immutable tag.

**Expected**: every companion works and T573 passes, or the release holds (FR-039, SC-024).

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
