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
the durable incident remains retained. Cleanup can open a sidecar namespace only while its
private `SidecarCleanupOwner<'guard>` exclusively borrows the exact
`CandidateSidecarGuard` that owns the locked OFD; a lock loser cannot fabricate either, and
the cleanup owner cannot outlive or be paired with another guard. A parked incident is success-shaped only after its
immutable structured `Sc002IncidentPreimageV1` with every applicable kind-specific
component is the complete write-ahead record. It is file-synced in an unnamed inode before
that exact opened inode is capability-free linked through a validated procfs fd directly to
the final no-replace name, and it is durably published before the kind-bearing anchor,
preimage-complete metadata, exact typed content-addressed
payload, and append-only
`parked` status are leaf-and-every-ancestor durable. A legacy source-copy or final-reopen
race is classified as
exactly `recovery-resumable` or `recovery-irreconcilable`, preserves every still-named leaf,
and exposes no parked status. Inspect returns the stable incident id, one closed cause,
deterministic remediation, and the exact human/JSON status for both variants. Recover is
advertised only when one preimage/anchor/metadata-bound next step is uniquely resumable.

Every durable anchor, metadata, status, resolution, successor freeze, request, disposition,
and admission record repeats the same structured preimage byte-for-byte. If the expected
identity cannot be recovered, authenticated apply leaves each representable currently named
leaf in place, direct-final publishes immutable incident residue copies, and binds every
retained name in the frozen primary-evidence scope; otherwise it uses a complete census or an
identity-bearing bounded-failure commitment. One byte grammar recursively records every
required `(root-code, root-instance-code)` pair and every descendant. It encodes absent,
directory, regular-file, symlink, block-device, character-device, fifo, socket, mount, and
other observations injectively, including owner, group, `st_rdev`, and symlink-target
payload identity. Invalid representable kinds remain evidence and never masquerade as
absent. Unavailable observations are private denied scope only; all-zero `0xff` is rejected
from every serialized body. The failure form embeds the full stable ordered node sequence plus the fixed
root-instance, canonical failing-path digest, saturated counts, and equal before/after
recursive identities. That scope excludes every resolution,
resolution-evidence, successor-freeze, disposition-request, and disposition leaf. A raw `01ff` sentinel never authorizes apply or
successor admission. Invalid, unstable, or over-bound primary census state remains
actionable. A stable semantic failure with full coverage exposes only the typed evidence
kind and digest needed by the external disposition authority. Unreadable, unstable,
depth-65, node-hard-ceiling, or byte-hard-ceiling scope instead exposes null evidence and
the closed `restore-primary-evidence-coverage` remediation; request exits `4` with the same
status until read access is restored, the writer is stopped, or an injected unrecognized
over-ceiling entry is moved outside the immutable candidate scope. Recognized evidence is
never removed. Apply publishes and syncs only exact admission-capable canonical evidence
bytes outside the frozen scope; successor admission recursively replays the same scan and
revalidates those bytes, the current
scope identity, complete preimage, and incident/parked binding. A stale or copied commitment exits `4` with
a fresh inspect projection and no write.

Only a direct-final ordinary terminal requires an empty ephemeral/staging census. A
residue-backed terminal entry instead requires the terminal legacy incident's exact frozen
recursive census retaining every original legacy source name at its frozen locator, the
exact identity-derived residue census, and synced `mismatch-retained` status. A
resolution-backed terminal likewise carries its exact frozen retained-name census and has
the exact durable evidence object plus append-only `disposition-validated`. No path unlinks
or restores a suspect. Every branch requires the same external disposition and one distinct
successor snapshot. The successor is frozen
before signing, and the canonical request is the only authority input:

```bash
set -eu
X="cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave"

$X sc002-incident-inspect \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID"

$X sc002-incident-recover \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID"

$X sc002-disposition-request \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID" \
  --successor-snapshot "$SUCCESSOR_SNAPSHOT" \
  --request-out "$SC002_DISPOSITION_REQUEST"

# Submit exactly "$SC002_DISPOSITION_REQUEST" to the Version-2-pinned authority workflow.

$X sc002-incident-apply \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID" \
  --disposition "$SC002_DISPOSITION" \
  --successor-snapshot "$SUCCESSOR_SNAPSHOT"

$X sc002-successor-admit \
  --snapshot "$PARKED_SNAPSHOT" \
  --incident-id "$SC002_INCIDENT_ID" \
  --disposition-id "$SC002_DISPOSITION_ID" \
  --successor-snapshot "$SUCCESSOR_SNAPSHOT"
```

