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

Entry first requires FR-036's separate accepted Principle VI constitution amendment to be an
ancestor of the exact execution base. The feature-local W0/W1 historical record and W2-W4
remedial receipts cannot satisfy it. After that external prerequisite, entry requires Gate 0
passed, no unresolved contention flag on this wave's destination paths,
the stack proposed against the exact named parent commit rather than a stale `v3`, a free
heavy-gate semaphore, and a green fast hermetic suite. If the predecessor is not yet merged,
implementation may start only after at least five of its selected-roster reviews return and
integration is green on its converged tree. The candidate-bound selection uses the current
thirteen-seat role domain and may only widen over fix deltas. Prior-wave `Merged` state is not entry evidence. It is checked at
the successor's panel request, seal, and merge-eligibility boundary, after the successor
rebases onto the merged predecessor.

### 1b. Reconcile `adr046w5` progress before implementation

At one clean base, validate the accepted FR-036 predecessor and exactly one T072 disposition,
run cross-artifact analysis, and create one current selected-roster plan lifecycle for the
complete feature snapshot. Audit T073-T218 against commits and delivery records. If any row
is open, stop without changing a checkbox. If all rows are satisfied, submit one
`/d2b-spec-edit` batch that checks exactly T073-T218 and T603, then create one dedicated
checkbox-only commit. The editor batch receipt and that Git commit are the sole authority; do
not create a validator, changelog fragment, scratch receipt, sidecar, digest chain, or custom
resume state. Rerun analysis and a new selected-roster plan lifecycle on the changed commit
and snapshot before T589.

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
{T590,T592,T594,T605} -> T595 -> {T596,T597,T598,T599} ->
T220 -> F -> {T600,T601} -> T602 -> T219`.
T595 may not start until both serialized branches and the other completion slices converge and consumes T605's
`SystemCoreHost` and `SystemCoreUser` variants. T220 reconciles generated manifests and every
remaining content change before F through exactly one nonbinding lifecycle and one stable
discovery ledger. Every provisional candidate or fix reruns deterministic widen-only
selection and scoped verification; comprehensive discovery runs once and is never rerun.
T219 remains an external-disposition gate because Wave 5
already consumed its binding request; it performs no binding action. Until the
external delivery-contract/tooling owner lands the contract and typed validator for
`Wave5RetainedRequestDispositionV1`, and that validator imports one record bound to the
retained request and exact F, the actionable refusal is: `adr046w5 binding request already
consumed; obtain an accepted external delivery-contract/tooling disposition naming the
retained request, exact F, and one closed action`. `remain-blocked` stays blocked;
`abandon-without-merge` cannot advance; and `recover-panel-without-new-request` still requires
the complete unanimous selected-roster exact-F lifecycle from the current thirteen-seat role
domain, with selection allowed only to widen over fix deltas, before seal or merge. The record creates no
second request and is never panel sign-off or a constitutional waiver.

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

**`adr046w5` exception:** this subsection's `/d2b-panel-round work` instruction is forbidden
for this wave because its binding request is already consumed. Its only panel work is T220's
one nonbinding `/d2b-panel-round plan` lifecycle with one stable discovery ledger. For every
provisional candidate or fix, rerun deterministic selection, widen but never reduce the
roster, and run scoped verification with the ledger and full candidate. Run comprehensive
discovery exactly once. These iterations create no delivery request, reservation,
attestation, or seal. After T220 freezes F, run T600, T601, and T602, then stop for T219's
accepted external disposition. Do not fall through to section 5.

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
binding panel request on an ordinary unconsumed wave. For `adr046w5`, a defect found here
returns to T220, preserves its lifecycle and stable discovery ledger, reruns deterministic
widen-only selection and scoped verification without comprehensive discovery, freezes a
replacement F, reruns T600-T602, and stops again for the external disposition; no binding
panel is invoked.

### 5. Snapshot, validate, panel, seal

This procedure applies only to a wave whose binding request has not been consumed.
`adr046w5` MUST NOT execute any command in this subsection.

```bash
set -eu

CHECKOUT_ROOT="$(git rev-parse --show-toplevel)"
REPOSITORY="github.com/owner/repository"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/d2b/delivery"
BASE_OID="$(git merge-base v3 HEAD)"
HEAD_REF="$(git symbolic-ref --short HEAD)"
PR_NUMBER="$(gh pr view --json number --jq .number)"
PROGRAM="ADR046"
WAVE="W2"

: "${SELECTION:?set SELECTION to the candidate-bound lifecycle selection JSON}"
: "${RECORDS_DIR:?set RECORDS_DIR to the exact selected-roster record directory}"
: "${MERGE_TARGET_INPUT:?set MERGE_TARGET_INPUT to the current merge-target JSON}"

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

PANEL_REQUEST_RESULT="$("${X[@]}" panel-request \
  --snapshot "$SNAPSHOT" \
  --selection "$SELECTION" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
PANEL_REQUEST="$(printf '%s\n' "$PANEL_REQUEST_RESULT" | artifact_ref)"

PANEL_ATTEST_RESULT="$("${X[@]}" panel-attest \
  --snapshot "$SNAPSHOT" \
  --records "$RECORDS_DIR" \
  --repo "$REPOSITORY=$CHECKOUT_ROOT" \
  --state-dir "$STATE_DIR")"
