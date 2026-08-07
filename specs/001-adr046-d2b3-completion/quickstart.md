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

Entry first requires FR-036's separate accepted Principle VI constitution amendment to be an
ancestor of the exact execution base. The feature-local W0/W1 historical record and W2-W4
remedial receipts cannot satisfy it. After that external prerequisite, entry requires Gate 0
passed, no unresolved contention flag on this wave's destination paths,
the stack proposed against the exact named parent commit rather than a stale `v3`, a free
heavy-gate semaphore, and a green fast hermetic suite. If the predecessor is not yet merged,
implementation may start only after at least 5 of its 10 reviews return and integration is
green on its converged tree. Prior-wave `Merged` state is not entry evidence. It is checked at
the successor's panel request, seal, and merge-eligibility boundary, after the successor
rebases onto the merged predecessor.

### 1b. Reconcile `adr046w5` progress before implementation

T603 never infers task completion from code presence and never edits a feature artifact
directly. First freeze clean pre-validator base A and feature snapshot P0, run
`/speckit-analyze`, and obtain unanimous plan signoff at A/P0. That pair authorizes exactly
three repository paths: the two Rust files `packages/xtask/src/delivery/mod.rs` and
`packages/xtask/src/delivery/resume.rs`, plus mandatory
`changelog.d/delivery-resume-reconciliation.md`. Land validator-and-fragment commit V with
sole parent A and exactly those three paths, freeze B exactly at V, require P to remain
byte-identical to P0, rerun analysis over
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
is clean C, the finalized receipt validates, all 147 checkboxes are checked, and fresh
analysis plus unanimous plan sign-off bind exact C/Q. The P-to-Q content change invalidates
B/P sign-off for implementation dispatch; any later content or history change invalidates
the C/Q gate. T602 later
validates C as an ancestor of separate final candidate F rather than requiring R to match F.

C1 is approved and fully assigned under Constitution 2.2.0. Run the pre-T603 analysis and
plan panel first. Implementation remains pending: after validator V, the post-T603 analysis
and plan panel must rerun before T603 may reconcile exactly T073-T218, then the fresh C/Q
analysis and plan panel must pass before T589; T605 remains future work after resume rather
than a 147th receipt row.

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
remaining content change before F. T219 remains an external-disposition gate because Wave 5
already consumed its binding request; it performs no binding action. Until the
external delivery-contract/tooling owner lands the contract and typed validator for
`Wave5RetainedRequestDispositionV1`, and that validator imports one record bound to the
retained request and exact F, the actionable refusal is: `adr046w5 binding request already
consumed; obtain an accepted external delivery-contract/tooling disposition naming the
retained request, exact F, and one closed action`. `remain-blocked` stays blocked;
`abandon-without-merge` cannot advance; and `recover-panel-without-new-request` still requires
the complete unanimous ten-role exact-F panel before seal or merge. The record creates no
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
repeatable, nonbinding `/d2b-panel-round plan` phase review. Those rounds create no delivery
request, reservation, attestation, or seal. After T220 freezes F, run T600, T601, and T602,
then stop for T219's accepted external disposition. Do not fall through to section 5.

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
binding panel request on an ordinary unconsumed wave. For `adr046w5`, a defect found here
returns to T220, reruns its nonbinding plan round, freezes a replacement F, reruns T600-T602,
and stops again for the external disposition; no binding panel is invoked.

### 5. Snapshot, validate, panel, seal

This procedure applies only to a wave whose binding request has not been consumed.
`adr046w5` MUST NOT execute any command in this subsection.

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
pre-request phase convergence. After T602, stop until an accepted external disposition
preserves the consumed request and authorizes a specific non-request close action; never
silently re-attest changed content, waive findings, or infer successor admission. Any
authorized integration-lineage merge preserves F's tree.

### Recover an SC-002 sidecar incident

> **Planned contract, not a command available at this committed base.** T589 owns these
> delivery subcommands, focused tests, generated help/schema goldens, and the
> operator-visible entry in its existing `changelog.d/resource-api-production.md` fragment.
> Before T589, a separate external amendment must bump accepted
> `ADR-046-validation-and-delivery` from Version 1 to Version 2, receive the required
> approvals, regenerate the spec-set/work-item/implementation-graph artifacts, and pass Gate
> 0 on an ancestor of T589's base. T589 does not own that amendment. Do not claim recovery
> until the external gate, T589, and T220's coordinated contract/changelog checks pass.