The request file is the exact 19-field `Sc002IncidentDispositionRequestV1`, not inspect JSON
and not a caller-written disposition prefix. The authority substitutes only the envelope
kind, copies the semantic fields and freeze digest, omits the verified embedded freeze,
derives the request digest, inserts the pinned authority/key fields, and signs the exact
22-field disposition. Before candidate publication, `--request-out` resolves the output parent with anchored
openat2, verifies the privilege-dropped caller has no effective capabilities, validates a
retained procfs `/proc/self/fd` directory fd plus the exact target mount/filesystem, and
writes, file-syncs, and revalidates an unnamed `O_TMPFILE` inode. Unsupported open or an
invalid procfs/mount environment refuses with zero output name and zero freeze/request
mutation. Candidate-internal freeze/request durability then completes. The command links
the exact opened inode directly to the final no-replace name with capability-free
fd-relative `linkat(..., AT_SYMLINK_FOLLOW)`, verifies the final inode and bytes, and syncs
the parent. It never uses `AT_EMPTY_PATH`, a linked temporary, a name-consuming rename, or a
create-and-unlink preflight. Every fd is CLOEXEC. Unsupported linking after internal
durability is an ordinary replayable output failure: no output name is created and the
internal freeze/request remains. A crash before the direct link exposes no output name; a
crash after it may expose only the complete final inode. Exact replay revalidates or
recreates that final without truncating or replacing a foreign leaf.

The transition is closed and operational:

| Inspect state/cause | Required action | Successful next state |
| --- | --- | --- |
| `recovery-resumable` | run `sc002-incident-recover` | `parked`, `mismatch-retained`, `disposition-validated`, or `successor-admitted`, whichever is the maximal uniquely reconstructible contiguous branch |
| `recovery-irreconcilable`, including `evidence-census-conflict`, without a disposition matching the current typed evidence digest | run inspect with `--json`, run `sc002-disposition-request` with the clean successor snapshot, submit that exact request, then run `sc002-incident-apply` with the same snapshot | `disposition-validated` |
| exact `primary-evidence-coverage:<failure-class>:<root-class>` | run the mapped owner repair procedure, then rerun `sc002-incident-inspect`; do not run disposition request while this cause remains | unchanged with null evidence until two complete equal walks succeed; only a later `obtain-incident-disposition` projection permits the request |
| `recovery-irreconcilable` with the matching signed disposition already durable | rerun `sc002-incident-apply` with the freeze-bound successor snapshot | `disposition-validated` |
| `parked` without a matching disposition | freeze the successor and create/submit the canonical request, then run `sc002-incident-apply` with that same successor snapshot | `disposition-validated` |
| `mismatch-retained` or a crash after disposition publication | rerun `sc002-incident-apply` with the freeze-bound successor snapshot | `disposition-validated` |
| `disposition-validated` | run `sc002-successor-admit` with the same freeze/request-bound successor snapshot | `successor-admitted` |
| `successor-admitted` | inspect only | unchanged idempotent terminal |