PANEL_RECORDS="$(printf '%s\n' "$PANEL_ATTEST_RESULT" | artifact_ref)"

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

`history-proof` is **not** a separate subcommand; it runs inside `merge-eligibility`.

Panel lanes are exactly the read-only seats and profiles in the lifecycle selection artifact, dispatched on their recorded bindings
together in one message. They take no heavy-gate slot, so all selected lanes run
concurrently. They must not run tests or builds unless you explicitly ask a
specific lane to.

For ordinary waves, required CI, local/host validators, and panel lanes may run concurrently
against the snapshot. For `adr046w5`, T600/T601 evidence and T602's closed-set check complete,
then execution stops before T219 until the accepted external disposition exists. They do not
authorize another binding request.

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
re-snapshot, and rerun before requesting the panel. For `adr046w5`, the retained historical request already consumed the binding surface. F and
its evidence identity do not receive a request, and no candidate receives another one through
this feature. T220 may replace provisional candidates only during nonbinding
pre-request phase convergence within its one lifecycle and stable discovery ledger, with
deterministic widen-only reselection and scoped verification but no repeated comprehensive
discovery. After T602, stop until an accepted external disposition
preserves the consumed request and authorizes a specific non-request close action; never
silently re-attest changed content, waive findings, or infer successor admission. Any
authorized integration-lineage merge preserves F's tree.

### Recover an SC-002 sidecar incident

This quickstart does not restate the protocol. Recovery is unavailable until accepted
Version 2 `ADR-046-validation-and-delivery` and its generated
`ADR-046-validation-and-delivery-traceability.{json,md}` artifacts provide complete rows for
`VD2-SC002-INCIDENT`, `VD2-SC002-DISPOSITION`, and `VD2-SC002-RECOVERY`, and T589 installs
the commands those rows own. Before then, stop and do not infer a command from historical
feature prose.

After those gates pass, use only the exact invocation or versioned runbook anchor resolved by
the generated traceability row for the emitted action. A missing row, broken link, unknown
action, or action without an owned invocation is a release-blocking refusal. The runbook is
`docs/how-to/host-generation-recovery-v1.md`; T599 owns it and
`docs/reference/host-generation-recovery-actions-v1.json`, while T220 verifies complete
emitted-action coverage.

## Operator loop: prove the plane works

This is the loop that distinguishes a live control plane from a sealed wave. Its exact
operator activation positive remains W6 acceptance after T221. T220 first requires the
accepted external Network contract/work-item amendment to remove every current-facing sole
Network-opt-in path and retain T336-T355 plus all four double-opt-in cases as authoritative
W6 work. T604 then consumes their merged implementation. A stale sole-opt-in contract makes
T220 fail closed; an unimplemented T336-T355 row remains expected before W6 starts.

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
The host configuration must set `d2b.site.hostGenerationRebuildRef` to the exact
`<flake-ref>#<configuration-name>` value. It is required, has no default, and is limited to
2048 bytes. Use the real validated flake and configuration values below; this procedure has
no fixed illustrative target.

> **Blocked at this committed base.** The installed protocol-4 broker has no
> host-generation handoff operation, and the existing broker service cannot execute a
> target-closure compatibility binary before profile publication. Do not run migration or
> rollback until the accepted external compatibility disposition installs and validates
> `SourceGenerationCompatibilityFloorV1` on the source generation.
>
> The source-floor schema, encoding, digest and signature rules, receipts, capability
> transitions, fixtures, poison registries, and transition matrices are owned solely by
> accepted Version 2 through `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and
> `VD2-SC002-TRACEABILITY`. T589 and T592 consume only their generated rows. A missing,
> stale, non-ancestor, wrong-owner, or failing row blocks with remediation to accept Version 2,
> regenerate traceability, and pass Gate 0. Do not infer any field, count, or command from
> superseded feature-local prose. The accepted external disposition must name the source
> producer/installer and typed import/validation owners; no feature task substitutes for them.
>
After that prerequisite is accepted and installed, the first 3/1-to-4/2 migration cannot read
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
`VD2-SC002-TRACEABILITY` rows assigned to T589, T592, T595, and T604. This quickstart does
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
Host/Network double opt-in. Before T220 freezes F, the accepted external Network
contract/work-item amendment must require
`effectiveEastWest = Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`,
default both inputs false, remove every current-facing sole Network-opt-in path, and
regenerate the manifest with T336-T355 retained as authoritative W6 implementation under
T221 and all four Network/Host production cases assigned there. T219 revalidates that
migration and ownership before seal or merge. T604 and T479 later require the merged W6
implementation and all four passing cases. Historical or current sole opt-in cannot close
T070, T071, T220, T219, T604, or T479. Do not change feature status to bypass that stop.

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

Do not triage or enter W8 until T556 has completed W7 seal, merge, ordered worktree/branch/
target/Nix-store cleanup, and the residue audit. T557 derives the terminal work set only from
that complete observed friction, and T558 starts from the resulting updated `v3` HEAD.

All six release-gate conditions, evaluated against the **final** candidate:

1. The five closing specs Accepted with evidence imported
2. Every DELETE and REPLACE row's removal proof passing **on the shipping tree**
3. The complete test matrix including manual hardware, live-host, and cloud tiers with
   recorded external evidence, plus the reset and cutover scenarios
4. Unanimous selected-roster panel, seal, and merge-eligibility on the W8 snapshot
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