An identity-ambiguous sidecar is never unlinked. The parked candidate remains ineligible and
the durable incident remains retained. A parked incident is success-shaped only after its
immutable metadata, exact payload at
`evidence-sidecars/sc002/incidents/payload/sha256/<incident-id>.bin`, and append-only
`parked` status are file-and-ancestor-directory durable. A rename/reopen race remains
`recovery-pending`, preserves every still-named leaf, and exposes no parked status until
restart recovery or the owned recovery command finishes that protocol. Inspect still returns
the stable incident id, one closed cause, deterministic remediation, and the exact
human/JSON status. Recovery-pending is not a terminal cleanup result. A revalidated durable
payload outside both ephemeral namespaces plus `parked` is one terminal entry. If the
expected identity cannot be recovered, the authenticated disposition may instead retain
each currently named leaf by no-replace rename/reopen through durable incident residue
staging. Only an empty ephemeral/staging census, exact identity-derived residue census, and
synced `mismatch-retained` status form the other terminal entry. No path unlinks or restores
a suspect. Both terminal entries require the same external disposition and one distinct
successor snapshot:

```bash
set -eu
X="cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave"

$X sc002-incident-inspect \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID"

$X sc002-incident-recover \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID"

$X sc002-incident-apply \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID" \
  --disposition "$SC002_DISPOSITION"

$X sc002-successor-admit \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID" \
  --disposition-id "$SC002_DISPOSITION_ID" \
  --successor-snapshot "$SUCCESSOR_SNAPSHOT"
```

`SC002_INCIDENT_ID` and `SC002_DISPOSITION_ID` are stable lowercase 64-hex typed digests.
Exit `0` means the requested read or transition completed; `2` means invalid syntax or
malformed input; `3` means an ID was not found; and `4` means stale state, conflict, or
blocked admission. Repeating the exact already durable apply or successor admission exits
`0` without a write. Recovering an incident already at `parked` or later also exits `0`
without a write. A recovery that still cannot prove the metadata-bound move exits `4`,
preserves every name, prints the same cause/remediation status as inspect, and leaves
publication and close blocked. Applying the authenticated disposition may complete
no-unlink mismatch retention and then disposition validation. Changing any binding exits
`4`.

Human output is exactly these thirteen lines in this order:

```text
incident-kind: <INCIDENT_KIND>
incident-id: <INCIDENT_ID>
parked-candidate-id: <PARKED_CANDIDATE_ID>
parked-content-id: <PARKED_CONTENT_ID>
parked-snapshot-sha256: <PARKED_SNAPSHOT_SHA256>
state: <STATE>
cause: <CAUSE>
disposition-id: <DISPOSITION_ID_OR_NONE>
successor-candidate-id: <SUCCESSOR_CANDIDATE_ID_OR_NONE>
successor-content-id: <SUCCESSOR_CONTENT_ID_OR_NONE>
successor-snapshot-sha256: <SUCCESSOR_SNAPSHOT_SHA256_OR_NONE>
remediation: <REMEDIATION>
next-command: <NEXT_COMMAND>
```

The bracketed forms above denote bounded values. Null IDs render exactly `none`.
`STATE` is exactly `recovery-pending`, `parked`, `mismatch-retained`,
`disposition-validated`, or `successor-admitted`. `CAUSE` is one closed
`Sc002IncidentCauseV1` from `data-model.md`; it is never a path, errno, or free-form
sentence.
`NEXT_COMMAND` is static: `sc002-incident-apply` for either parked remediation,
`sc002-incident-recover` for `resume-incident-recovery`,
`sc002-successor-admit` for successor admission, and `none` after admission. It never
contains flags, IDs, paths, argv, an executable path, a shell fragment, or free-form
guidance. The equivalent `--json` output is the distinct version-1
`sc002-incident-cli-status` projection, not the persisted `sc002-incident-status` envelope.
JSON contains no `nextCommand` or guidance field. Its required final `remediation` field is
derived from the validated metadata/source/payload/residue prefix, durable status, and locked
disposition census and is exactly one of
`resume-incident-recovery`, `obtain-incident-disposition`,
`apply-incident-disposition`, `admit-successor`, or `none`; there is no free-form guidance
field. Persisted status has no remediation field.