If the locked primary-evidence digest changes between inspect and apply, apply exits `4`,
returns the new inspect projection, and requires a newly bound disposition. No generic retry,
force flag, deletion selector, or alternate path is accepted.
The test contract compares independently authored literal expectations with all 61 receipt,
73 malformed retired/primary-census, and 35 direct-final publication ids from
`data-model.md`, scans all
seventeen SC-002 recovery redaction canaries, and uses the shared
nineteen-digest/one-signature SC-002 golden. A generated expected set or a digest copied from
production is not evidence.

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
`STATE` is exactly `recovery-resumable`, `recovery-irreconcilable`, `parked`,
`mismatch-retained`, `disposition-validated`, or `successor-admitted`. `CAUSE` is one closed
`Sc002IncidentCauseV1` from `data-model.md`; it is never a path, errno, or free-form
sentence. Coverage denial uses the exact bounded
`primary-evidence-coverage:<failure-class>:<root-class>` cause, so unreadable, unstable,
depth, node-ceiling, and byte-ceiling failures are not collapsed. The root class is one
closed non-path class and exposes no source-slot instance or raw identity.
`NEXT_COMMAND` is static: `sc002-disposition-request` when a disposition must be obtained,
`sc002-incident-apply` when a matching signed disposition is already durable,
`sc002-incident-recover` for `resume-incident-recovery`,
`sc002-successor-admit` for successor admission, and `none` after admission or while
primary-evidence coverage repair is required. It never
contains flags, IDs, paths, argv, an executable path, a shell fragment, or free-form
guidance. The equivalent `--json` output is the distinct version-1
`sc002-incident-cli-status` projection, not the persisted `sc002-incident-status` envelope.
JSON contains no `nextCommand` or guidance field. Its required final `remediation` field is
derived from the validated metadata/source/payload/residue prefix, durable status, and locked
disposition census and is exactly one of
`resume-incident-recovery`, `obtain-incident-disposition`,
`restore-primary-evidence-coverage`, `apply-incident-disposition`, `admit-successor`, or
`none`; there is no free-form guidance field. For an irreconcilable state it also carries nullable
`resolutionEvidenceKind` and `resolutionEvidenceSha256`; these are bounded typed values, not
a raw locator or evidence bytes. Persisted status has no remediation field but does persist
the complete incident-id preimage; CLI output does not expose that preimage.

`resume-incident-recovery` means run `sc002-incident-recover` with the stable incident id;
the command accepts no alternate source, payload, identity, disposition, successor, or
deletion selector.
`obtain-incident-disposition` means run inspect with `--json`, then run
`sc002-disposition-request` with one clean successor snapshot. That command durably freezes
the derived successor candidate/content/snapshot triplet, writes the canonical mode-`0600`
request, and cannot sign it. Submit exactly that request to the disposition
authority/workflow pinned by accepted Version 2 and receive its signed mode-`0600` record;
inspect output alone and a caller-written triplet are not signing requests. Then run
`sc002-incident-apply` with the same successor snapshot.
`apply-incident-disposition` means run or rerun that command with the already obtained record
and freeze-bound snapshot. On a nonrecoverable prefix it retains every current leaf in durable
incident residue and publishes `mismatch-retained`, or publishes the complete census or
bounded-failure commitment and the separate resolution status; it never unlinks.
`admit-successor` means run `sc002-successor-admit` with the disposition id and the same
freeze/request-bound successor snapshot. These are the only operator actions.

`restore-primary-evidence-coverage` is not permission to request a disposition. It exits
`4`, emits null resolution evidence and `next-command: none`, and selects one exact
owner-run repair procedure from the failure class:

| Failure class | Required procedure | Recheck |
| --- | --- | --- |
| `enumeration-unavailable` | registered root owner runs `restore-primary-evidence-access` and restores the prior owner/mode/mount read and execute contract without editing recognized evidence | rerun `sc002-incident-inspect` |
| `identity-unstable` | registered root owner runs `quiesce-primary-evidence-writer` and stops the non-d2b writer while leaving recognized evidence byte-identical | rerun `sc002-incident-inspect` |
| `depth-limit` | registered root owner runs `relocate-unrecognized-primary-evidence-subtree` for only the injected depth-65 subtree | rerun `sc002-incident-inspect` |
| `node-hard-ceiling` | registered root owner runs `relocate-unrecognized-primary-evidence-subtree` for only injected unrecognized entries until the complete walk fits 4,096 nodes | rerun `sc002-incident-inspect` |
| `byte-hard-ceiling` | registered root owner runs `relocate-unrecognized-primary-evidence-subtree` for only injected unrecognized entries until the complete walk fits 67,108,864 bytes | rerun `sc002-incident-inspect` |

If the named owner cannot complete the procedure, escalate to that owner with the stable
incident id and bounded root class. Do not run `sc002-disposition-request` until a later
inspect projection changes to `obtain-incident-disposition`.

