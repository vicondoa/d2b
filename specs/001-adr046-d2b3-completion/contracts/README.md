# Contracts: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

d2b exposes contracts to operators, to guest components, to Nix consumers, and to sibling
desktop companions. This directory records **which contract surfaces this program must
deliver, change, or retire**, and what "done" means for each.

These files are an index and an obligation list. They do not restate field-level schemas: the
normative definitions live in the ADR-046 specification set, and the machine-readable form is
generated into `docs/reference/schemas/v3/` by `xtask`. Duplicating them here would create a
third source of truth that the drift gates do not check.

## Contract surfaces

| File | Surface | Consumers | Wave |
| --- | --- | --- | --- |
| [resource-api.md](./resource-api.md) | The `d2b.resource.v3` service and ComponentSession admission | In-Zone components, controllers, Providers | W2-W5 |
| [operator-cli.md](./operator-cli.md) | The `d2b` command surface | Human operators, companion tools | W5 |
| [nix-configuration.md](./nix-configuration.md) | `d2b.zones.<zone>.resources.*` option schema | Host configurations | W2, W5 |
| [generated-artifacts.md](./generated-artifacts.md) | Schemas, per-Zone bundles, UI colors, delivery artifacts | Broker, daemon, companions, drift gates | W2-W7 |
| [companion-contracts.md](./companion-contracts.md) | What the four revision-2 desktop companions consume | Sibling repositories | W5 publish, W8 verify |
| [Candidate recovery prerequisite v1](#candidate-recovery-prerequisite-v1) | Immutable-candidate failure closure and external disposition | Plan integrator, delivery tooling, panel process | Historical W2 entry; requalified at close if unattested |
| [ADR-046 Wave 6 historical predecessor guard](#adr-046-wave-6-historical-predecessor-guard) | Exact one-time Wave 5 historical disposition | T221 and delivery snapshot/panel/seal/eligibility | W6 entry and close |

## Candidate recovery prerequisite v1

<!-- RETIRED-READONLY-BEGIN -->

**Contract id**: `adr046-candidate-recovery-prerequisite/v1`

**Owner**: T008 and the ADR046 plan integrator

**Status**: historical W2 entry attestation; fail-closed remedial disposition required before
W2 close when contemporaneous entry evidence is absent

The feature-local sequencing contract is:

1. one immutable candidate receives at most one binding request;
2. a nonunanimous candidate is durably closed as failed and retains its request and records;
3. the active candidate slot is not released until that closure is durable;
4. no same-wave successor may receive another binding request; an accepted external
   disposition is required and may authorize only a non-request close action; and
5. same-candidate retry, two active candidates, stale or cross-wave recovery evidence, and
   any post-request content, history, or evidence move fail closed.

One canonical hermetic `candidate_recovery_v1` validator owns those five invariants and both
entry-disposition receipt shapes. Historical receipts contain exactly `historicalTask`,
`entryBaseCommit`, `entryBaseTree`, `firstDispatchCommit`, `recordedAtUnix`, the closed
prerequisite-locator set, and the closed original-check result set. Remedial receipts contain
exactly `historicalTask`, `historicalEntry: "unproven"`, `candidateId`, `candidateCommit`,
`candidateTree`, `recordedAtUnix`, the closed prerequisite-locator set, and the closed current
requalification-check result set. The validator rejects an unknown task, unknown or duplicate
field, duplicate receipt, both receipt variants, neither receipt variant, a missing or failed
check, a non-ancestor prerequisite or implementation head, a wrong entry/candidate
commit/tree, and a historical label attached to current evidence.

Its table-driven suite starts with one valid state and one valid receipt variant, then changes
exactly one field, binding, or transition at a time. The case inventory is itself asserted and
must cover every required and forbidden field in both receipt variants; candidate, program,
wave, request, round, commit, tree, recommendations, convergence, and validation bindings;
same-candidate second request; alternate candidate while active; release before durable
failure closure; successor admission before or after durable release; missing or stale recovery
evidence; cross-candidate and cross-wave evidence; and post-request content, history, or
evidence movement. A prose assertion or one happy-path test is not this matrix.

This contract does not amend the external ADR or tooling by assertion. Before T008 may
complete, the ADR046 plan integrator owns a separate external scope escalation that must
merge all of the following as one accepted policy generation:

- a new or superseding ADR plus its `docs/adr/README.md` index row;
- the coordinated `docs/specs/ADR-046-validation-and-delivery.md` and generated spec-manifest
  amendment;
- delivery implementation and tests under `packages/xtask/src/delivery/`; and
- matching `AGENTS.md` and `docs/contributing/` panel/delivery guidance.

T008's evidence record must name accepted commit locators for all four scopes (locators may
coincide when one commit owns multiple scopes) and prove each is an ancestor of its W2 base.
It must also record successful, nonempty execution of:

```bash
set -euo pipefail
listed="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  candidate_recovery_v1 -- --list)"
ignored="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  candidate_recovery_v1 -- --list --ignored)"
test "$(printf '%s\n' "$listed" |
  awk '/candidate_recovery_v1.*: test$/ { n += 1 } END { print n + 0 }')" -ge 1
test "$(printf '%s\n' "$ignored" |
  awk '/candidate_recovery_v1.*: test$/ { n += 1 } END { print n + 0 }')" -eq 0
cargo test --manifest-path packages/Cargo.toml -p xtask candidate_recovery_v1
make test-adr-index-coverage
make test-lint
```

`set -e` makes either Cargo listing failure fatal before a count is evaluated; no `grep -c`
pipeline may turn a failed listing into a numeric result. An ignored or skipped test, zero
discovered `candidate_recovery_v1` tests, an unmerged scope, or wording that still permits
only one request for the whole wave leaves T008 open. Downstream W2 work now exists while T008
remains unchecked, so a successful current rerun cannot be presented as historical entry
evidence. T008 may close only from a retained receipt bound to the actual W2 entry base and
first dispatch.

If that receipt does not exist, T008 remains unchecked. Exact frozen F2 must instead carry one
passing `EvidenceRecord` with validation `historical-entry-remediation-t008`; its external
receipt records `historicalEntry: "unproven"`, binds the F2 candidate/commit/tree, names the
accepted prerequisite commits, proves every W2 implementation head is an ancestor, and
records successful nonempty execution of the command set above plus current lineage,
destination, cleanliness, and fast-suite checks. T029 refuses panel request, seal, or merge
when neither the exact historical receipt nor this single remedial record exists, or when
either record is duplicated, malformed, failed, or bound to another commit or tree. This
requalification does not assert that original W2 entry complied.

T589 later hardens accepted v1 with the `adr046w5` strict storage profile. It must see accepted
v1 on its own actual base, but it does not retroactively complete T008, T030, or T037.
T029, T036, and T071 invoke this same validator, not local predicates, before pre-panel
dispatch, panel request, panel-attest, merge, post-merge seal, merge-target registration,
and merge eligibility. Any matrix case that would pass at one of those boundaries leaves the
corresponding wave open.

<!-- RETIRED-READONLY-END -->

## ADR-046 Wave 6 historical predecessor guard

**Owner**: production delivery tooling; T221 is the feature entry consumer

**Status**: binding one-time historical predecessor contract

Constitution 3.1.0 supplies only the generic historical-process disposition. This exact
feature-owned delivery validator/tooling contract instantiates it through merged Wave 5
commit `177235ed37188b3be87525e7f016fb43401574c5`. The retained state is immutable: candidate
`d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`, embedded snapshot
identity `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`,
`snapshot.json` SHA-256 `dcf4d71a572bdf0766de557dde6b8ede7fd680eb9f85572238575d2ab5c82149`,
head `19b77dad63060bcadd41f1ef800978d2c53cc030`, retained `panel-request.json` SHA-256
`15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7`, the exact candidate
root and `evidence/local-host/` inventory in `data-model.md`, evidence-tree SHA-256
`7deb84943d36962493422407ac74342fd598b2fea4970ea1a162942e25cfd33d`, zero attestations, and
no seal.

Before Wave 6 implementation dispatch, T221 must:

1. fetch `origin/v3` and use the exact fetched tip as the clean base;
2. require merged Wave 5 and the unique integration commit on the base's first-parent
   lineage after it whose tree contains the exact accepted generic Constitution 3.1.0 bytes to be
   ancestors of the Wave 6 base and head;
3. create the Wave 6 entry snapshot through the production guard and match every retained
   root entry, filename, and digest;
4. pass the focused `xtask` work-item-state tests; and
5. run the ordinary exact-base selected-roster plan lifecycle to N/N sign-off with zero
   recommendations.

The first discovery packet used fetched base
`bfeaf3fe39e4eea9c9180441b7a892b682dfc7f0`, entry commit
`d6de52ca44240b890dd7cc90e6962bf244945b7c`, panel candidate
`1062f5348470756577abe0e11d315fec5819f81b5977a5450adf70e16401e8f7`, content ID
`fc123bf263d8ed82e54c3554ab549a7f4ab75c9b249ea94a768c2068d1e8fbac`, panel
snapshot `edd532c5e3dc13c74f1ab8daa285fee17a3347938f77af674eeb047ad19f0cf3`,
and selection digest
`2399894e8b1b0383d84511853b5a89c4bee553c5eaa3a6f6353a6b81963463a6`. This
feature-content correction invalidates that binding. It remains discovery evidence, not an
entry pass. T221 requires a replacement snapshot, selection, complete structured command
evidence, external dispatch ledger, unanimous result, and durably written plan-approval
receipt. Those records provide process correlation/completeness and are not authentication.
Passing T221 authorizes T606 only.

The predecessor guard validates the retained W5 candidate, embedded snapshot, head, request,
evidence inventory, zero-attestation state, and absent seal inside the predecessor record.
The W6 snapshot emits distinct new candidate/content/snapshot identities from the current W6
material. It must not equal a retained W5 identity. The public sequence first sets
`D2B_W6_DISPATCH_LEDGER` and `D2B_W6_COMMAND_EVIDENCE_DIR`, then runs
`delivery wave snapshot ... --entry-prepare true` without command evidence. That call
fresh-fetches/discovers the candidate and atomically create-or-compares the 36-entry
`NotLaunched` ledger plus empty evidence directory; it does not write `snapshot.json`.
The integrator then runs the closed eight commands through an external strict-JSON recorder,
including nonempty focused guard enumeration, zero ignored/skipped results,
`make test-drift`, `make test-policy`, `make test-unit` with flake/nix-unit/runtime-ledger
membership, heavy-gate acquisition, and the machine-derived 258/29/7/265 census. Entry
preparation is rerun with one repeated `--command-evidence PATH` for each record. Ordinary
snapshot omits both entry-prepare and command-evidence flags, validates the exact imported
eight-record set and ledger, and only then writes the W6 snapshot. No command record is
required before the first candidate discovery, and no evidence may be fabricated.

Before the ledger's first `Dispatched` transition, a material base/guard/requirement/
dependency/ownership/validation/readiness change invalidates T221. After first dispatch,
status-only checkbox/completion/evidence/dispatch/merge/seal projections derived from the
ledger do not invalidate it and cannot change plan authority. A later material change blocks
affected groups until replacement plan material and approval receipt are accepted.

The current `plan-approval` command consumes the candidate-bound plan selection and canonical
`d2b-panel/approval`, then durably writes the simplified receipt binding snapshot identities/
fingerprints plus selection, approval, dispatch-ledger, and command-evidence digests. It does
not embed duplicate seat records or a second lifecycle approval schema. These records remain
correlation/completeness evidence rather than authentication.

The entry snapshot's plan selection, final F6 plan selection, and F6 work selection are
non-interchangeable. Final F6 uses a distinct snapshot and new plan selection. Its unchanged
candidate receives a separate exactly-once work selection passed to `panel-request`. Groups
reach Completed only through current `validate`/`complete` commands with accepted commit/tree
evidence; T479/T480 own the prospective post-integration/merge projection to Merged without
inventing another artifact schema. `merge-eligibility` retains its result only after
evaluation succeeds.

Candidate material auto-derives the canonical graph. Snapshot generated/dependency/contract
fingerprints plus selection, canonical approval, ledger, and command-runner digests form the
composite binding. Only parsed status projections normalize; free-form text and every
authority-bearing requirement, dependency, owner, destination, validation, handoff, profile,
graph, selection purpose, and guard remain byte-significant.

The same production guard runs at Wave 6 snapshot/entry and is rechecked at panel request,
seal, and merge eligibility. Missing, extra, partial, changed, unfetched, non-first-parent,
non-ancestor, or substituted state refuses with remediation to restore the exact retained
state or rebase onto the fetched accepted integration lineage. The guard does not create or
require a Wave 5 seal.

The former actionable retained-request disposition contract is retired. No Wave 5 recovery,
second request, replacement candidate, retroactive attestation, reconstructed seal, import
record, or close action exists. This guard tracks process integrity and signoff state; it is
not authentication and is not a security boundary.

## Recovery-point attestation validator v1

**Contract id**: `d2b-recovery-point-attestation/v1`

**Owner**: T548; consumers T580, T555, and T556

One hermetic validator owns canonical JSON shape, every FR-043 field and delivery binding,
bounded integer timestamps, checked expiration arithmetic, external locator resolution, and
candidate/commit/tree/preview/host/operator/restore-instruction matching. T580 uses it for
import; T555 and T556 invoke the same implementation at every close boundary. A stage-local
predicate copy is ineligible.

The table-driven negative suite begins with one valid record and varies exactly one field or
binding at a time. It covers every required top-level field and qualification member,
including wrong `operatorSubjectSha256` and `restoreInstructionsSha256`, as well as missing,
duplicate, extra, malformed, wrong-type, per-timestamp negative/fractional/out-of-range,
future-event, checked-add-overflow, stale, expired, and unresolvable cases. The prerequisite
is:

```bash
set -euo pipefail
listed="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  recovery_point_attestation_v1 -- --list)"
ignored="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  recovery_point_attestation_v1 -- --list --ignored)"
test "$(printf '%s\n' "$listed" |
  awk '/recovery_point_attestation_v1.*: test$/ { n += 1 } END { print n + 0 }')" -ge 1
test "$(printf '%s\n' "$ignored" |
  awk '/recovery_point_attestation_v1.*: test$/ { n += 1 } END { print n + 0 }')" -eq 0
cargo test --manifest-path packages/Cargo.toml -p xtask \
  recovery_point_attestation_v1
```

Cargo listing failure, zero discovery, any ignored matching test, any skip, or a failing
per-field case blocks import, pre-panel dispatch, panel request, panel-attest, merge,
post-merge seal, merge-target registration, and merge eligibility.

## Rules that apply to every surface

1. **Versioned, not silently changed.** Adding, removing, or renaming a field or operation
   bumps the relevant version, updates the paired schema and prose, and lands in the same
   change as the emitter (FR-031, constitution Principle IV).
2. **Generated artifacts are the contract.** The committed bytes under
   `docs/reference/schemas/v3/` are authoritative; `make test-drift` regenerates and requires
   a clean diff. A hand-edited schema is a gate failure, not a shortcut.
3. **Documentation ships with behavior.** A reference page affected by a change lands in the
   same wave, never deferred (FR-019).
4. **No compatibility layer.** 3.0 is a clean break. A surface being removed is removed, with
   a removal proof, in its own commit, after its successor is integrated and tested (FR-023).
5. **Retirement is explicit.** A capability may only disappear if it is on the retirement list
   with a justification and named in the release notes (FR-042).
6. **Nothing leaks.** No secret, credential, command output, raw host path, or PII crosses any
   of these surfaces into telemetry, audit, logs, or errors (FR-018).
7. **Production evidence crosses production boundaries.** Resource-plane acceptance enters
   through a registrar-admitted, pidfd-bound ComponentSession and the published ZoneBus route.
   Restart uses a fresh pidfd, and PID reuse, mismatch, `ESRCH`, or ambiguity denies. A direct
   ResourceService or `WatchService` call, fixed subject, fake endpoint, independent readiness
   flag, disabled audit owner, or result from another commit cannot satisfy the
   `adr046w5` gate
   (FR-066-FR-072).
8. **One readiness projection, no partial publication.** Store, matching policy, session/router,
   controller endpoint, watch admission, audit catch-up, and the
   `d2b-core-controller`-owned `Provider/system-core` registration publish together with
   exactly one `Zone.status.handlers[]` record named `system-core-host` and one named
   `system-core-user`, each carrying `phase` and `lastReconciledAt` from its live handler, or
   the affected Zone refuses with remediation. `ProviderLifecycle` is distinct and cannot
   substitute. No other Wave 6 Provider dossier gates this wave (FR-069).
9. **Policy bootstrap is private and one-shot.** `ZoneResourceRuntime` may consume one sealed,
   private-issuer, non-`Clone`, non-`Copy` `PolicyBootstrapRead` for the first exact-revision
   policy-envelope snapshot. Compiler/API/external seals forbid public construction, default,
   conversion, reconstruction, or reuse. It carries no public Resource API subject or general
   read/mutation surface; all later policy access uses authenticated Resource API revision
   rules, and both store crates remain policy-neutral (FR-067, FR-073).
10. **A committed mutation is never unaudited or reported as rolled back.** Its immutable
    authoritative journal row commits in the same transaction. Until separate segment export
    and completion finish, return operation-bound `CommittedPendingAudit` through the layered
    `ResourceStatus`:
    `phase = ResourcePhase::Degraded`,
    `outcome.code = StatusCode("committed-pending-audit")`,
    `update.state = UpdateState::Blocked`, and
    `update.operation_id = Some(original_operation_id)`. Existing condition, outcome, and
    update detail stays bounded and redacted. Direct Version 2 operator CLI/JSON status and
    recovery responses may return only bounded `zoneRef` and `operationId` values supplied or
    received by that operator as recovery coordinates. They never become telemetry labels,
    spans, exported audit identities, or unrelated error context. Additive protobuf
    `PendingAuditStatus` carries
    the composite on every mutation response, including delete. Keep the Zone unpublished,
    require exact subject/Zone/request/target/verb/revision/idempotency replay binding, and
    make same-ID retry observe rather than reapply the mutation. Audit/export identifiers are
    fixed domain-separated digests, and retention/prune failure is typed degraded health
    (FR-070).

Current prospective host-generation, handler-contract, admission, and operator-acceptance
implementation ownership resolves only from authoritative member specs and generated
manifests. Active feature-local T604 is the narrow cross-provider acceptance exception; its
task row owns its files, development validation, and the
`operator-nix-activation-cleanup` validator identity. It authors the daemon-restart host case
and its Makefile recipe after manifest-backed `ADR046-ch-001`, but emits no candidate-bound
record. After converging and freezing F6, T479 invokes the operator validator, runs the
daemon-restart case with the Cloud Hypervisor case, emits the one
`operator-nix-activation-cleanup` record, and records FR-075 only in
`w6-cloud-hypervisor-guest-acceptance`; T480 revalidates both closed predicates.

<!-- RETIRED-READONLY-BEGIN -->

11. **Amended-plan reconciliation is historical.** The former T603/T589 editor and lifecycle
    sequence is read-only history and authorizes no current mutation. Code canon lacks the
    source-generation handoff.
12. **Operator activation is acceptance evidence.** The acceptance task starts from the emitted Nix
    resource declaration and per-Zone bundle, activates on startup and public declaration and
    removal switches without manual restart, observes the spec-pinned Provider/config/effect
    and readiness for `Volume/acceptance-state`, `Network/acceptance-net`, and
    `Device/acceptance-tpm`, then removes only the Device and proves its state-preserving
    cleanup without disturbing the ready, identity-stable, unrecreated acceptance
    Volume/Network or unrelated resources. W4 history remains byte-preserved but its sole
    Network opt-in is nonconforming and non-authorizing.
    This one denied-east-west sample is not double-opt-in evidence. T221 requires the
    accepted Network contract/work-item amendment and double-opt-in
    migration, remove every current-facing sole Network-opt-in path, and retain T336-T355
    plus all four Network/Host production cases as W6 work under T221. T070 and T071 retain
    historical evidence only. T221 fail-closes prospective Wave 6 entry until the migration,
    ownership, and exact historical-predecessor guard are on the fetched integration base.
    The task remains W6 acceptance-only after T336-T355 merge and consumes the landed
    implementation.
    Guest runtime-effect acceptance
    is deferred specifically to Wave 6 `Provider/runtime-cloud-hypervisor` T384/T479/T480;
    Guest emission, status, or refusal cannot
    satisfy this partial US1 production-plane checkpoint. Refusals are
    separate negative cases. The exact
    candidate result is emitted once as
    `operator-nix-activation-cleanup`, imported by T479 on exact F6, and excluded from the
    Wave 5 T589/T600-T602 profile.
13. **C1 is a prospective W6 correction.** Retired T605 did not land: code canon contains
    neither `ZoneHandlerName::SystemCoreHost` nor `SystemCoreUser`. T423 now owns those values, serialized only
    as `system-core-host` and `system-core-user`; underscore spellings remain internal
    telemetry labels. Both governing normative specs and their version metadata move with
    targeted Rust/contract tests, compiler-derived public/private API snapshots, paired
    reference status text, and byte-identical Zone desired-schema proof before acceptance.
    Former T595/T599/T605/T220 ownership is historical and not reconstructed.
14. **Wave 5 evidence is immutable history.** The former T220 graph and T600/T601 evidence
    ownership are retained as unchecked historical design only. The T600 set was
    `production-session-watch`,
    `effect-replay-cleanup`, `audit-drain-replay`, and `system-core-handler-contract`;
    T601 owns exactly
    `resource-plane-rss-owner-fanin`, `wave5-removal-proofs`, and
    `cli-reference-conformance`. Those seven identifiers were the complete historical plan.
    The acceptance task separately produces W6 `operator-nix-activation-cleanup`, which T479 imports and which
    cannot enter the Wave 5 profile. T602's planned validator rejects any unknown, duplicate,
    missing, extra, wrong-lane, or conflated identifier. The exact disposition does not claim
    those unchecked planned rows completed. Wave 5's retained `panel-request.json` consumed
    its binding surface with zero attestations and no seal. T219 records only that immutable
    historical disposition. T221 is the next executable gate.
15. **SC-002 has one external authority.** The accepted Version 2
    `ADR-046-validation-and-delivery` specification and its generated
    `ADR-046-validation-and-delivery-traceability.{json,md}` artifacts solely own
    `VD2-SC002-RECEIPT`, `VD2-SC002-PUBLICATION`, `VD2-SC002-INCIDENT`,
    `VD2-SC002-DISPOSITION`, `VD2-SC002-RECOVERY`, `VD2-SC002-SOURCE-FLOOR`,
    `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY`. T589/T600/T220 are historical
    planned consumers. Prospective acceptance uses only current generated rows. This feature
    contains no normative SC-002 encoding, census, registry count, or recovery state copy.
16. **Recovery is never a status-only dead end.** `VD2-SC002-RECOVERY` requires every emitted
    action to resolve to an exact invocation or an owned versioned runbook section and binds
    the resulting state transition. T599 separately owns
    `docs/how-to/host-generation-recovery-v1.md` and the generated public action mapping.
    The former T220 release check is historical and supplies no current gate. Recovery uses
    only the existing broker unit and preserves the daemon-only three-unit architecture.

<!-- RETIRED-READONLY-END -->

17. **Observed panel values are process metadata, not authentication.** Before prospective
    Track A `make-records`, round-local `observed.json` contains exactly one entry for every
    selected seat and no other seat. Every entry requires `provider`, `model`,
    `reasoning_effort`, `context_tier`, `communication`, `agent_type`,
    `agent_definition_sha256`, `run_id`, and `receipt_locator`. The fixed provider and
    completion-bound dispatch binding supply the policy fields, the completion marker
    supplies the staged definition digest, and the same-user integrator captures the run ID
    and receipt locator from the actual selected Task result envelope. The record generator
    validates those observations against the completed packet and requires unique run and
    receipt values. This correlation evidence is not authentication, an authentication
    proof, a security boundary, or proof that a particular definition executed.
18. **A reviewed base cannot move into merge.** Every prospective `v3` merge requires
    effective protection with a nonempty set of required status checks configured for strict
    up-to-date enforcement. The operator checks the exact base OID before snapshot, after
    required checks, and immediately before merge; GitHub atomically refuses the final race
    once the expected base is stale. A merge queue does not replace this requirement and is
    sufficient only when a required `merge_group` check compares the actual merge-group
    integration tree with the snapshot-bound expected `integration_tree_oid` and refuses a
    mismatch. A head-only match or post-merge tree comparison is insufficient. Any base
    change requires an integration-branch update and makes the old validation,
    selected-roster verification, snapshot, binding, records, attestation, and CI evidence
    ineligible; validation, selected-roster verification, snapshot, binding, and required
    checks restart in Track A order. If a prospective wave's sole request is consumed,
    replacement binding is forbidden unless a later accepted contract explicitly authorizes
    it. The exact ADR-046 contract provides no such authorization for Wave 5. This rule does
    not reorder request, records, attestation, merge, seal, merge-target, or
    merge-eligibility and does not relax the Wave 6 T221 gate.