`resume-incident-recovery` means run `sc002-incident-recover` with the stable incident id;
the command accepts no alternate source, payload, identity, disposition, successor, or
deletion selector.
`obtain-incident-disposition` means submit the stable incident id to the disposition
authority/workflow pinned by accepted Version 2 and receive its signed mode-`0600` record;
no repository command may mint or self-sign that authority record. Then run
`sc002-incident-apply`. `apply-incident-disposition` means run or rerun that command with the
already obtained record. On a nonrecoverable prefix it retains every current leaf in durable
incident residue, publishes `mismatch-retained`, and then validates the disposition; it never
unlinks.
`admit-successor` means run `sc002-successor-admit` with the disposition id and the fresh
successor snapshot. These are the only operator actions.

The disposition's only action is `abandon-candidate-admit-successor`. It cannot delete the
incident, make the parked candidate eligible, reuse its receipt/evidence, release a binding
reservation, or issue another binding request. The successor must have a distinct freshly
derived candidate/content/snapshot triplet and no copied SC-002 bytes. For `adr046w5`, this
admits only T220's nonbinding replacement-candidate and exact-candidate evidence flow while
preserving the retained request byte-for-byte; T219's external retained-request disposition
is still required. A consumed ordinary wave stops for its external wave disposition.
`SC002_DISPOSITION` is the exact canonical, signed `Sc002IncidentDispositionV1`; the apply
command trusts only the Version 2 contract's pinned authority and Ed25519 key, never a key
selected by the file. The cleanup refusal and each command return the same stable incident
id, cause, and remediation as bounded data fields. T589's existing
`changelog.d/resource-api-production.md` fragment names
all four command nouns, exits `0|2|3|4`, the disposition authority, and the fresh-successor
requirement; T220 verifies and folds that existing fragment rather than creating another.

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

The host leg declares Zone `acceptance` with the exact Wave 5 acceptance set -
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
runtime-effect acceptance is deferred specifically
to Wave 6 `Provider/runtime-cloud-hypervisor` T384/T479/T480; Guest emission, status, or
refusal is not a positive Wave 5 result.
This is a partial US1 production-plane checkpoint, not full US1 completion. The acceptance-set
label does not reassign Network implementation from Wave 4. Full US1 completes only after
T479/T480 accept exact-F6 `Provider/runtime-cloud-hypervisor` evidence for a real Cloud
Hypervisor process effect, authenticated guest-control session, and ready Guest; missing,
skipped, status-only, fake-boundary, other-family, or refusal evidence leaves it incomplete.
Direct ResourceService calls, private reloads, and status-only effects do not satisfy T604.
The host configuration must set `d2b.site.hostGenerationRebuildRef` to the exact
`<flake-ref>#<configuration-name>` value. It is required, has no default, and is limited to
2048 bytes. Use the real validated flake and configuration values below; this procedure has
no fixed illustrative target.

> **Blocked at this committed base.** The installed protocol-4 broker has no
> host-generation handoff operation, and `d2b-priv-broker.service` executes the installed
> generation's `brokerPackage`. The target closure cannot make its compatibility binary that
> service's executable before profile publication. Do not run the migration or rollback
> procedures below until an accepted external source-generation compatibility disposition
> has been installed on the source 3/1 host. That prerequisite must make the installed
> source daemon and broker negotiate numeric protocol 4 plus Hello
> `operation_catalogue_sha256` exactly equal to the `source-handoff-v1` operation-catalogue
> fingerprint, and must atomically install the exact nonempty 13-member
> `SourceGenerationCompatibilityFloorV1` census from `data-model.md`. Every closed role occurs
> once under one disposition and source generation; `missing`, `duplicate`, `extra`, `empty`,
> `stale-generation`, `stale-digest`, and `cross-disposition` members refuse. The accepted
> external disposition must name the producer/installer owner and the typed import/validation
> authority. Do not continue until its immutable manifest, installation, validation, and
> exact-C/Q import receipts form the accepted `SourceGenerationCompatibilityFloorV1`
> append-only chain. T589 and T592 consume that object read-only and no feature task creates
> or imports it. The separately accepted external
> `ADR-046-validation-and-delivery` Version 2 amendment owns the canonical encoding, complete
> digest/domain/framing registry, strict source-floor schemas, and exact
> `hash-vectors-v1.json` with 15 digest and four signature vectors. The 13 role/artifact
> rows, 91 member poisons, and five copied-issuer poisons are
> independently pinned. A proof is authority only when its authority digest, key digest,
> actual verifier key, signature domain, and signature all match the accepted disposition;
> copying expected digests into a chain signed by another valid key refuses after enclosing
> hashes and unaffected proofs validate.
> The source producer/installer and typed import/validation authority must conform to those
> artifacts; they may not redefine them in the compatibility disposition. The source
> broker's ordinary `serve` process under the existing
> `d2b-priv-broker.socket`/`d2b-priv-broker.service` pair consume exactly one accepted
> public-socket evidence fd, seal the typed authority, pin the target object, and pin one
> exact broker-managed privileged apply executable from the installed source generation.
> That immutable apply object, never an executable obtained from the caller's target flake,
> durably resumes the coordinator and transfers it to the target broker. The accepted external
> source-generation disposition owns that entire source set; T592 owns only target-v5
> adoption and target artifacts. Bare committed protocol 4 or a source-peer catalogue
> mismatch refuses. No target-only binary, new unit or override, child, mutating
> entrypoint, daemon recovery owner, serialized credential, or root/provenance claim
> substitutes. This quickstart claims no implementation of that prerequisite.