The disposition's only action is `abandon-candidate-admit-successor`. It cannot delete the
incident, make the parked candidate eligible, reuse its receipt/evidence, release a binding
reservation, or issue another binding request. The successor must have a distinct freshly
derived candidate/content/snapshot triplet and no copied SC-002 bytes. The freeze, unsigned
request, signed disposition, apply, and admission all bind that exact triplet; changing the
snapshot after signing exits `4` without a write. For `adr046w5`, this
admits only T220's nonbinding replacement-candidate and exact-candidate evidence flow while
preserving the retained request byte-for-byte; T219's external retained-request disposition
is still required. A consumed ordinary wave stops for its external wave disposition.
`SC002_DISPOSITION` is the exact canonical, signed `Sc002IncidentDispositionV1`; the apply
command trusts only the Version 2 contract's pinned authority and Ed25519 key, never a key
selected by the file. The cleanup refusal and each command return the same stable incident
id, cause, and remediation as bounded data fields. T589's existing
`changelog.d/resource-api-production.md` fragment names
all five command nouns, exits `0|2|3|4`, the disposition authority, and the pre-signing
successor-freeze/request requirement
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
> rows, 91 member poisons, five copied-issuer poisons, 26 issuer-authentication/capability
> negatives, 21 hash-vector negatives, and 32 receipt/transition negatives are
> independently pinned. A proof is authority only when its authority digest, key digest,
> actual verifier key, signature domain, and signature all match the accepted disposition;
> copying expected digests into a chain signed by another valid key refuses after enclosing
> hashes and unaffected proofs validate and cannot produce private authority. The installed
> source coordinator acquires the exact origin record under one exclusive OFD claim into one
> nonserializable, non-clonable `ProtectedSourceFloorOrigin` without durable consumption.
> The disposition-pinned validator consumes that process-local owner while creating private
> `AuthenticatedSourceFloorIssuerProvenance`, then consumes the
> intermediate by value to create the separate private
> `ValidatedSourceGenerationCompatibilityFloor`; direct DTO decode, copied digest tuples,
> serialization, clone/copy, concurrent origin replay, or a repeated validator call cannot
> create another result. Durable consumption commits only with atomic durable dispatch
> publication. Failure or owner death before publication permits exact-origin reacquisition
> after proving no dispatch exists; restart after publication resumes without another mint.
> Later handoff boundaries borrow and attenuate that one result and never revalidate
> serialized floor evidence.
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

Inspect the sole current-source handoff without an intent selector:

```bash
"$D2B_DEPLOY_EXE" --inspect-authorized-handoff
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
"$D2B_DEPLOY_EXE" --repair-authorized-handoff
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
   "$D2B_DEPLOY_EXE" --restore-immutable-audit-backup "$RESTORATION_ARTIFACT"
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

The unchanged 168-case broker registry owns only its literal caller, nineteen request-shape,
artifact/binding, conflict, legacy backup/restoration publication, and no-write cases. The
mandatory independent 216-case durable-record/boundary registry adds every listed
amendment record class at every publication boundary, including reservation, both releases,
settlement, repair-resume, and continuity-repair pre/evidence/watermark/outcome. The
mandatory independent 88-case lifecycle registry adds aggregate and continuity-evidence
limits, all five standing-reserve states, cycle-unique capacity success/refusal/retry,
malformed-prefix hooks and poisons for both release machines and continuity ordering,
retention-anchor conflict, continuity evidence export/compaction, continuity and permit
seals, transport-response-loss recovery, private-identifier/body canaries, and shrinkage
poisons. None of these three registries substitutes for another.

The broker-private linear `HostGenerationImmutableAuditBackupOwner` retains at most 256
members and 16,777,216 encoded bytes per intent and at most 64 intents, 4,096 members, and
268,435,456 bytes at the root. It reserves capacity before handoff, never prunes the current
intent, and uses a checked durable clock epoch plus sealed typed prune op and immutable
pre/outcome audit from day 30 through the hard day-90 deadline. A prune, bound, clock, or
settlement failure is a typed actionable degraded result that blocks later mutation. An unaudited extra mutation instead
reports `action: preserve-and-escalate-audit-integrity-incident`; the site security authority
runs `host-generation-audit-integrity-escalation-v1`, preserves the coordinator and backup
artifacts, and does not attempt restoration. Pointer and restoration conflicts similarly
map to `host-generation-pointer-conflict-escalation-v1` and
`host-generation-audit-restoration-conflict-escalation-v1`. None of these external
procedures authorizes retry, copy, replace, delete, or force. There is no force flag,
generic copy procedure, new unit, or daemon recovery owner.

The host acceptance must race two authorization commands and two apply commands, inject an
otherwise impossible two-pending-intent census, disconnect before and after the first
mutation, and invoke apply after terminal completion. Exactly one contender may win only
when one pending intent exists. Every refusal has zero selected and successor mutations, and
post-mutation recovery resumes only the same durable intent. Hermetic Rust tests own tuple
validation, exact human/JSON/error goldens, forbidden inspect and repair inputs, pointer
selection, and the exact independent seven-member, 32-audit-member, 15-transition-edge
rollback matrices. Their 156-case registry covers every missing and mismatched member, each
changed transition edge, unaudited extra mutation, unauthenticated pointer, every repair
restart/conflict/no-write case, and all four shrinkage meta-negatives, plus exact successful
pointer-repair, repairable-absence, bounded audit-restoration, and integrity-incident
goldens. The separate two-row restoration and two-row prune audit-edge fixtures plus the unchanged
168-case broker registry prove their literal privileged restoration cases. The mandatory
216-case record-boundary and 88-case lifecycle registries prove the supplemental
publication, capacity, continuity, capability, taxonomy, and redaction obligations; neither
the 156-case status registry nor the 168-case registry can substitute. The Type-1 Nix
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

The post-first negative registry is exactly these 15 ids, independently literal in both the
fixture and test constant:

```text
post-first-negative/missing-edge
post-first-negative/duplicate-edge
post-first-negative/unknown-edge
post-first-negative/reordered-edge
post-first-negative/empty-edge-set
post-first-negative/missing-transition
post-first-negative/duplicate-transition
post-first-negative/unknown-transition
post-first-negative/unvisited-case
post-first-negative/dynamic-case-skipped
post-first-negative/verification-hook-missing
post-first-negative/selected-edge-mutated
post-first-negative/successor-mutated
post-first-negative/durable-prefix-changed
post-first-negative/first-audit-missing
```

The exact fixture, literal constant, 15-edge fixture, 90-case fixture, three edge
meta-poisons, and production enumerator are mutually read-independent. An empty set,
runtime-derived count, skipped visit, or early failure cannot satisfy a negative.

The separate literal `host-generation-pre-start-case-ids.txt` runs before the first mutation:
one unprivileged positive, root refusal for bootstrap/stable-reference/rollback, apply
without authorization, and apply before each source daemon/broker/Hello/catalogue/capability/
target pin/apply pin/GC-root/coordinator/existing-unit prerequisite. All fifteen ids must run
with zero mutation. The source-floor `poison-case-ids.txt` is the exact 91-line list printed
in `data-model.md`; neither it nor the separately literal 90 apply-peer ids is formed by a
runtime Cartesian product.

Outside transient verifier-local kernel handles and bytes, every raw value in apply-peer
admission and identity verification is forbidden in coordinator state,
receipts/evidence, human, JSON, wire, error/`Display`, log, tracing event/span, metric
name/label/value/exemplar, audit, panic, or `Debug` output. Persisted correlation contains
only typed fixed domain-separated digests, and metrics carry no raw or digested peer-identity
label or value. The exact fifteen-row literal canary registry in `data-model.md` injects
pidfd number, PID, start identity, socket uid/gid, cgroup/proc paths, executable store path,
derivation, NAR identity/hash, content digest, and device/inode/mount identity one at a time.
Only that fixture and the test's private injection buffer are scan exclusions. Every
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

The host test repeats that exact census before VM start, after public start, after daemon
restart/adoption, and after public stop. Its independent 27-id unit registry retains the
positive/enumeration/empty/missing/service/socket/slice/path/timer/template/instance/
malformed/skip cases and adds d2b target/template/instance plus microvm
socket/slice/target/path/timer poisons. Every injected unit survives the sole `d2b.slice`
exclusion and fails exact equality. A transient per-VM unit therefore cannot hide between
lifecycle observations.

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
# Companion verification - exercise all four revision-2 rows on a live host
# d2b-toolkit, d2b-wlterm, d2b-wlcontrol, d2b-clip-picker
# weezterm is excluded by a recorded negative surface-consumption determination
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