After that prerequisite is accepted and installed, the first 3/1-to-4/2 migration cannot read
the stable reference because only the target broker can publish it. The following is the
post-prerequisite operator contract using the deployment entrypoint from the explicit target
configuration:

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
D2B_DEPLOY_EXE="${D2B_DEPLOY_OUT}/bin/d2b-host-generation-deploy"
[ -x "$D2B_DEPLOY_EXE" ] ||
  fail 'target deployment store object has no deployment executable'
D2B_APPLY_EXE="$(readlink -e \
  /run/current-system/sw/bin/d2b-host-generation-deploy 2>/dev/null)" ||
  fail 'installed broker-managed apply executable cannot be resolved'
case "$D2B_APPLY_EXE" in
  /nix/store/*/bin/d2b-host-generation-deploy) ;;
  *) fail 'installed broker-managed apply executable is not an immutable store object' ;;
esac

"$D2B_DEPLOY_EXE" --authorize-handoff ||
  fail 'public-socket administrator authorization failed'
sudo -- "$D2B_APPLY_EXE" --apply-authorized-handoff ||
  fail 'authorized host generation handoff failed'
```

`--apply-authorized-handoff` intentionally has no intent selector and no authority token.
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

The host acceptance must race two authorization commands and two apply commands, inject an
otherwise impossible two-pending-intent census, disconnect before and after the first
mutation, and invoke apply after terminal completion. Exactly one contender may win only
when one pending intent exists. Every refusal has zero selected and successor mutations, and
post-mutation recovery resumes only the same durable intent.

Once the external prerequisite exists, the unprivileged invocation traverses the existing
public socket and its `SO_PEERCRED`/`d2b`-group Admin classification. It emits no authority
token. The installed source daemon forwards exactly one accepted-socket evidence fd over the
authenticated source channel only after both installed peers negotiate numeric protocol 4
plus the exact `source-handoff-v1` catalogue fingerprint, and the installed source broker
consumes it into one durably sealed nonfabricable handoff capability bound to the staged
intent. Bare protocol 4 or a source-peer fingerprint mismatch refuses.
It creates and retains the GC root and immutable identity for the exact target store object,
and separately pins the canonical identity and digest of `D2B_APPLY_EXE` from the installed
source generation. The caller-flake `D2B_DEPLOY_EXE` is executed only while unprivileged.
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
D2B_APPLY_EXE="$(readlink -e \
  /run/current-system/sw/bin/d2b-host-generation-deploy 2>/dev/null)" ||
  fail 'installed deployment executable cannot be resolved'
case "$D2B_APPLY_EXE" in
  /nix/store/*/bin/d2b-host-generation-deploy) ;;
  *) fail 'installed deployment executable is not an immutable store object' ;;
esac
"$D2B_APPLY_EXE" \
  --from-reference /etc/d2b/host-generation-rebuild-ref \
  --authorize-handoff ||
  fail 'stable reference validation or public-socket authorization failed; no privileged command was run'
sudo -- "$D2B_APPLY_EXE" \
  --apply-authorized-handoff ||
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

The mutation-edge registry is closed and ordered:

```text
host-generation.source-bootstrap-publish
host-generation.target-profile-publish
host-generation.target-broker-service-transition
host-generation.coordinator-transfer-to-target
host-generation.target-daemon-service-transition
host-generation.target-pointer-publish
host-generation.target-reference-publish
host-generation.target-pointer-repair
host-generation.target-reference-repair
host-generation.rollback-target-daemon-service
host-generation.rollback-pointer-restore
host-generation.rollback-reference-restore
host-generation.rollback-profile-publish
host-generation.rollback-source-broker-service
host-generation.rollback-source-daemon-service
```

The six transition ids are `peer-exit`, `peer-exec`, `peer-pid-reuse`,
`peer-start-identity-mismatch`, `peer-executable-identity-mismatch`, and
`peer-identity-ambiguity`. The pre-first matrix has exactly six ids
`apply-peer/pre-first/<transition>`. For every later edge, a fresh scenario executes the real
required prefix through the first durable mutation and audit, then injects immediately before
that named edge. Its id is `apply-peer/post-first/<edge>/<transition>`. The exact post-first
set is therefore 14 later edge ids times six transition ids, or 84 cases; a literal
independent expected-set fixture must match all 84 rather than deriving its expectation from
production enumeration. A second independent fixture pins the 15 ordered mutation ids, and a
third pins the closed post-first negative matrix. Every run proves the selected edge and all
successors remain unexecuted while the durable prefix and first mutation audit are unchanged.
Missing, extra, duplicate, unknown, reordered, dynamically omitted, or unvisited
edge/transition cases, selected-edge mutation, successor mutation, durable-prefix change,
or missing first audit fail the matrix.

Outside transient verifier-local kernel handles and bytes, raw peer PID/start and executable
store path, derivation name, NAR identity/hash, or executable content digest is forbidden in coordinator state,
receipts/evidence, human, JSON, wire, error/`Display`, log, tracing event/span, metric
name/label/value/exemplar, audit, panic, or `Debug` output. Persisted correlation contains
only typed fixed domain-separated digests, and metrics carry no raw or digested peer-identity
label or value. The exact seven-row literal canary registry in `data-model.md` injects PID,
start, store-path, derivation, NAR identity, NAR hash, and executable-content values one at a
time. Only that fixture and the test's private injection buffer are scan exclusions. Every
literal must be absent from every captured surface while the expected class-specific
correlation digests remain present where allowed. An empty, malformed,
over-bound, mismatched, changed, unreadable, or nonexistent input exits 2 with the named
remediation and runs no privileged command.

**Expected**: all three exact resources are ready through their owned effects; removal of
`Device/acceptance-tpm` completes the pinned state-preserving cleanup; and FR-075 continuity
passes on the same candidate. Actionable refusal coverage runs
separately and cannot satisfy this positive proof. Guest is not expected to pass until Wave 6
`Provider/runtime-cloud-hypervisor` and its T479/T480 exact-F6 acceptance exist. Network
remains Wave 4 implementation; Wave 5 accepts it through the production plane without taking
implementation ownership.

This acceptance run fixes `isolation.allowEastWest = false`; it does not prove or introduce
Host/Network double opt-in. The untouched external Network specification remains sole-opt-in
canon. W4 adjudication, T070, T071, and T220 stop until an accepted external versioned
correction/migration binds all four Network/Host cases or preserves sole Network opt-in and
leaves double opt-in unimplemented. Do not change feature status to bypass that stop.

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
D2B_APPLY_EXE="$(readlink -e \
  /run/current-system/sw/bin/d2b-host-generation-deploy 2>/dev/null)" ||
  fail 'installed broker-managed apply executable cannot be resolved'
case "$D2B_APPLY_EXE" in
  /nix/store/*/bin/d2b-host-generation-deploy) ;;
  *) fail 'installed broker-managed apply executable is not an immutable store object' ;;
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
D2B_ROLLBACK_EXE="${D2B_ROLLBACK_OUT}/bin/d2b-host-generation-deploy"
[ -x "$D2B_ROLLBACK_EXE" ] ||
  fail 'prior deployment store object has no deployment executable'

"$D2B_ROLLBACK_EXE" --authorize-handoff ||
  fail 'public-socket rollback authorization failed'
sudo -- "$D2B_APPLY_EXE" --apply-authorized-handoff ||
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
D2B_APPLY_EXE="$(readlink -e \
  /run/current-system/sw/bin/d2b-host-generation-deploy 2>/dev/null)" ||
  fail 'installed deployment executable cannot be resolved'
case "$D2B_APPLY_EXE" in
  /nix/store/*/bin/d2b-host-generation-deploy) ;;
  *) fail 'installed deployment executable is not an immutable store object' ;;
esac
"$D2B_APPLY_EXE" \
  --from-reference /etc/d2b/host-generation-rebuild-ref \
  --authorize-handoff ||
  fail 'stable reference validation or public-socket authorization failed; no privileged command was run'
sudo -- "$D2B_APPLY_EXE" \
  --apply-authorized-handoff ||
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
