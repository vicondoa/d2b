---
description: "Task list for completing the ADR-046 Provider control plane (d2b 3.0)"
---

# Tasks: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Input**: Design documents from `/specs/001-adr046-d2b3-completion/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/),
[spec-coverage.md](./spec-coverage.md)

## How this task list is organized

This is not a greenfield feature. The primary task set preserves the 531 work items that were
remaining at program opening and were already sequenced by a committed dependency graph,
delivered under a wave contract with a hard gate between waves.
Tasks are therefore grouped by **wave** first, then by **parallel group**, because that is the
real dependency and gating structure. User-story mapping is recorded per wave.

Waves are **pipelined**: the next wave starts coding once 5 of 10 predecessor panels return and
integration tests pass, but panel, seal, and merge remain **strictly ordered**. Within a wave, parallel groups are file-disjoint by
construction and MUST be launched in the same coordination cycle - a ready slice left
unlaunched without a recorded blocker is a process failure, not a scheduling preference.

## Format: `[ID] [P?] [Story] WorkItemId - destination label (reuseAction display)`

- **[P]**: Parallelizable with other ready tasks after all of its declared prerequisites
  complete, with no file-overlap edge among the tasks launched together. The marker does not
  remove an incoming dependency or mean immediate wave-entry eligibility. 91 of 545 manifest
  work items qualify. Across the full task list, 99 of 605 total tasks carry `[P]`.
- **[Story]**: US1 live resource plane, US2 Providers, US3 cutover, US4 release.
- Text after `WorkItemId` is a **non-authoritative navigation label**, not the manifest
  `destination` field, a writable path list, or a substitute for retrieval. Labels use
  balanced path syntax but may omit destinations and descriptive detail. The complete
  `destination`, canonical `reuseAction`, and every other field come only from the full
  manifest object retrieved at dispatch.

## The authoritative-detail rule

Each task below is a **pointer to a manifest entry, never a summary of one**. Before starting
or dispatching any task, retrieve its complete 15-field entry verbatim:

```bash
jq --arg id ADR046-routing-001 \
  '.items[] | select(.workItemId==$id)' \
  docs/specs/ADR-046-work-items.json
```

The returned object includes `workItemId`, `specId`, `specPath`, `implementationState`,
`detailedDesign`, `validation`, `destination`, `integration`, `dataMigration`,
`currentSource`, `reuseAction`, `reuseSource`, `dependencyOwner`, `removalProof`, and
`evidence`. Retrieve and carry the whole object; selecting only the fields named in prose is
not equivalent. Its design, validation, destination, integration, migration, reuse,
dependency, and removal-proof fields are the implementation obligations. A task is not
complete until its `validation` obligations are satisfied and its `removalProof` passes where
it retires a path.

Deliberately, this file does not copy that text. Duplicating 531 manifest entries into
Markdown would create a second source of truth that no drift gate checks - the same failure
this program is trying to avoid. Reference, never paraphrase.

## Wave gate tasks

Prospective wave delivery carries two distinct panel surfaces. Before implementation dispatch,
`/d2b-panel-round plan` must return all ten role records against one exact implementation base
and feature snapshot, each with `signoff: true` and no recommendations. The same nonbinding
phase surface handles integrated-candidate finding-fix rounds and may iterate, with
delta/full-context review after each content change, before the final candidate is selected.
Only that final immutable candidate may receive the wave's exactly one binding
`/d2b-panel-round work` delivery request. A binding finding leaves the wave unsealed and
cannot be followed by a second binding request for any candidate in that wave. The consumed
request and its disposition remain external delivery facts; feature-local prose, a
replacement candidate, or a later plan panel cannot free the request slot. A work-panel
receipt is never entry plan-review evidence. Gate tasks are numbered inline with the wave
they enter or close; these requirements add no task IDs.

W2-W4 are not prospective delivery: external delivery state reports them sealed and merged.
Their remaining open gate tasks verify/adjudicate that history only and cannot rerun a phase
or binding panel, create a candidate, attest, seal, or merge. Each requires exact external
delivery-record confirmation or an accepted external correction. Wave 5 is also exceptional:
its retained pre-amendment `panel-request.json` consumed the once-per-wave binding request.
Nonbinding `/d2b-panel-round plan` phase rounds create no delivery request or reservation and
cannot replace that record. T219 remains non-authorizing until an accepted external
delivery-contract/tooling disposition expressly resolves the retained request. T219 has no
binding-action path: it may perform only a non-request close action expressly
authorized by that external disposition.

**Program-wide external constitution prerequisite (FR-036, open)**: every numbered task and
every dependency statement below is subordinate to a separately accepted amendment to
Constitution Principle VI. Before any implementation, resume, convergence-fix, panel-fix,
work-panel, seal, merge, or advance action, require the amendment commit to be on the
integration lineage and an ancestor of the exact execution base, and require its text to
expressly disposition both the missing W0/W1 panel/seal history and the unproven
contemporaneous W2-W5 plan panels. `waiver-w0-w1.md`, a remedial plan receipt, a
`historical-entry-remediation-*` record, a checked task, or a later work panel cannot satisfy
this prerequisite. Until it lands, those artifacts are evidence only and all such actions
refuse. This prerequisite adds no task ID and changes no checkbox.

**W2-W6 host-continuity close gate (FR-075)**: prospective execution is limited to
T220/T604 for W5 and T479 for W6. Those tasks MUST run the existing heavy-gated
`make test-host-integration` target against their exact proposed candidate, require nonempty
enumeration and a successful build of
`vmChecks.x86_64-linux.daemon-restart-vm-survival`, and reject every skip. The result must
prove public `d2b vm` start/status/stop, an explicit `Ready` state before restart, guest
reachability, `d2bd.service` restart, same runner PID/start-time adoption through a newly
acquired pidfd, continued reachability, and an explicit `Stopped` state after stop. It must query the complete loaded `d2b*`/`microvm*` namespace, exclude exactly the canonical
`d2b.slice`, and require the remaining set to be exactly `d2bd.service`,
`d2b-priv-broker.socket`, and `d2b-priv-broker.service`. A nonzero
`systemctl list-units --all` result is fatal before filtering and cannot be masked by a later
pipeline stage. No other slice, target, service, socket, timer, path, or template is excluded;
every unexpected lifecycle unit refuses.
The negative matrix injects one unexpected loaded `d2b-unexpected.slice` and, separately, one
unexpected loaded `d2b-unexpected.service`; both remain after the sole `d2b.slice` exclusion
and must fail the exact-set comparison.
Historical T028,
T035, and T070 only inspect retained evidence, MUST NOT rerun the target, and own no test
edit. T604 is the sole
prospective owner of `tests/host-integration/daemon-restart-vm-survival.nix` and that check's
discovery/build recipe in `Makefile`; its public target must fail on empty discovery, any
`SKIP`, a missing build, or a non-x86 result presented as passing. The positive case proves
`Ready` before daemon restart, reachability, fresh-pidfd adoption of the original runner with
the same PID and start identity, continued reachability, and `Stopped` after public stop.
Its negative cases inject numeric PID reuse, pidfd/start-identity mismatch, and multiple
plausible runners and require quarantine with no adoption, signal, or cleanup against an
unproven process.
T029, T036, and T071 MUST reopen that result only while verifying the external historical
close records; they do not rerun it or use it to authorize another panel, seal, or merge.
T219 revalidates the W5 result only as a precondition to an externally authorized close
action, and T480 revalidates W6's prospective result before panel request, seal, merge
eligibility, and merge. Historical F2/F3/F4 records must each contain exactly one
candidate-bound `local-host`
`EvidenceRecord.validation = "pre-adr046-host-continuity"` result; W5 folds the result into
the existing `operator-nix-activation-cleanup` record; W6 folds it into the existing
`w6-cloud-hypervisor-guest-acceptance` record. Missing, duplicate, empty, skipped,
wrong-candidate, stale, status-only, private-hook, missing Ready/Stopped, non-fresh-pidfd,
incomplete unit enumeration, or nonexecuted historical W2-W4 evidence requires external
correction and leaves adjudication unchecked; it does not schedule a replacement close.
The same defects block any externally authorized W5 action and W6's prospective close.
Passing evidence names the exact enumerated and built attr, records command success, and
contains no `SKIP` result. This adds no task ID and no W5 evidence identifier.

For prospective W6-W8, the entry tasks refuse the first implementation dispatch until the
plan receipt validates. For already-delivered W2-W4 and already-dispatched W5, no
contemporaneous plan-panel receipt is cited by these feature artifacts; historical plan-review
compliance is unproven. T008, T030, T037, and T072 may be checked only by exact retained
evidence from the applicable first-dispatch base. W2-W4 do not run remedial plan panels:
their remaining tasks require exact external delivery-record confirmation or an accepted
external correction and remain historical verification/adjudication only.

Wave 5 uses the T603 A/P0 and B/P nonbinding phase-plan chain after its separate
`historical-entry-remediation-t072` disposition. Those current receipts leave T072 unchecked,
do not rewrite the missed boundary, create no delivery `panel-request.json` or reservation,
and cannot dispose the retained binding request. T219 separately requires accepted external
disposition of that consumed request.

Prospective ADR046 `SINGLE BINDING WORK GATE` tasks beginning with W6 permit exactly one
binding delivery-panel request per wave. Before that request, provisional candidates may be
replaced, validation may be refreshed, and scoped findings may iterate through
delta/full-context `/d2b-panel-round plan` phase reviews. The final unanimous phase result
selects the immutable candidate. The binding request pins its exact commit, tree, candidate,
request digest, and round address. A content change, evidence rewrite, history-only rebase,
same-candidate retry, or alternate-candidate request after that point is refused at
panel-attest, seal, and merge-eligibility.

A nonunanimous binding result is the failed wave close, not another fix-round input. Preserve
its request, findings, and records, issue no second binding request for any candidate, and
stop for an accepted external disposition. Feature-local successor guidance is
non-authorizing and must not be used. Findings are never discarded or waived.

T589 owns the wave-scoped fail-closed implementation and table-driven coverage in
`packages/xtask/src/delivery/{panel.rs,seal.rs,eligibility.rs,history_proof.rs,storage.rs}` for
`adr046w5` and later users. W2-W4 cannot depend on code delivered in W5. T008 therefore owns
the historical W2 entry attestation for versioned feature-local contract
`adr046-candidate-recovery-prerequisite/v1` in `contracts/README.md`: the coordinated external
ADR/index, validation/delivery spec and generated manifests, delivery tooling, `AGENTS.md`,
and contributor guidance had to be ancestors of the actual W2 entry base. The unchecked T008
state cannot now authorize work that was already dispatched. W2-W4 may use the accepted
external v1 behavior only to read and adjudicate retained delivery records or an accepted
external correction; they do not invoke it to repeat a close and never claim T589's later
strict storage profile. The generic history-proof reuse path remains available only before a
candidate's binding request or for delivery programs that do not select the strict profile.

### Historical entry-attestation adjudication

Committed implementation exists downstream of T008, T030, and T037 even though those entry
tasks are unchecked. Their pre-dispatch predicates are facts about the actual first-dispatch
bases and cannot be made true later by rerunning commands on a newer tree. T008, T030, and
T037 are therefore historical entry attestations, not prospective dispatch authorizations.
Preserve each current checkbox. It may be checked only when contemporaneous retained evidence
proves every stated predicate against the exact historical entry base; current code presence,
current manifest state, or a successful command rerun is not that proof.

External delivery state also reports W2, W3, and W4 sealed and merged. Therefore T028/T029,
T035/T036, and T070/T071 are historical close verification/adjudication tasks, not a recovery
queue. They own no new candidate, phase review, binding request, attestation, seal,
merge-target registration, merge eligibility, merge, successor rebase, or cleanup action.
They must not claim that this batch produced a new seal or merge.

Historical adjudication fails closed unless exactly one of these external dispositions exists
for each wave:

1. `delivery-record-confirmed`: exact external delivery records name `historicalTask`,
   `entryBaseCommit`, `entryBaseTree`, `firstDispatchCommit`, `recordedAtUnix`, every required
   prerequisite commit locator, and every original check with its contemporaneous result.
   The same record set must identify the actual immutable candidate and tree, the wave's sole
   binding `panel-request.json`, ten unanimous attestations, seal, merge target, eligibility
   result, merged pull request, and resulting `v3` commit. Hashes and ancestry must agree; the
   first dispatch commit must descend from the named base. Current command output cannot fill
   a missing historical field.
2. `delivery-record-corrected`: an accepted external delivery-contract/tooling correction
   identifies the exact inaccurate or missing historical record, preserves the original
   bytes, and states the authoritative corrected status. The feature task records only that
   adjudication. It does not reinterpret a current rerun as historical proof or authorize
   this batch to run another binding panel, seal, or merge.

For either disposition, the imported `EvidenceRecord.output` is the digest and byte count of
the canonical external receipt and `locator` resolves that receipt without storing command
output in the repository. Missing, duplicate, malformed, failed, wrong-task, wrong-commit,
wrong-tree, wrong-candidate, wrong-panel, wrong-seal, wrong-merge, or
current-rerun-presented-as-historical evidence refuses T029, T036, or T071. An accepted
correction does not rewrite history or claim a new close; it resolves what the external
delivery authority says happened.

---

## Phase 0: Pre-W2 spec hygiene (BLOCKING)

**Purpose**: Close the requirements defects that would otherwise be inherited by every wave.
Gate 1 items were closed during planning; these are the Gate 2 items that block declaring W2
entry criteria met.

- [X] T001 Resolve CHK013 - state Gate 0's standing re-evaluation obligation as a requirement, not only an assumption
- [X] T002 Resolve CHK027 - record the ordinary entry-evidence versus exit-evidence distinction. The later 2026-08-06 correction supersedes its application to the W0/W1 and W2-W5 Principle VI gaps: FR-036's external constitution prerequisite blocks both entry and exit until accepted
- [X] T003 Resolve CHK028 - bound the FR-034 historical record so it waives no work-item completion obligation, including the nine `ADR046-delivery-*` items that remain Planned
- [X] T004 Resolve CHK039 - state the contended-file prep discipline; W2 has a single `nixos-modules/assertions.nix` writer and this has immediate effect
- [X] T005 Record every Gate 3 checklist item as a deliberate deferral naming its owning wave, so a scheduled obligation is never mistaken for a coverage gap
- [X] T006 Answer CHK047 - confirm whether cloud accounts and access exist for the Azure-backed Provider validation required at W6 and by the release gate
- [X] T007 Prototype the RSS corrections (range-seek replay, streaming decode, shared immutable ChangeBatch fan-out) in `proofs/redb-resource-store-spike/` so W5 confirms rather than discovers (mitigates RK-1)
- [X] T574 **Author and record the W0/W1 delivered-without-seal history** (FR-034). It names the missing artifacts (the ten panel receipts and the seal for each wave) and the evidence actually available (all 14 assigned work items recorded as Merged through reviewed pull requests). Completion means the historical deviation is documented only; it does not waive Principle VI, authorize W2 entry, or satisfy FR-036's separate external constitution prerequisite
- [X] T575 **Raise the recorded W2 destination drift to the integrator as a specification amendment** (FR-046). `ADR-046-validation-and-delivery` §3.2 lists `packages/d2b-process/` and `packages/d2b-provider-supervisor/` under W2, but the graph assigns their owning item `ADR046-process-001` to W4. Follow the graph; do not correct the prose inside a wave
- [X] T576 **Inventory which migration-map DELETE and REPLACE rows still lack a removal proof** and assign each missing proof to the wave that removes its path (FR-023). The current [`removal-proof-inventory.md`](./removal-proof-inventory.md) 48-row census records 5 proofed DELETE rows and 33 outstanding DELETE/REPLACE rows overall

### Prior panel model migration (COMPLETED)

T581-T584 record the earlier migration to `gemini-3.1-pro-preview`. That
binding is now the exact legacy compatibility pair; current gate instructions
below use `gpt-5.6-sol` at `xhigh`.

- [X] T581 Amend `ADR-046-validation-and-delivery` §12.3 to bind the panel to `gemini-3.1-pro-preview`, updating the pinned provider/model/reasoning-effort triple and the 14-field record example. This is a member-spec amendment: it re-opens that spec's validation and panel evidence and re-triggers Gate 0 (FR-046)
- [X] T582 Update the pinned constants in `packages/xtask/src/delivery/model.rs` (`PANEL_PROVIDER_POLICY`, `PANEL_MODEL_POLICY`, `PANEL_REASONING_EFFORT_POLICY`) and the unit test at the bottom of that file that asserts their exact values
- [X] T583 Update the `ADR046-delivery-005` work item text, which explicitly says "adapt to bind the fixed `gpt-5.6-sol` model at reasoning effort `xhigh`", then regenerate the spec-set and work-item manifests and confirm `make test-drift` is clean
- [X] T584 Add the ten read-only Copilot panel agents and bind them through `.github/skills/d2b-panel-round/SKILL.md`, then correct the AGENTS.md panel-tooling wording so panel lanes do not silently fall back to a model whose records `panel-attest` will reject. The panel table explicitly binds `github-copilot` / `gemini-3.1-pro-preview` / `high` / `default`; the retired integration is not a supported path

### Pipelined-wave migration (LANDED)

Constitution 2.0.0 permits pipelined implementation start, and the accepted
`ADR-046-validation-and-delivery` amendment plus delivery tooling now enforce the distinction:
implementation may start under the four conditions below, while panel request, seal, and merge
remain ordered behind predecessor merge and the successor's mandatory rebase.

- [X] T585 Amend `ADR-046-validation-and-delivery` §4 to permit pipelined implementation start under the four conditions (5 of 10 reviews returned, integration green, no successor panel/seal/merge before predecessor seal and merge, mandatory post-merge rebase before the successor panel). Preserve the strict panel/seal/merge ordering verbatim. Member-spec amendment: re-opens that spec's evidence and re-triggers Gate 0 (FR-046)
- [X] T586 Relax the `wave snapshot` entry check so an unsealed predecessor blocks the successor's **exit boundary** rather than its implementation start; the predecessor-merged assertion moves to the exit boundary: `panel-request`, `seal`, and `merge-eligibility`. Add tests covering: start permitted at 5 of 10, panel request refused while the predecessor is unsealed, and seal refused when the successor has not rebased since the predecessor merge
- [X] T587 Record the accepted rework cost (FR-050) in the delivery contract so a future integrator cannot cite pipeline rework as grounds to shorten a panel
- [X] T588 Configure or document review scoping for the `v3` lineage. `detect-changed-files.sh` resolves the default branch to `main` via `origin/HEAD`, but ADR-046 integrates on `v3`, which never merges to `main`. Every wave review MUST pass an explicit diff scope (wave integration branch against its real base) or it will treat the whole v3 divergence as the wave changes

**Checkpoint**: process hygiene is complete. W2 implementation already exists, so entry cannot
now be declared prospectively met. T008 remains the historical attestation. Because delivery
state reports W2 already sealed and merged, missing contemporaneous evidence routes T028/T029
to accepted external delivery-record correction, not to a remedial panel or replacement
close.

---
## Wave W2: Primitive resource composition and Zone routing

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 19 | **Parallel groups**: 2

- [ ] T008 [US1] W2 HISTORICAL ENTRY ATTESTATION - determine whether the actual first W2 dispatch base satisfied feature-local prerequisite `adr046-candidate-recovery-prerequisite/v1`. Require retained contemporaneous evidence that the separately accepted external ADR and ADR-index commit, validation/delivery-spec plus generated-manifest commit, delivery-tooling commit, and `AGENTS.md`/contributor-guidance commit were ancestors of that exact base; that the contract's nonempty `candidate_recovery_v1` discovery and test commands plus `make test-adr-index-coverage` and `make test-lint` passed there; that destinations were uncontended, the stack targeted that exact parent, the heavy-gate slot was available, and the fast hermetic suite was green before dispatch; and that a unanimous ten-role plan panel reviewed that exact base and feature snapshot before the first implementation dispatch. No such plan-panel receipt is currently cited, so this predicate is unproven. A current rerun or retained work-panel record cannot satisfy this historical task. If any predicate is unproven, preserve T008 unchecked and require T028/T029's accepted external delivery-record correction to retain the gap; do not run a remedial plan panel, create a replacement candidate, or repeat W2 close actions. Under FR-057, T029 only verifies whether the historical records prove predecessor status and rebase ancestry; it authorizes no current panel, seal, or merge.

### Group `wi:ADR-046-primitive-resource-composition` (3 items)

- [X] T009 [P] [US1] `ADR046-primitives-001` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [X] T010 [P] [US1] `ADR046-primitives-002` - `packages/d2b-provider-system-systemd/` (adapt)
- [X] T011 [P] [US1] `ADR046-primitives-003` - `packages/d2b-provider-volume-*/` (adapt)

### Group `wi:ADR-046-zone-routing` (16 items)

- [X] T012 [P] [US1] `ADR046-routing-001` - `packages/d2b-contracts/src/v3/zone_routing.rs` (adapt)
- [X] T013 [US1] `ADR046-routing-002` - `packages/d2b-zone-routing/src/engine.rs` (adapt)
- [X] T014 [US1] `ADR046-routing-003` - `packages/d2b-zone-routing/src/resolver.rs` (ZoneEntrypointResolver) (adapt)
- [X] T015 [US1] `ADR046-routing-004` - `packages/d2b-core-controller/src/zone_links.rs` (adapt)
- [X] T016 [US1] `ADR046-routing-005` - `packages/d2b-bus/src/zone_route.rs` (cross-Zone bus routing) (adapt)
- [X] T017 [US1] `ADR046-routing-006` - `packages/d2b-zone-routing/tests/route_engine_vectors.rs` (adapt)
- [X] T018 [P] [US1] `ADR046-routing-007` - `packages/d2b-bus/src/session/` (adapt)
- [X] T019 [US1] `ADR046-routing-008` - `packages/d2b-bus/src/transport/unix.rs` (adapt)
- [X] T020 [US1] `ADR046-routing-009` - `packages/d2b-contracts/src/v3/zone_session.rs` (adapt)
- [X] T021 [US1] `ADR046-routing-010` - `packages/d2b-resource-client/` (adapt)
- [X] T022 [US1] `ADR046-routing-011` - `nixos-modules/options-zones.nix` (new structural base) (adapt)
- [X] T023 [US1] `ADR046-routing-012` - `nixos-modules/zone-resources-json.nix` (new) (adapt)
- [X] T024 [US1] `ADR046-routing-013` - `packages/d2b-core-controller/src/configuration.rs` (defined by ADR-046-core-controllers) (adapt)
- [X] T025 [US1] `ADR046-routing-014` - `packages/d2b-provider/src/` (adapted in place) (adapt)
- [X] T026 [US1] `ADR046-routing-015` - `packages/d2b-provider-toolkit/src/` (adapted in place) (adapt)
- [X] T027 [US1] `ADR046-routing-016` - `packages/d2b-zone-routing/src/service.rs` (adapt)

- [ ] T028 [US1] W2 HISTORICAL CONVERGENCE VERIFICATION - depends on every W2 work-item row, beginning with T009 and ending with T027. Delivery state reports W2 sealed and merged, so this task verifies the retained external record rather than creating or freezing F2. Require either exact external delivery-record confirmation or an accepted external correction. Confirmation must identify the actual clean F2 commit/tree, prove every W2 implementation head is its ancestor, and reopen the contemporaneous plan-review disposition, integration/CI results, reference/docs/decision/removal-proof/register checks, fragment fold, and candidate-bound `pre-adr046-host-continuity` evidence. That evidence must show nonempty enumeration and successful no-`SKIP` build of `vmChecks.x86_64-linux.daemon-restart-vm-survival`, Ready/reachability/restart/fresh-pidfd adoption/continued-reachability/Stopped, exactly the three ADR-0015 units, and PID-reuse/mismatch/ambiguity quarantine. Missing historical evidence requires the approved-correction path; T028 owns no source, test, feature, candidate, panel, or delivery-state mutation and must not rerun commands, merge slices, freeze a replacement, or open/update a PR.
- [ ] T029 [US1] W2 HISTORICAL DELIVERY CLOSE ADJUDICATION - depends on T028. Verify, from exact external delivery records, that the retained F2 received W2's sole binding `/d2b-panel-round work` request, ten unanimous attestations, a seal, merge-target registration, merge eligibility, and a byte-identical merge whose resulting `v3` commit descends from the recorded target. Reopen the T008 historical-entry disposition, plan-review receipt, candidate-recovery result, host-continuity evidence, candidate/tree hashes, panel address, seal, PR, and merge ancestry; absence, duplication, or mismatch fails closed. Alternatively require an accepted external correction that identifies and preserves the inaccurate historical record and states the authoritative delivery status. T029 is adjudication only: do not dispatch reviewers, run a phase or binding panel, create a candidate or evidence replacement, attest, seal, register a target, merge, rebase a successor, clean delivery worktrees, or claim that this batch produced a new W2 close.
  `candidate_recovery_v1` may be invoked only to decode and verify retained historical records
  or the accepted correction against its asserted field/binding matrix. It is not authority
  to construct a remedial candidate or repeat any close stage. Empty discovery, an
  ignored/skipped case, a missing matrix member, or a different interpretation of the
  historical record leaves T029 open.

**Checkpoint**: W2's reported seal and merge are externally confirmed or authoritatively corrected. This batch performed and claims no new W2 panel, seal, merge, rebase, or cleanup.

---

## Wave W3: Provider model and packaging (strictly serial - gates every dossier)

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 4 | **Parallel groups**: 1

- [ ] T030 [US1] W3 HISTORICAL ENTRY ATTESTATION - determine whether the actual first W3 dispatch base had Gate 0 passed, destinations uncontended, the stack proposed against the exact named parent commit, a heavy-gate slot available, the fast hermetic suite green, and a unanimous ten-role plan panel bound to that exact base and feature snapshot before implementation dispatch. If W2 was not yet merged at first dispatch, require retained contemporaneous evidence of at least 5 of 10 reviews returned and green integration on its converged tree. No historical plan-panel receipt is currently cited. A current rerun cannot prove those historical predicates. If any predicate is unproven, preserve T030 unchecked and require the accepted external delivery-record correction used by T035/T036 to record the gap; do not run a remedial plan panel or dispatch more W3 implementation.

### Group `wi:ADR-046-provider-model-and-packaging` (4 items)

- [X] T031 [P] [US1] `ADR046-provider-001` - `packages/d2b-contracts/src/v3/provider.rs` (adapt)
- [X] T032 [P] [US1] `ADR046-provider-002` - one `packages/d2b-provider-<base>-<implementation>/` per Provider with mandatory src/ (adapt)
- [X] T033 [P] [US1] `ADR046-provider-003` - `packages/d2b-provider-system-core/` (adapt)
- [X] T034 [US1] `ADR046-provider-004` - `packages/d2b-contracts/src/v3/semantic_services/{mod,audio,security_key,telemetry,usb}.rs` (create)

- [ ] T035 [US1] W3 HISTORICAL CONVERGENCE VERIFICATION - depends on T031, T032, T033, and T034. Delivery state reports W3 sealed and merged, so verify rather than recreate the actual F3. Require exact external delivery records or an accepted external correction identifying F3's clean commit/tree, W2 merge ancestry and W3 rebase, every implementation head's ancestry, the contemporaneous plan-review disposition, integration/CI results, fragment fold, and FR-019/FR-047/FR-023/FR-051/FR-052/FR-053 checks. If T030 is unproven, the correction must retain that fact rather than substitute a current rerun. T035 owns no implementation, fix, candidate, phase-panel, or PR action and must not freeze a replacement F3.
- [ ] T036 [US1] W3 HISTORICAL DELIVERY CLOSE ADJUDICATION - depends on T035. Verify exact external records binding actual F3 to W3's sole binding `/d2b-panel-round work` request, ten unanimous attestations, seal, merge target, merge eligibility, merged PR, and resulting `v3` commit, including the T030 disposition and predecessor-merge/rebase ancestry. Alternatively require an accepted external correction that preserves the inaccurate record and states authoritative status. T036 must not dispatch reviewers, run a phase or binding panel, create replacement evidence, attest, seal, register a target, merge, rebase W4, clean delivery state, or claim a new W3 close.
  The canonical `candidate_recovery_v1` validator is read-only adjudication evidence here. It
  may decode the retained records or accepted correction under the same asserted matrix as
  T029, but it cannot authorize any repeated close boundary. A local predicate, missing
  matrix member, or inconsistent historical interpretation leaves T036 open.

**Checkpoint**: W3's reported seal and merge are externally confirmed or authoritatively corrected. This batch performed and claims no new W3 panel, seal, merge, rebase, or cleanup.

---

## Wave W4: Components/processes/sandbox, core controllers, provider state, network and credential resources

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 31 | **Parallel groups**: 6

- [ ] T037 [US1] W4 HISTORICAL ENTRY ATTESTATION - determine whether the actual first W4 dispatch base had Gate 0 passed, destinations uncontended, the stack proposed against the exact named parent, a heavy-gate slot available, the fast hermetic suite green, and a unanimous ten-role plan panel bound to that exact base and feature snapshot before implementation dispatch. If W3 was not yet merged at first dispatch, require retained contemporaneous evidence of at least 5 of 10 reviews returned and green integration on its converged tree. No historical plan-panel receipt is currently cited. A current rerun cannot prove those historical predicates. If any is unproven, preserve T037 unchecked and require the accepted external delivery-record correction used by T070/T071 to record the gap; do not run a remedial plan panel or dispatch more W4 implementation.

### Group `wi:ADR-046-components-processes-and-sandbox` (1 item)

- [x] T038 [P] [US1] `ADR046-process-001` - `packages/d2b-process/src/` (adapt)

### Group `wi:ADR-046-core-controllers` (1 items)

- [x] T040 [P] [US1] `ADR046-core-001` - `packages/d2b-core-controller/src/{main,configuration,api_catalog,authz,providers,controllers,ownership,watches,cleanup,zone_links,budgets,store}.rs` (adapt)

### Group `wi:ADR-046-provider-state` (12 items)

- [x] T041 [US1] `ADR046-pstate-001` - `packages/d2b-contracts/src/v3/volume_state.rs` (adapt)
- [x] T042 [US1] `ADR046-pstate-002` - `packages/d2b-contracts/src/v3/provider.rs` (component descriptor `stateNamespaces` field) (adapt)
- [x] T043 [US1] `ADR046-pstate-003` - `packages/d2b-provider-volume-local/` (adapt)
- [X] T044 [US1] `ADR046-pstate-004` - `packages/d2b-provider-volume-local/src/migration.rs` (adapt)
- [X] T045 [US1] `ADR046-pstate-005` - `packages/d2b-provider-volume-local/src/sealing.rs` (adapt)
- [X] T046 [US1] `ADR046-pstate-006` - `packages/d2b-provider-volume-local/src/snapshot.rs` (adapt)
- [X] T047 [US1] `ADR046-pstate-007` - `packages/d2b-provider-volume-local/src/relocation.rs` (adapt)
- [x] T048 [US1] `ADR046-pstate-008` - `packages/d2b-provider-volume-local/src/audit.rs` (adapt)
- [x] T049 [US1] `ADR046-pstate-009` - `packages/d2b-provider-volume-local/tests/state.rs` (ported hermetic atomic/lock/quarantine/lease tests) (adapt)
- [x] T050 [US1] `ADR046-pstate-010` - `nixos-modules/zone-resources.nix` (per-Zone bundle emitter NixOS module) (adapt)
- [x] T051 [US1] `ADR046-pstate-011` - `packages/xtask/src/provider_crate_policy.rs` (adapt)
- [X] T052 [US1] `ADR046-pstate-012` - `packages/d2b-core-controller/src/optional_state_admission.rs` (adapt)

### Group `wi:ADR-046-resources-credential` (8 items)

- [x] T053 [US1] `ADR046-credential-001` - `packages/d2b-contracts/src/v3/credential.rs` (adapt)
- [x] T054 [US1] `ADR046-credential-002` - `packages/d2b-contracts/proto/v3/credential.proto` (adapt)
- [x] T055 [US1] `ADR046-credential-003` - `packages/d2b-provider-credential-secret-service/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [x] T056 [US1] `ADR046-credential-004` - `packages/d2b-provider-credential-entra/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [x] T057 [US1] `ADR046-credential-005` - `packages/d2b-provider-credential-managed-identity/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [X] T058 [US1] `ADR046-credential-006` - `packages/d2b-provider-credential-<impl>/src/controller.rs` (adapt)
- [x] T059 [US1] `ADR046-credential-007` - `nixos-modules/options-resources.nix` (adapt)
- [X] T060 [US1] `ADR046-credential-008` - `packages/d2b-provider-credential-<impl>/src/audit.rs` (adapt)

### Group `wi:ADR-046-resources-network` (8 items)

- [x] T061 [P] [US1] `ADR046-network-001` - `packages/d2b-contracts/src/v3/network.rs`: NetworkSpec (adapt)
- [x] T062 [US1] `ADR046-network-002` - `packages/d2b-provider-network-local/src/ifname.rs` (adapt)
- [x] T063 [US1] `ADR046-network-003` - `packages/d2b-provider-network-local/` - artifact catalog integration for net-VM nixos-system artifact resolution (adapt)
- [X] T064 [US1] `ADR046-network-004` - `nixos-modules/resources-network.nix`: Nix resource object emitter for Network ResourceType (adapt)
- [X] T065 [US1] `ADR046-network-005` - `packages/d2b-provider-network-local/src/controller.rs`: async NetworkReconciler (adapt)
- [X] T066 [US1] `ADR046-network-006` - `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API) (adapt)
- [X] T067 [US1] `ADR046-network-007` - `Provider/device-usbip` owns one relay Process/Endpoint authority per Network and calls the typed UsbipEffectPort for the shared closed `ApplyNftablesProjection` request with closed action enum `Apply/Remove` (adapt)
- [X] T068 [US1] `ADR046-network-009` - `packages/d2b-contracts/src/v3/network.rs` external-attachment sharing schema/status (adapt)

The checked T064-T068 rows record accepted implementation history, while delivery state
reports W4 sealed and merged despite the unresolved Network set recorded in
`implementation-debt.md` sections 15.1, 15.3, 16.2, 16.5, 18.2, and 18.3. T070 and T071
adjudicate that contradiction; they do not reopen implementation or close W4 again. Exact
external delivery-record confirmation must show how the historical F4 satisfied the encoding
and ownership conflicts, production typed-broker Network effect, single-consumption
external-NIC authority lease with all named denials, and executable repository-routed mDNS,
bridge, east-west, nftables, persistent-TAP, macvtap, disruptive-update, deletion, status,
and raw-identity-exclusion coverage. If it cannot, an accepted external correction must
preserve the original record and state the authoritative status. No W4 slice may duplicate or
anticipate a later W6 owner.

The intended retained Network hardening is double opt-in: east-west access requires both the
Network resource and Host/site acknowledgement. Its executable matrix is closed over all four
combinations: Network false/Host false denies, Network false/Host true denies, Network
true/Host false denies, and Network true/Host true allows. Each case must assert both host
bridge-port isolation and net-VM forwarding behavior for the derived bridge and owned net-VM
of the same Network identity.

That target currently conflicts with the untouched external
`ADR-046-resources-network`, which normatively makes `Network.spec.isolation.allowEastWest`
the sole opt-in and requires no Zone/Host gate. This feature batch therefore does not claim
that W4 implemented double opt-in. T070 and T071 remain blocked on an accepted external
correction that preserves the existing record and either proves a versioned normative
amendment plus migration was an ancestor of actual F4 before the four-case implementation,
or states that actual W4 retained sole Network opt-in and that the double-opt-in work remains
unimplemented with a prospective owner outside this historical close. A feature-local matrix,
single-opt-in assertion, test covering only matching pairs, or evidence from the old env
surface cannot resolve the conflict or confirm the reported close. W4 adjudication, T070,
T071, and T220 all refuse until that external correction exists and binds evidence for all
four Network/Host combinations; no feature-local status correction can unblock them.

### Group `wi:core-config-hub:w4` (1 items)

- [x] T069 [US1] `ADR046-network-008` - `packages/d2b-core-controller/src/configuration.rs`: bundle application (create)

- [ ] T070 [US1] W4 HISTORICAL CONVERGENCE VERIFICATION - depends on every current W4 work-item row, beginning with T038 and ending with T069; T039 remains the manifest-authoritative W6 process-provider integration item and is not a W4 prerequisite. Verify exact external records or an accepted external correction for the actual F4 commit/tree, W3 merge ancestry and W4 rebase, implementation ancestry, contemporaneous plan-review disposition, integration/CI and fragment fold, FR-019/FR-047/FR-023/FR-051/FR-052/FR-053 checks, and the candidate-bound `w4-network-authoritative-closure`. Because the untouched external Network spec says sole Network opt-in, T070 cannot claim double opt-in from feature prose. Confirmation requires an external versioned normative amendment/migration that predates F4 plus retained production adapter, authority-lease negatives, all four Network/Host combinations through the ResourceType emitter/controller/broker/net-VM path, and successful nonempty non-skipped `make test-flake`, `make test-integration`, and `make test-host-integration` results. Otherwise an accepted external correction must preserve F4's record, state sole Network opt-in as the authoritative W4 behavior, and leave double opt-in prospectively unimplemented. Missing, later-wave-only, declaration-only, fake-adapter, empty, skipped, advisory, or feature-local-only evidence leaves T070 open. T070 owns no implementation, fix, evidence replacement, candidate freeze, phase-panel, rebase, or PR action.
- [ ] T071 [US1] W4 HISTORICAL DELIVERY CLOSE ADJUDICATION - depends on T070. Verify exact external records binding actual F4 to W4's sole binding `/d2b-panel-round work` request, ten unanimous attestations, seal, merge target, merge eligibility, merged PR, and resulting `v3` commit. Reopen the T037 disposition, predecessor merge/rebase ancestry, `w4-network-authoritative-closure`, and canonical `candidate_recovery_v1` result. Require either the externally versioned pre-F4 double-opt-in contract with its exact four-case matrix or an accepted external correction preserving the sole-opt-in historical result and leaving double opt-in open; do not infer double-opt-in implementation from this feature. T071 must not dispatch reviewers, run a phase or binding panel, create replacement evidence, attest, seal, register a target, merge, rebase Wave 5, clean delivery state, or claim a new W4 close. Missing matrix/version discovery, a local receipt predicate, or inconsistent historical interpretation leaves T071 open.

**Checkpoint**: W4's reported seal and merge are externally confirmed or authoritatively corrected. This batch performed and claims no new W4 panel, seal, merge, rebase, or cleanup.

---

## Wave `adr046w5` (manifest label W5): Production store engine and watch, resource catalog, telemetry, CLI, Nix configuration

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 146 | **Parallel groups**: 12

**US1 scope boundary**: this wave is a partial production-plane checkpoint, not US1
completion. The "Wave 5 acceptance set" means exactly `Volume/acceptance-state`,
`Network/acceptance-net`, and `Device/acceptance-tpm` in Zone `acceptance`, with the exact
Provider installs, configs, effects, readiness, and Device cleanup frozen in `spec.md`.
Support resources cannot substitute. This does not assign implementation ownership: Network
implementation remains owned and close-blocked by Wave 4 T061-T071. Wave 5 must not duplicate
or claim that implementation, and Guest runtime-effect acceptance remains fail-closed until
Wave 6 `Provider/runtime-cloud-hypervisor` completes T384 and T479/T480 accept its exact-F6
evidence.

- [ ] T072 [US1] `adr046w5` HISTORICAL ENTRY ATTESTATION - determine whether the actual first Wave 5 dispatch base had Gate 0 passed, destinations uncontended, the stack proposed against the exact named parent commit, the heavy-gate semaphore available, the fast hermetic suite green, and an exact contemporaneous unanimous ten-role `/d2b-panel-round plan` receipt with zero recommendations bound to that base and the exact feature snapshot before first implementation dispatch. If W4 was not yet merged at first dispatch, require retained contemporaneous evidence of at least 5 of its 10 reviews returned and green integration on its converged tree. Existing code is not evidence that these predicates passed. No exact Wave 5 historical plan-panel receipt is cited at committed HEAD `e6bece5d9debebef467e0c553a4d911701f6223e`, so the predicate is unproven; do not check T072 unless that retained receipt and every other historical predicate are produced. A current rerun or the T603 A/P0 or B/P nonbinding phase-plan panel cannot check T072, and neither can the retained Wave 5 binding delivery request. If any predicate is unproven, preserve T072 unchecked and require FR-036's external constitution amendment to expressly disposition the Wave 5 gap before any implementation or close action. Then, before T603 analysis, plan review, or source change, import exactly one passing `historical-entry-remediation-t072` record bound to clean A/P0. It reruns the current Gate 0, destination, lineage, cleanliness, semaphore, and fast-suite checks and proves every existing Wave 5 implementation head considered by reconciliation is an ancestor of A, but it does not claim historical plan compliance or any T073-T218 obligation complete. T219 remains non-authorizing until W4's reported close is externally confirmed or corrected
and the external owner lands and validates one `Wave5RetainedRequestDispositionV1` for exact
F; no feature-local receipt licenses another request, seal, or merge.

**Approved production-completion amendment**: the 12 manifest groups remain authoritative for
their 146 work items. T589-T602, T604, and T605 add the missing Wave 5 composition, coordinated
contract correction, and evidence, and T603 adds the amended-plan resume reconciliation; they
do not renumber, replace, or complete a manifest item. Dependency order is:

**Current state for this task graph:**
`67f0ba8e32c4f91ebfcb4038aff77821d42b64b1` is a historical amendment input, not
the current pre-T603 A/P0 identity. `2c7195d07e665705edfc63d17c2cd64531d56850`
is the clean committed input to this repair batch and the base of the analysis receipt that
required this feature edit; the resulting snapshot change makes that input ineligible to
authorize T603 afterward. Once this batch is committed, freeze A as the exact clean resulting
commit and P0 as the digest of the exact 28-file feature snapshot defined in `plan.md`.
Require both the pre-T603 analysis and unanimous plan-panel receipts to name that same A/P0;
otherwise the gate fails and T603 remains blocked. This A/P0 is pre-validator authorization
only and is not V/B, C/Q, F, or a delivery candidate. None of the 147 authorized checkbox
changes has occurred. C/Q and the finalized `progress-editor-receipt.json` remain future
artifacts, and neither the reconciliation receipt nor the progress-editor receipt is required
by the A/P0 gate. The C/Q edges below become active only after T603's future V/B
implementation, B/P gates, reconciliation authorization, authorized editor transition,
dedicated checkbox commit C, and receipt finalization.

```text
current pre-T603 A/P0 analysis + unanimous plan review -> T603 validator-and-fragment V/B
T073-T218 obligations + post-T603 B/P gates -> T603 receipt/editor transition -> future C/Q
future C/Q -> fresh exact C/Q analysis + unanimous plan review
accepted external source-generation compatibility disposition -> installed source 3/1 floor
{fresh exact C/Q analysis + unanimous plan review, installed source 3/1 floor} -> T589 -> {T590,T591,T594}
T591 -> T592 -> T593 route -> T605
{T590,T592,T594,T605} -> T595
T595 -> {T596,T597,T598,T599,T604} -> T220 -> freeze candidate F
freeze candidate F -> {T600,T601} -> T602 -> T219
```

T603 is the sole in-feature direct prerequisite of T589. FR-070's accepted and installed
source-generation compatibility floor is a separate external dispatch prerequisite. T589
remains blocked until that floor exists, the external reconciliation receipt and editor
progress receipt pass, every receipt row is satisfied, and
T073-T218 plus T603 are checked by the sole authorized `/d2b-spec-edit` progress batch.
Because that batch changes feature content from P to Q, T589 also requires fresh analysis and
unanimous plan review bound to exact clean C/Q; the earlier B/P sign-off cannot authorize
implementation dispatch.
T590, T591, and T594 are file-disjoint and launch together from the T589 prep commit.
T592 launches only after T591 because both tasks serially own `transaction.rs`: T591 first
removes store-side policy interpretation, then T592 becomes the sole audit/replay writer.
T593 launches only after T592 because it consumes T592's frozen broker operation and FFI
quarantine; it owns no Cargo manifest or lockfile.
T605 launches after T593 so it can regenerate the one shared API-snapshot set after both the
registrar surface reduction and the Zone enum addition.
T595 is the only daemon-composition writer. T596-T599 and T604 are file-disjoint. T220 is the
integrator convergence and immutable-candidate boundary; it completes every repository
change, including generated-manifest reconciliation, before freezing F. T600 and T601 are
read-only evidence lanes and write only delivery evidence outside the repository.
Current panel rounds, checkpoints, and commit tags use qualified lowercase `adr046w5`; `W5`
above is only the manifest label. The C1 correction is approved and fully assigned to T605,
but implementation remains gated on successful cross-artifact analysis, unanimous plan
signoff, and T603 progress reconciliation.

Completion-slice fragments are file-disjoint task ownership: T589 solely owns
`changelog.d/resource-api-production.md`; T590
`changelog.d/resource-policy-bootstrap.md`; T591
`changelog.d/store-policy-neutrality.md`; T592
`changelog.d/resource-bundle-audit-carrier.md`; T593
`changelog.d/componentsession-peer-admission.md`; T594
`changelog.d/controller-effect-ledger.md`; T595
`changelog.d/zone-runtime-production.md`; T596
`changelog.d/authenticated-publication-acceptance.md`; T597
`changelog.d/effect-replay-acceptance.md`; T598
`changelog.d/audit-acceptance.md`; T599
`changelog.d/cli-operation-recovery.md`; T604
`changelog.d/operator-resource-activation.md`; and T605
`changelog.d/system-core-handlers.md`. T603 uniquely owns
`changelog.d/delivery-resume-reconciliation.md`. Each path supplements its task row's owned
files. T603 is the integrator-owned validator prerequisite, not a slice, and its authorization
is exactly the two Rust files `packages/xtask/src/delivery/mod.rs` and
`packages/xtask/src/delivery/resume.rs` plus that mandatory fragment; T600-T602 and T219 write only
external evidence or delivery state, and T220 only folds. T220 requires exactly this
fourteen-fragment
set; a missing, duplicate, differently named, or cross-owned fragment blocks convergence.

### Group `wi:ADR-046-cli-and-operations` (13 items)

- [ ] T073 [US1] `ADR046-cli-001` - `packages/d2b/src/lib.rs` (adapt)
- [ ] T074 [US1] `ADR046-cli-002` - `packages/d2b/src/guest.rs` (`d2b guest start/stop/restart/list/status`) (adapt)
- [ ] T075 [US1] `ADR046-cli-003` - `packages/d2b/src/exec.rs` (`d2b exec run/attach/wait/status/list/logs/kill`) (adapt)
- [ ] T076 [US1] `ADR046-cli-004` - `packages/d2b/src/shell.rs` (`d2b shell open/attach/list/detach/kill/status`) (adapt)
- [ ] T077 [US1] `ADR046-cli-005` - `packages/d2b/src/provider.rs` (adapt)
- [ ] T078 [US1] `ADR046-cli-006` - `packages/d2b/src/complete.rs` (`d2b complete bash/zsh/fish`) (adapt)
- [ ] T079 [US1] `ADR046-cli-007` - `packages/d2b/src/activation.rs` (`d2b activation build/generations/switch/boot/test/rollback/gc/migrate/keys/trust/rotate-known-host/config`) (adapt)
- [ ] T080 [US1] `ADR046-cli-008` - `packages/d2b/src/host.rs` (all `d2b host` subcommands) (adapt)
- [ ] T081 [US1] `ADR046-cli-009` - `packages/d2b/src/zone.rs` (`d2b zone get/list/status`) (adapt)
- [ ] T082 [US1] `ADR046-cli-010` - `packages/d2b/src/resource.rs` (standard `d2b get/list/watch/create/update-spec/delete/status` top-level verbs) (adapt)
- [ ] T083 [US1] `ADR046-cli-011` - Nix: `nixos-modules/options-zones.nix` (replace)
- [ ] T084 [US1] `ADR046-cli-012` - `packages/d2b/src/endpoint.rs` (`d2b endpoint get/list/watch/status/resolve`) (adapt)
- [ ] T085 [US1] `ADR046-cli-013` - `packages/d2b/src/share.rs` (`d2b export …` and `d2b import …` nouns) (adapt)

### Group `wi:ADR-046-nix-configuration` (35 items)

- [ ] T086 [US1] `ADR046-nix-001` - `nixos-modules/options-zones.nix` (adapt)
- [ ] T087 [US1] `ADR046-nix-002` - `Network` resource fields in `nixos-modules/options-zones-resources.nix` (adapt)
- [ ] T088 [US1] `ADR046-nix-003` - `nixos-modules/options-site.nix` (retained) (adapt)
- [ ] T089 [US1] `ADR046-nix-004` - `nixos-modules/index.nix` (rewritten) (adapt)
- [ ] T090 [US1] `ADR046-nix-005` - `nixos-modules/bundle-zones.nix` (per-Zone bundle derivation) (adapt)
- [ ] T091 [US1] `ADR046-nix-006` - `nixos-modules/resources-zones-processes.nix` (adapt)
- [ ] T092 [US1] `ADR046-nix-007` - `nixos-modules/resources-zones-volumes.nix` (adapt)
- [ ] T093 [US1] `ADR046-nix-008` - Compiler-only `parentZone` map in `nixos-modules/options-zones.nix` (adapt)
- [ ] T094 [US1] `ADR046-nix-009` - Provider/display-wayland and Provider/shell-terminal Process configs in `zones/<z>/resource-bundle.json` (adapt)
- [ ] T095 [US1] `ADR046-nix-010` - User-only `Host` resource in `zones/<z>/resource-bundle.json` (adapt)
- [ ] T096 [P] [US1] `ADR046-nix-011` - `nixos-modules/privileges-json.nix` (retained baseline only; T592 later adapts it for the handoff op) (copy-unchanged)

T096's manifest disposition covers only the pre-handoff retained matrix and is not a claim
that Wave 5 privileges remain unchanged. The new `ApplyHostGenerationHandoff` operation
cannot inherit that disposition: T592 is its serialized sole writer and must update the
canonical Rust matrix, Nix renderer, generated schemas/catalogues, parity tests, and reference
table before the operation exists.
- [ ] T097 [US1] `ADR046-nix-012` - `nixos-modules/closures-json.nix` (adapt)
- [ ] T098 [US1] `ADR046-nix-013` - Per-Zone `zones/<z>/resource-bundle.json` (`schemaVersion`) (replace)
- [ ] T099 [US1] `ADR046-nix-014` - `nixos-modules/assertions.nix` (adapt)
- [ ] T100 [US1] `ADR046-nix-015` - Same files (adapt)
- [ ] T101 [US1] `ADR046-nix-016` - Network reconciliation by `Provider/network-local` Process resources (copy-unchanged)
- [ ] T102 [US1] `ADR046-nix-017` - Per-VM store reconciliation by `Provider/volume-virtiofs` EphemeralProcess/Process resources (copy-unchanged)
- [ ] T103 [US1] `ADR046-nix-018` - `Provider/device-tpm` (replace)
- [ ] T104 [US1] `ADR046-nix-019` - `docs/reference/schemas/v3/<ResourceType>.json` for each ResourceType (adapt)
- [ ] T105 [US1] `ADR046-nix-020` - Configuration-publication controller handler in `packages/d2b-core-controller/src/configuration.rs` (create)
- [ ] T106 [US1] `ADR046-nix-021` - `packages/d2b-contract-tests/tests/provider-crate-layout.rs` (create)
- [ ] T107 [US1] `ADR046-nix-022` - `nixos-modules/artifact-catalog.nix` (new emitter) (create)
- [ ] T108 [US1] `ADR046-nix-023` - `packages/d2b-bus/src/session/` (new crate `d2b-bus`) (adapt)
- [ ] T109 [US1] `ADR046-nix-024` - `packages/d2b-bus/src/session/` (same crate as ADR046-nix-023). (adapt)
- [ ] T110 [US1] `ADR046-nix-025` - `packages/d2b-bus/src/session/`. (adapt)
- [ ] T111 [US1] `ADR046-nix-026` - `packages/d2b-bus/src/transport/unix/`. (adapt)
- [ ] T112 [P] [US1] `ADR046-nix-027` - `packages/d2b-contracts/src/v3/component_session.rs`. (adapt)
- [ ] T113 [US1] `ADR046-nix-028` - `packages/d2b-contracts/src/v3/services/`. (adapt)
- [ ] T114 [US1] `ADR046-nix-029` - `packages/d2b-provider/src/` (adapt in place). (adapt)
- [ ] T115 [US1] `ADR046-nix-030` - `packages/d2b-provider-toolkit/src/` (adapt in place). (adapt)
- [ ] T116 [US1] `ADR046-nix-031` - `nixos-modules/resources-sharing.nix` (create)
- [ ] T117 [US1] `ADR046-nix-032` - `packages/d2b-client/src/` (adapt in place). (adapt)
- [ ] T118 [US1] `ADR046-nix-033` - `packages/d2b-bus/src/routing/zone_service.rs`. (adapt)
- [ ] T119 [US1] `ADR046-nix-034` - `packages/d2bd/src/provider_registry.rs` (adapt in place). (adapt)
- [ ] T120 [US1] `ADR046-nix-035` - `packages/d2bd/src/provider_effects.rs` (adapt in place). (adapt)

### Group `wi:ADR-046-resources-device` (7 items)

- [ ] T121 [P] [US1] `ADR046-device-001` - `packages/d2b-contracts/src/v3/device.rs` (adapt)
- [ ] T122 [US1] `ADR046-device-002` - `packages/d2b-provider-device-tpm/src/` (adapt)
- [ ] T123 [US1] `ADR046-device-003` - `packages/d2b-provider-device-usbip/src/` (adapt)
- [ ] T124 [US1] `ADR046-device-004` - `packages/d2b-provider-device-security-key/src/` (adapt)
- [ ] T125 [US1] `ADR046-device-005` - `packages/d2b-provider-device-gpu/src/` (adapt)
- [ ] T126 [US1] `ADR046-device-006` - `nixos-modules/resources-device.nix` (adapt)
- [ ] T127 [US1] `ADR046-device-008` - `packages/xtask/src/main.rs` (`check-provider-layout` subcommand) (adapt)

### Group `wi:ADR-046-resources-host-guest-process-user` (22 items)

- [ ] T128 [P] [US1] `ADR046-exec-001` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [ ] T129 [US1] `ADR046-exec-002` - `packages/d2b-contracts/src/v3/process_provider.rs`: LaunchTicket (adapt)
- [ ] T130 [US1] `ADR046-exec-003` - `packages/d2b-provider-system-core/src/host.rs`: HostReconciler (adapt)
- [ ] T131 [US1] `ADR046-exec-004` - `packages/d2b-provider-system-core/src/user.rs`: UserReconciler (adapt)
- [ ] T132 [US1] `ADR046-exec-005` - `packages/d2b-provider-system-core/src/host.rs` (continued) (adapt)
- [ ] T133 [US1] `ADR046-exec-006` - `packages/d2b-provider-system-systemd/src/`: launch.rs (opaque EffectPort requests) (adapt)
- [ ] T134 [US1] `ADR046-exec-007` - `packages/d2b-provider-system-minijail/src/`: sandbox_compiler.rs (adapt)
- [ ] T135 [US1] `ADR046-exec-008` - `packages/d2b-process-conformance/src/`: shared conformance test matrix run against both system-systemd and system-minijail providers (adapt)
- [ ] T136 [US1] `ADR046-exec-009` - `packages/d2b-provider-system-core/src/host.rs` (user-only no-isolation Host) (adapt)
- [ ] T137 [US1] `ADR046-exec-010` - `packages/d2b-provider-system-systemd/src/guest_exec.rs` (guest-domain EphemeralProcess launch via systemd-run inside guest) (adapt)
- [ ] T138 [US1] `ADR046-exec-011` - guest-domain process attachment becomes a ComponentSession named stream to the EphemeralProcess running in the guest (adapt)
- [ ] T139 [US1] `ADR046-exec-012` - `nixos-modules/options-zones.nix`: `d2b.zones.<zone>.resources` option as `types.attrsOf (types.submodule resourceModule)` where each resource module has `type` (required enum) (adapt)
- [ ] T140 [US1] `ADR046-exec-014` - `nixos-modules/zone-bundle.nix`: Zone resource bundle emitter (adapt)
- [ ] T141 [US1] `ADR046-exec-016` - `packages/d2b-bus-session/src/`: all above modules verbatim (adapt)
- [ ] T142 [US1] `ADR046-exec-017` - `packages/d2b-bus-session-unix/src/`: all above modules verbatim (adapt)
- [ ] T143 [US1] `ADR046-exec-018` - `packages/d2b-bus-wire/src/session.rs`: v3 bus protocol constants and wire types (adapt)
- [ ] T144 [US1] `ADR046-exec-019` - `packages/d2b-provider-runtime/src/`: `registry.rs` (adapt)
- [ ] T145 [US1] `ADR046-exec-020` - `packages/d2b-provider-toolkit/src/`: retain all modules verbatim (adapt)
- [ ] T146 [US1] `ADR046-exec-021` - `packages/d2b-bus-contracts/src/generated_v3_services/` (adapt)
- [ ] T147 [US1] `ADR046-exec-022` - `packages/d2b-bus-client/src/`: all above modules (adapt)
- [ ] T148 [US1] `ADR046-exec-023` - `packages/d2b-zone-router/src/`: `router.rs` (adapt)
- [ ] T149 [US1] `ADR046-user-session-001` - `packages/d2b-core-controller/src/user_session_authority.rs` (or a core/user-agent per-session agent Process under `Provider/system-systemd`) (adapt)

### Group `wi:ADR-046-resources-volume` (6 items)

- [ ] T150 [P] [US1] `ADR046-volume-001` - `packages/d2b-contracts/src/v3/volume.rs` (adapt)
- [ ] T151 [US1] `ADR046-volume-002` - `packages/d2b-provider-volume-local/src/` (adapt)
- [ ] T152 [US1] `ADR046-volume-003` - `packages/d2b-provider-volume-virtiofs/src/` (adapt)
- [ ] T153 [US1] `ADR046-volume-004` - `nixos-modules/resources-volume.nix` (adapt)
- [ ] T154 [US1] `ADR046-volume-005` - `packages/d2b-provider-volume-local/src/` (create)
- [ ] T155 [US1] `ADR046-volume-006` - `nixos-modules/resources-volume.nix` (create)

### Group `wi:ADR-046-resources-zone-control` (26 items)

- [ ] T156 [US1] `ADR046-client-001` - `packages/d2b-client/src/` (adapt)
- [ ] T157 [US1] `ADR046-pkg-001` - `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` (create)
- [ ] T158 [US1] `ADR046-provider-agent-001` - `packages/d2b-provider/src/agent.rs` (v3 provider agent dispatch) (adapt)
- [ ] T159 [US1] `ADR046-wire-001` - `packages/d2b-contracts/src/v3/{services,state,identity,provider}.rs` (adapt)
- [ ] T160 [US1] `ADR046-zone-control-001` - `packages/d2b-contracts/src/v3/zone.rs` (adapt)
- [ ] T161 [US1] `ADR046-zone-control-002` - `packages/d2b-contracts/src/v3/zone_link.rs` (adapt)
- [ ] T162 [US1] `ADR046-zone-control-003` - `packages/d2b-contracts/src/v3/provider.rs` (adapt)
- [ ] T163 [US1] `ADR046-zone-control-004` - `packages/d2b-contracts/src/v3/role.rs` (adapt)
- [ ] T164 [US1] `ADR046-zone-control-005` - `packages/d2b-contracts/src/v3/role_binding.rs` (adapt)
- [ ] T165 [US1] `ADR046-zone-control-006` - `packages/d2b-resource-api/src/authz.rs` (adapt)
- [ ] T166 [US1] `ADR046-zone-control-007` - `nixos-modules/options-zones.nix` (adapt)
- [ ] T167 [US1] `ADR046-zone-control-008` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [ ] T168 [US1] `ADR046-zone-control-009` - `packages/d2b-contracts/src/v3/quota.rs` (create)
- [ ] T169 [US1] `ADR046-zone-control-010` - `packages/d2b-contracts/src/v3/emergency_policy.rs` (create)
- [ ] T170 [US1] `ADR046-zone-control-011` - `packages/d2b-bus/src/{lifecycle,engine,driver,streams,transport,error}.rs` (adapt)
- [ ] T171 [US1] `ADR046-zone-control-012` - `packages/d2b-bus-unix/src/{adapter,socket,pidfd,credit,descriptor,error,systemd}.rs` (adapt)
- [ ] T172 [US1] `ADR046-zone-control-013` - `packages/d2b-contracts/src/v3/component_session.rs` (new v3 namespace in existing contracts crate) (adapt)
- [ ] T173 [US1] `ADR046-zone-control-014` - `nixos-modules/options-zones.nix` (create)
- [ ] T174 [US1] `ADR046-zone-control-015` - `packages/d2b-resource-compiler/src/{main,bundle,schema,validator,digest,sort,secret_lint,generation}.rs` (create)
- [ ] T175 [US1] `ADR046-zone-control-017` - `packages/d2b-provider/src/{registry,rpc}.rs` (adapt)
- [ ] T176 [US1] `ADR046-zone-control-018` - `packages/d2b-core-controller/src/zone_link.rs` (ZoneLink handler) (adapt)
- [ ] T177 [US1] `ADR046-zone-control-019` - `packages/d2b-contracts/src/v3/{resource_export,resource_import}.rs` (adapt)
- [ ] T178 [US1] `ADR046-zone-control-020` - `packages/d2b-core-controller/src/export_import_projection.rs` (local qualified Service projection lifecycle owned by `ResourceImport`) (create)
- [ ] T179 [US1] `ADR046-zone-control-022` - `packages/d2b-core-controller/src/authority.rs` (adapt)
- [ ] T180 [US1] `ADR046-zone-control-023` - `packages/d2b-core-controller/src/{quota,emergency_policy}.rs` (adapt)
- [ ] T181 [US1] `ADR046-zone-control-024` - `packages/d2b-core-controller/src/authority.rs` (Host-global index scope + hardware admission) (adapt)

### Group `wi:ADR-046-telemetry-audit-and-support` (26 items)

- [ ] T182 [P] [US1] `ADR046-audit-001` - `packages/d2b-audit/src/{hash_chain.rs,segment.rs,rate_limit.rs,record_types.rs,sink.rs,export.rs}` (adapt)
- [ ] T183 [US1] `ADR046-audit-002` - `packages/d2b-resource-store-redb/src/audit.rs` (adapt)
- [ ] T184 [US1] `ADR046-audit-003` - `packages/d2b-session/src/audit.rs` (adapt)
- [ ] T185 [US1] `ADR046-audit-004` - `packages/d2b/src/zone_audit.rs` (new `d2b zone audit export` subcommand) (adapt)
- [ ] T186 [US1] `ADR046-doctor-001` - `packages/d2b/src/zone_doctor.rs` (adapt)
- [ ] T187 [US1] `ADR046-doctor-002` - `packages/d2b/src/zone_support_bundle.rs` (adapt)
- [ ] T188 [US1] `ADR046-host-posture-001` - `packages/d2b-provider-system-core/src/{host_reconciler.rs,host_status.rs,host_process_audit.rs}` (adapt)
- [ ] T189 [US1] `ADR046-reuse-001` - `packages/d2b-session/` copied verbatim (adapt)
- [ ] T190 [US1] `ADR046-reuse-002` - `packages/d2b-session-unix/` copied verbatim. (adapt)
- [ ] T191 [US1] `ADR046-reuse-003` - `packages/d2b-client/` copied (adapt)
- [ ] T192 [US1] `ADR046-reuse-004` - `packages/d2b-provider/` and `packages/d2b-provider-toolkit/` copied with v3 session admission and bus routing adaptations. (adapt)
- [ ] T193 [US1] `ADR046-reuse-005` - `packages/d2b-provider-observability-otel/src/agent.rs` adapted (adapt)
- [ ] T194 [US1] `ADR046-reuse-006` - `packages/d2b-bus/src/routing.rs` adapted from `service_v2.rs` (adapt)
- [ ] T195 [US1] `ADR046-reuse-007` - `packages/d2b-bus/src/service_router.rs` and `packages/d2b-core-controller/src/provider_effects.rs`. (adapt)
- [ ] T196 [US1] `ADR046-reuse-008` - `packages/d2b-contract-tests/tests/component_session_v2_vectors.rs` and `tests/noise_vectors.rs` copied verbatim. (adapt)
- [ ] T197 [US1] `ADR046-reuse-009` - `packages/d2b-telemetry/src/session_metrics_sink.rs`. (adapt)
- [ ] T198 [P] [US1] `ADR046-telem-001` - `packages/d2b-telemetry/src/{trace_context.rs,audit_hash.rs,emitter.rs,meter_registry.rs,metric_label_policy.rs,redaction_guard.rs}` (adapt)
- [ ] T199 [US1] `ADR046-telem-002` - `packages/d2b-resource-store-redb/src/metrics.rs` (adapt)
- [ ] T200 [US1] `ADR046-telem-003` - `packages/d2b-resource-api/src/metrics.rs` (adapt)
- [ ] T201 [US1] `ADR046-telem-004` - `packages/d2b-core-controller/src/metrics.rs` (adapt)
- [ ] T202 [US1] `ADR046-telem-005` - `packages/d2b-provider-supervisor/src/metrics.rs` (adapt)
- [ ] T203 [US1] `ADR046-telem-006` - `packages/d2b-provider-observability-otel/src/` (adapt)
- [ ] T204 [US1] `ADR046-telem-007` - `packages/d2b-provider-observability-otel/src/nix/journald.nix` (new Nix fragment) (adapt)
- [ ] T205 [US1] `ADR046-telem-008` - `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (new) (adapt)
- [ ] T206 [P] [US1] `ADR046-telem-009` - `nixos-modules/resources.nix` (adapt)
- [ ] T207 [US1] `ADR046-telem-010` - `nixos-modules/resources-bundle.nix` (build-time validation step 4 in the `resources-bundle` derivation) (adapt)

### Group `wi:core-config-hub:w5` (6 items)

- [ ] T208 [US1] `ADR046-device-007` - `packages/d2b-core-controller/src/configuration.rs` (create)
- [ ] T209 [US1] `ADR046-exec-013` - `packages/d2b-core-controller/src/cleanup.rs`: EphemeralProcess TTL cleanup controller handler (create)
- [ ] T210 [US1] `ADR046-exec-015` - `packages/d2b-core-controller/src/configuration.rs`: `ZoneConfigController` (create)
- [ ] T211 [US1] `ADR046-telem-011` - `packages/d2b-core-controller/src/{configuration.rs, ownership.rs}` (adapt)
- [ ] T212 [US1] `ADR046-zone-control-016` - `packages/d2b-core-controller/src/configuration/{mod,bundle_apply,generation_transition}.rs` (adapt)
- [ ] T213 [US1] `ADR046-zone-control-021` - `packages/d2b-core-controller/src/{coordinator,configuration,zonelink}.rs` (adapt)

### Group `wi:reconciliation-real-backend:w5` (1 items)

- [ ] T214 [US1] `ADR046-reconcile-003` - `packages/d2b-controller-toolkit/benches/reaction.rs` (adapt)

### Group `wi:resource-store-backend:w5` (1 items)

- [ ] T215 [US1] `ADR046-store-004` - `packages/d2b-resource-store-redb/src/lib.rs` (adapt)

### Group `wi:resource-store-integration:w5` (2 items)

- [ ] T216 [US1] `ADR046-store-003` - `packages/d2b-contracts/src/v3/storage.rs` (adapt)
- [ ] T217 [US1] `ADR046-store-005` - `packages/d2b-resource-store-redb/src/backup.rs` (adapt)

### Group `wi:resource-store-watch:w5` (1 items)

- [ ] T218 [US1] `ADR046-store-002` - `packages/d2b-resource-store-redb/src/revision_log.rs` (adapt)

- [X] T577 [US1] **Publish the desktop-companion inventory** as a versioned reference document naming each companion, its exact consumed surface, and its verification status (FR-039, contracts/companion-contracts.md CO-1). Published at W5, not at release, so companions have time to adapt. **Done**: `docs/reference/companion-contracts.md` revision 1, landed at `b72b205f`; all five rows read "Pending live-host verification", so publication claims no compatibility
- [X] T578 [US1] **Publish the replacement contracts the companions consume**, early enough for them to adapt given that no preview release may be published (contracts/companion-contracts.md CO-2, FR-045). **Done**: `docs/reference/zone-cli-contract.md` revision 1, landed at `b72b205f`. CO-5 remains the W5 exit condition: every "surface consumed" cell in the inventory must resolve to a committed contract at a public ref
- [X] T579 [US1] **Resolve the FR-039 / FR-045 tension before these contracts publish** (CHK025). FR-039 blocks release on external repositories while FR-045 forbids the preview build they would adapt against. This is the last moment the choice is cheap: resolve it here or amend FR-045. **Done, out of order**: T577 and T578 published first, so the resolution was encoded in shipped prose before any requirement said it. Closed by **FR-061** (contract/artifact boundary, publish-adapt-verify sequencing, per-stage refusals, amendment-only relaxation of FR-045) and **FR-062** (the adaptation assumption recorded as unvalidated with a mitigation, a detection point, and an escalation path). FR-045 is preserved, not amended. See `checklists/coverage.md`, "The W5 date-bound gate"

### Approved production resource-plane completion

- [ ] T603 [US1] **`adr046w5` RESUME RECONCILIATION - implement, re-attest, then bind the amended plan gate before T589.** First freeze clean pre-validator base A and feature snapshot P0 and require exactly one T072 disposition: checked T072 backed by exact contemporaneous historical evidence, or unchecked T072 plus one passing `historical-entry-remediation-t072` record bound to A/P0. Absence, duplication, wrong base/snapshot, or a current rerun labeled historical refuses analysis, plan review, and every T603 source change. This disposition does not check T072 or claim implementation evidence. A current no-HIGH/CRITICAL cross-artifact analysis and unanimous `adr046w5-r<n>` plan panel bound to A/P0 then authorize only T603's validator implementation scope. T603 owns exactly three repository paths: the two Rust source files `packages/xtask/src/delivery/mod.rs` and `packages/xtask/src/delivery/resume.rs`, plus the mandatory unique fragment `changelog.d/delivery-resume-reconciliation.md`. It may change no other repository path and may place only receipts outside Git under `.scratch/autopilot/adr046w5/`. Implement the reusable hermetic validator and its table-driven negative suite through the existing `make test-rust` workspace gate, write that fragment, then land one dedicated validator-and-fragment commit V with sole parent A and exactly those two Rust files plus the fragment. A missing or differently named fragment makes V invalid. T603's fragment obligation is complete when the exact unique fragment is created and validated in V; it remains unfolded until T220 alone folds it after later convergence, and no T220 action is a prerequisite of T603. Freeze post-validator resume base B exactly at V and compute feature snapshot P; because V cannot edit this feature root, P MUST be byte-identical to P0. Before any reconciliation receipt or checkbox edit, revalidate the same exclusive T072 disposition and A-to-B ancestry, rerun cross-artifact analysis over A..B plus the full feature artifacts, and rerun the unanimous plan panel against B/P, with both receipts naming B and P. Any post-validator finding or validator-code change invalidates B. A source-only or fragment-only fix requires a new V/B and both post-validator gates; a finding that requires a feature-artifact edit returns to a fresh `/d2b-spec-edit` batch, establishes a new A/P0, and reruns the entire pre-validator and post-validator sequence. Pre-validator receipts never authorize resume. Only after the post-validator analysis has no unresolved HIGH/CRITICAL finding and the post-validator plan panel is unanimous may T603 audit every T073-T218 obligation against B and delivery records; code presence alone is never completion evidence. Using the fd-anchored `openat2` and durable write protocol in `plan.md`, create the immutable schema-v2 authorization receipt at `.scratch/autopilot/adr046w5/reconciliation.json`. It binds opaque project sentinel `7f6d0beab0ce4c13a89f6865d5ac42e2`, never a hosting domain, account, remote URL, or checkout path; Git-discovered repository root; repository-relative feature path `specs/001-adr046-d2b3-completion`; the exact 28-file pre-edit snapshot P; the validator-computed authorized post-edit snapshot Q; B and its tree; branch; the post-validator analysis receipt; the post-validator qualified plan panel with ten record locators; the exact 147 changed task IDs; and exactly 146 unique ordered T073-T218 rows carrying obligation identity, `satisfied|open`, evidence kind, and qualifying commit/receipt locator. Reject unknown fields/statuses, missing or extra rows, stale or mismatched identities, dirty staged/unstaged/relevant-untracked state, nonlocal locators, symlinks, mount crossing, weak permissions, partial writes, and receipt payloads containing diffs, transcripts, command/validation output, secrets, credentials, store paths, or raw sink details. If any row is `open`, leave T603 unchecked and change no checkbox. Otherwise route one explicit `/d2b-spec-edit` apply whose only feature changes check T073-T218 and T603; T072 remains unchanged. The editor holds and revalidates the original `tasks.md` inode, `fsync`s the replacement, publishes with dirfd-relative `renameat2(RENAME_EXCHANGE)`, verifies the displaced inode is the original, removes it with `unlinkat`, and `fsync`s the feature directory; exchange unavailability or any mismatch fails closed and restores the original. The Wave 5 integrator alone owns dedicated checkbox commit C; require `C^ = B`, exact diff P-to-Q, and no second parent. Finalize `progress-editor-receipt.json` only after C exists. Resume idempotently from exactly B/P, permitted unstaged or staged B/Q, or C/Q as specified in `plan.md`; every other state refuses. **Done when** exactly one T072 disposition validates without changing T072, the pre-validator A/P0 and post-validator B/P receipts are distinct and valid, B equals V, P equals P0, malformed receipt, wrong-root/path-race, pre/post transition, replacement-race/exchange-rollback, crash-after-edit, crash-after-stage, crash-after-commit, duplicate finalize, permission, file-and-directory-sync, and ancestry tests pass; the finalized receipt binds B, C, P, Q, the authorization digest, and the exact changed-ID set; HEAD is clean at C; exactly T073-T218 plus T603 are checked by this transition; and the mandatory unique fragment exists at its exact path and passes T603 validation for T220's later sole fold.
  **Constitutional predecessor:** FR-036's separate Principle VI amendment must be accepted
  and its commit must be an ancestor of A before the A/P0 panel can authorize any T603 source
  change. Analysis or panel receipts gathered before that amendment are non-authorizing and
  must be rerun on a descendant base. T072 and `historical-entry-remediation-t072` remain
  evidence only and cannot replace this predecessor.

  **Future post-editor plan gate before T589:** dedicated checkbox commit C changes the reviewed
  feature snapshot from P to Q, so the B/P panel may authorize only the editor transition
  and is stale for implementation dispatch. After the finalized progress receipt exists,
  require clean HEAD C/Q, fresh `/speckit-analyze` with no unresolved HIGH or CRITICAL
  finding, and a fresh unanimous ten-role `/d2b-panel-round plan` review whose records bind
  exact C and Q. Any later content or history change invalidates this gate. Do not dispatch
  T589 until all of these conditions pass; no prior sign-off transfers.
- [ ] T589 [US1] **`adr046w5` INTEGRATOR PREP - freeze the shared production contracts before parallel work.** Its sole in-feature direct dependency is T603 and it semantically depends on all T073-T218 obligations reconciled there. It has two unresolved external dispatch prerequisites. First, a separate external specification-amendment workflow must bump accepted `ADR-046-validation-and-delivery` from Version 1 to Version 2 and normatively pin the five SC-002 incident commands, including pre-signing successor freeze and canonical disposition-request creation; every stable-id `recovery-resumable`, `recovery-irreconcilable`, and terminal state; the closed cause and five-value remediation table; exact exits, thirteen-line human output, and distinct JSON; fresh-successor disposition; the exact same frozen successor/request/triplet binding through apply and admission; `Sc002IncidentDispositionV1` canonical encoding and Ed25519 authority/key/signature binding; a durable structured `Sc002IncidentPreimageV1` containing every kind-specific component and repeated byte-identically by anchor/metadata/status/resolution/request/disposition/freeze/admission records; temporary-write/file-sync/no-replace/reopen/parent-and-every-ancestor-sync publication and idempotent recovery; verified-payload `parked`, no-unlink residue-backed `mismatch-retained`, and frozen-primary-evidence resolution branches outside ephemeral namespaces; complete and identity-bearing recursively enumerated bounded-failure census encodings that exclude resolution leaves and never authorize from raw `01ff`; separate preimage, anchor, metadata, durable-status, resolution, CLI-status, freeze, request, and disposition schemas; the complete `CanonicalRetiredCensusV1` framing/tag/unavailable/sentinel/ordering contract and golden vectors; the shared SC-002 typed domain-hash golden and exact receipt/census negative registries; collision-safe retirement identity; private nonserializable `SidecarCleanupOwner`; private zero-mutation candidate-retention owner; recursive whole-scope retention guard forbidding candidate-root deletion; source-floor canonical hash, exact 15-digest/four-signature vector encoding, disposition-pinned issuer-proof/copied-digest rejection contract, private nonserializable authenticated-issuer result, and independent closed negative registries; and typed-validator consumption. That amendment must receive the parent ADR's required pre-panel and post-panel approvals, regenerate `docs/specs/ADR-046-spec-set.json`, `ADR-046-work-items.json`, and `ADR-046-implementation-graph.{json,md}`, pass Gate 0 and drift validation on its exact commit, and be an ancestor of T589's base. T589 owns none of that external amendment or regeneration and refuses every source change before it validates. Second, the accepted source-generation compatibility disposition required by FR-070 must name one concrete source-floor producer/installer owner and one concrete typed import/validation authority, pin each transition authority's Ed25519 verification key, and pin the exact typed validator artifact/API. That owner must atomically install the source 3/1 generation's exact nonempty 13-member census, and that authority must complete the canonically encoded, length-framed, domain-separated, ordered, issuer-authenticated `SourceGenerationCompatibilityFloorV1` manifest, installation, validation, and exact-C/Q import chain from `data-model.md`, including strict integer/text/unknown policies, schemas, and independently recomputed golden vectors. T589 is only a read-only dispatch consumer and is not the producer, installer, validator, or importer. The disposition-pinned validator must first return private nonserializable `AuthenticatedSourceFloorIssuerProvenance` only after all four canonical proofs verify under disposition-selected keys, then consume it by value to return private nonserializable `ValidatedSourceGenerationCompatibilityFloor`. Both types have no public fields, constructors, accessors, serde implementations, `Clone`, `Copy`, `Default`, conversion, or byte importer. A caller-supplied or directly decoded receipt chain, copied digest tuple, or serialized intermediate is ineligible. Every closed role occurs exactly once and every member binds the same accepted disposition and source generation. Every `missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, or `cross-disposition` member refuses, as does any missing/wrong/copied issuer proof even when every copied authority and verification-key digest is present and every non-authentication enclosing hash is recomputed. The five copied-issuer negatives attack manifest, installation, validation, import, and all proofs with correct transition domains and otherwise canonical chains signed by unpinned valid test keys; each must reach the named disposition-pinned issuer verification before refusal, and the all-proof case reports the complete four-transition failure set. Bare committed protocol 4, a target-only binary, synthetic source image, prose claim, new unit or override, child supervisor, entrypoint mutation path, or daemon recovery owner does not satisfy this prerequisite. Refuse unless the final import receipt names clean C/Q and the exact migration source generation, the immutable authorization validates at B/P, the finalized progress receipt validates dedicated checkbox commit C with `C^ = B` and snapshot Q, HEAD is clean at C, the receipt has 146 `satisfied` rows, and `tasks.md` shows T073-T218 plus T603 checked. Its immediate contract inputs include T165, T170-T172, T182-T184, T195, and T208-T218. Owned files: `packages/d2b-resource-store/src/lib.rs`, `packages/d2b-resource-api/src/{adapter.rs,client.rs,service.rs,generated/d2b_resource_v3_ttrpc.rs}`, `packages/d2b-contracts/proto/d2b-resource-v3.proto`, `packages/d2b-contracts/src/generated/d2b_resource_v3.rs`, `packages/d2b-core-controller/src/runtime.rs`, `packages/d2b-resource-store-redb/src/{audit.rs,transaction.rs}`, `packages/d2b-bus/src/router.rs`, `packages/xtask/src/delivery/{command.rs,evidence.rs,panel.rs,seal.rs,eligibility.rs}`, accepted normative specification `docs/specs/ADR-046-resource-api-and-authorization.md`, `docs/reference/schemas/delivery/sc002-incident-{preimage,anchor,metadata,status,resolution,cli-status,disposition-request,disposition}-v1.schema.json`, `docs/reference/schemas/delivery/sc002-successor-freeze-v1.schema.json`, `tests/golden/delivery/sc002-incident-{human,json}-v1.txt`, `tests/golden/delivery/sc002-{successor-freeze,incident-disposition-request,incident-disposition}-v1.json`, `tests/golden/delivery/sc002-domain-hash-vectors-v1.json`, the independent expected-set and registry fixtures `tests/golden/delivery/{host-generation-mutation-edge-ids,host-generation-apply-peer-case-ids,host-generation-mutation-edge-meta-negative-case-ids,host-generation-post-first-negative-case-ids,sc002-sidecar-lock-case-ids,sc002-activation-receipt-negative-case-ids,sc002-census-negative-case-ids}.txt`, `tests/golden/delivery/host-generation-apply-peer-forbidden-values.tsv`, and `tests/golden/delivery/source-floor-v1/{role-artifact-matrix.tsv,poison-case-ids.txt,matrix-meta-negative-case-ids.txt,issuer-proof-negative-case-ids.txt,issuer-authentication-negative-case-ids.txt,hash-vector-negative-case-ids.txt,receipt-negative-case-ids.txt}`, `tests/golden/api-surface/{roots.json,capability-api.txt,capability-trait-impls.txt,hidden-public-api.txt,public-api.txt,workspace-metadata.json}`, and the dependency edges in `packages/{d2bd,d2b-bus,d2b-resource-api,d2b-core-controller,d2b-resource-store-redb}/Cargo.toml`. Establish the shared sealed interfaces for registrar-consumed session admission; authenticated post-install policy revision reads; an engine-neutral bounded controller-ledger port; the transactional immutable audit-journal hook and separate export-completion state; aggregate readiness observations; and a typed operation-status store request/result. Define `PolicyBootstrapRead` behind one private sealed issuer with no public constructor, fields, accessors, `Default`, `Clone`, `Copy`, `From`/`TryFrom`, capability conversion/trait implementation, or reconstruction path; register it as a capability root, add defining-crate compiler trait-solver seals, and regenerate the prep API snapshots. Add ResourceService `InspectOperation`, with a bounded request carrying `RequestMeta`, one exact 16-byte operation ID, watch intent, and deadline, and a closed response carrying the same ID plus exactly one of committed-pending-audit status or the stored typed final-result envelope; unknown and wrong-binding remain indistinguishable. Add protobuf `PendingAuditStatus { uint32 mutation_ordinal = 1; ResourceIdentity target = 2; bytes canonical_resource_status_json = 3; }`; add optional `pending_audit_status` fields 8, 8, 4, 4, 4, 5, and 5 respectively to `CreateResponse`, `UpdateSpecResponse`, `UpdateStatusResponse`, `UpdateMetadataResponse`, `UpdateFinalizersResponse`, `DeleteResponse`, and `UpgradeResponse`, and repeated field 5 to `CommitBatchResponse`. The field is absent on ordinary success and carries the exact bounded canonical `ResourceStatus` composite on committed-pending-audit; regenerate both Rust protobuf outputs and require the ResourceService schema fingerprint to change while Resource JSON `apiVersion`/`schemaVersion` stay unchanged. Bump accepted `ADR-046-resource-api-and-authorization` from Version 2 to Version 3 and normatively assign the additive pending-status fields, `InspectOperation`, exact authorization behavior, and generated bindings. Consume the separately approved external `ADR-046-validation-and-delivery` Version 2 contract without editing it; T220 coordinates paired references, contract tests, schemas/goldens, and changelog treatment after T589 but cannot substitute for the pre-dispatch amendment, approval, manifest-regeneration, and Gate 0 prerequisite. Freeze the `transaction.rs` journal/status calls so T591 may change policy-neutral transaction internals and T592, serialized after T591, may own final audit/replay persistence. Implement an `adr046w5` closed-evidence profile in the delivery validator and invoke it from panel-request/panel-attest, seal, and merge-eligibility. Its exact multiset is the five T600 and three T601 `(lane, validation)` pairs in `plan.md`; table-driven tests must reject missing, extra, duplicate, unknown, wrong-lane, and conflated mappings, while the exact eight pass. The source-floor production registry, independent 13-row role/artifact matrix, poison generator, and expected-id fixture must be mutually read-independent and exactly equal. The poison generator must visit all 13 canonical role/artifact pairs for all seven poison classes, keep vector and declared cardinality at 13 through one-for-one substitutions, recompute every enclosing hash, and re-sign the otherwise-valid fixture with test-only keys. Its exact visited count is 91; overlapping set errors assert their complete error set rather than first-error order. Independently reconstruct the accepted Version 2 `hash-vectors-v1.json` exact 15 digest and four signature records. Run the exact five copied-issuer, 20 issuer-authentication/capability, 21 hash-vector, and 32 receipt/transition negative registries. A stale enclosing hash, bad signature, wrong cardinality, unvisited role, early structural failure, missing visit, or registry learned from production makes the matrix itself fail. **Done when** both external prerequisites validate on the exact base before the first source edit; the prep commit compiles every affected crate; capability compiler/API seals and protobuf round-trips include `InspectOperation`, `DeleteResponse`, and batch ordinals; the evidence-profile positive and all six negative classes pass at every named stage; all 91 source-floor poisons reach their intended semantic checks only after authentication and canonical-envelope validation; all five copied-issuer cases fail only at the named pinned-key checks; all 20 authentication/capability and 21 vector negatives reach only their named checks; every one of the 15 digest and four signature vectors is independently reconstructed; copied provenance produces no authenticated issuer result; both T603 receipts remain valid by base/transition identity; no interface exposes a subject constructor, reusable bootstrap reader, independent mutation path, raw audit identifier, production-ready boolean setter, constructible/serializable authenticated issuer provenance, or constructible/serializable validated-floor capability; every T590, T591, and T594 branch is cut from that exact commit; T592 remains blocked on T591; and T605 remains blocked on T593.
  **Pinned independent expected sets:** T589 writes the following closed fixture and shared
  oracle set directly from this task contract. No expected-set fixture may import, call,
  parse, or enumerate the production registry or poison generator.

  - `host-generation-mutation-edge-ids.txt` contains exactly the 15 unique
    newline-terminated mutation ids below in the displayed order. It is authored separately
    from both production and the 90-case file. Production may not read it. A separately
    authored literal 15-id test constant must equal both this fixture and production order
    before running a case; neither expected cardinality nor visits may read
    `mutation_edge_count`, registry length, discovered hooks, or another runtime count.

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

  - `host-generation-apply-peer-case-ids.txt` contains exactly 90 unique newline-terminated
    ids: six `apply-peer/pre-first/<transition>` ids and 84
    `apply-peer/post-first/<edge>/<transition>` ids. The ordered transition axis is exactly
    `peer-exit`, `peer-exec`, `peer-pid-reuse`, `peer-start-identity-mismatch`,
    `peer-executable-identity-mismatch`, `peer-identity-ambiguity`. Its ordered edge axis is
    the independently pinned 15-id file above; the first edge is used only by the pre-first
    prefix and the remaining 14 each pair with all six transitions.

  - `host-generation-mutation-edge-meta-negative-case-ids.txt` contains exactly these three
    newline-terminated ids in order:
    `mutation-edge-meta/production-edge-removed`,
    `mutation-edge-meta/expected-edge-removed`, and
    `mutation-edge-meta/verification-hook-removed`. The first removes one edge from the
    production catalogue while preserving the literal constant and fixture; the second
    removes one fixture edge while preserving the literal constant and production; the
    third preserves all 15 ids but removes one immediately-before-mutation verification
    hook. Each must fail before evidence acceptance. A shared shrunken count cannot satisfy
    any case.

  - `host-generation-post-first-negative-case-ids.txt` contains exactly these 15 unique
    newline-terminated ids, in order:
    `post-first-negative/missing-edge`,
    `post-first-negative/duplicate-edge`,
    `post-first-negative/unknown-edge`,
    `post-first-negative/reordered-edge`,
    `post-first-negative/empty-edge-set`,
    `post-first-negative/missing-transition`,
    `post-first-negative/duplicate-transition`,
    `post-first-negative/unknown-transition`,
    `post-first-negative/unvisited-case`,
    `post-first-negative/dynamic-case-skipped`,
    `post-first-negative/verification-hook-missing`,
    `post-first-negative/selected-edge-mutated`,
    `post-first-negative/successor-mutated`,
    `post-first-negative/durable-prefix-changed`, and
    `post-first-negative/first-audit-missing`. Each poison must reach its named matrix
    invariant. A structural failure before that check is not a visit. A separately authored
    literal 15-id constant must equal the file; empty production/expectation sets and
    dynamically skipped cases fail before evidence acceptance.

  - `host-generation-apply-peer-forbidden-values.tsv` contains exactly the fifteen literal
    tab-separated rows in `data-model.md`, in order, with no header or blank line. Inject
    each literal independently into every pre-first and post-first scenario and scan every
    named persistence/output surface. Only this fixture and the test's private injection
    buffer are excluded. Require the independently computed class-specific correlation
    digest where allowed; metrics contain neither raw nor digested peer identity. Missing,
    duplicate, unknown, changed, or unvisited canary rows and production reads of this file
    fail.

  - `source-floor-v1/role-artifact-matrix.tsv` contains exactly the 13 role/artifact rows in
    the table in the next bullet, newline-terminated in table order and encoded as
    `<role>\t<artifactId>` with no header, comments, blanks, escapes, or extra columns. The
    production registry, this matrix, the poison generator, expected-id list, and a
    separately authored literal 13-row test constant are mutually read-independent and must
    agree exactly. No expected cardinality reads a production or fixture count.

  - `source-floor-v1/poison-case-ids.txt` contains exactly 91 unique
    newline-terminated `source-floor/<class>/<role>` ids in class-major then role-major order.
    The class axis is exactly `missing`, `duplicate`, `extra`, `empty`,
    `stale-generation`, `stale-digest`, `cross-disposition`. The role/artifact axis is
    exactly:

    | Role | Artifact id |
    | --- | --- |
    | `source-daemon-peer` | `source-daemon-peer-v1` |
    | `source-broker-peer` | `source-broker-peer-v1` |
    | `source-wire-schema` | `source-handoff-wire-schema-v1` |
    | `source-privilege-schema` | `source-handoff-privilege-schema-v1` |
    | `source-operation-catalogue` | `source-handoff-operation-catalogue-v1` |
    | `source-operation-catalogue-fingerprint` | `source-handoff-v1` |
    | `source-compatibility-disposition` | `source-compatibility-disposition-v1` |
    | `source-capability-api-fingerprint` | `source-capability-api-fingerprint-v1` |
    | `source-serialization-snapshot` | `source-handoff-serialization-snapshot-v1` |
    | `source-positive-fixture` | `source-handoff-positive-fixture-v1` |
    | `source-bare-protocol-negative-fixture` | `source-bare-protocol-negative-fixture-v1` |
    | `source-cross-fingerprint-negative-fixture` | `source-cross-fingerprint-negative-fixture-v1` |
    | `source-installed-apply-object` | `source-installed-apply-object-v1` |

    Every one of the 39 `missing`, `stale-digest`, and `cross-disposition` cases preserves
    both vector and declared cardinality 13, recomputes member content where applicable and
    every enclosing manifest, installation, validation, import, and aggregate digest, then
    re-signs every enclosing receipt with the independently pinned test keys. The case must
    reach only its named semantic refusal.

  - `source-floor-v1/matrix-meta-negative-case-ids.txt` contains exactly these four
    newline-terminated ids in order:
    `source-floor-meta/production-role-removed`,
    `source-floor-meta/fixture-role-removed`,
    `source-floor-meta/visitor-hook-removed`, and
    `source-floor-meta/enclosing-receipt-not-recomputed`. Each must fail for its named reason
    through `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`; a shared shrunken role
    count, early structural failure, stale enclosing receipt accepted as the intended
    refusal, or unvisited case cannot pass.

  - `source-floor-v1/issuer-proof-negative-case-ids.txt` contains exactly five
    newline-terminated ids in order:
    `source-floor/copied-issuer/manifest`,
    `source-floor/copied-issuer/installation`,
    `source-floor/copied-issuer/validation`,
    `source-floor/copied-issuer/import`, and
    `source-floor/copied-issuer/all`. Each case copies the expected authority and key
    digests, signs the correct-domain canonical object with an unpinned valid key, recomputes
    every enclosing hash, and keeps every unaffected proof valid. The four single cases fail
    only at their named pinned-key verification; `all` reports all four issuer failures.

  - `source-floor-v1/issuer-authentication-negative-case-ids.txt` contains exactly the 20
    ordered ids in `data-model.md`: four missing-proof, four wrong-key, four cross-domain,
    three rebound-binding, direct-decoded-chain, and four private-capability
    serialization/clone cases. A separately authored literal 20-id constant equals the file.
    Canonical runtime cases preserve every unaffected proof and enclosing digest; type cases
    fail through compile-fail/API-surface seals. No decoded DTO or copied digest tuple can
    construct `AuthenticatedSourceFloorIssuerProvenance` or
    `ValidatedSourceGenerationCompatibilityFloor`.

  - `source-floor-v1/hash-vector-negative-case-ids.txt` contains exactly the 21 ordered ids
    in `data-model.md` for digest/signature id set and order, domain/terminator/payload,
    frame width/endian/length, preimage/digest, verifier key, signing preimage, and
    signature. A separately authored literal 21-id constant equals the file before any
    vector runs. Production/vector consumers and expected ids are mutually read-independent;
    every poison reaches only its named byte-oracle check.

  - `source-floor-v1/receipt-negative-case-ids.txt` contains exactly the 32 ordered ids in
    `data-model.md`. A separately authored literal 32-id constant must equal the file before
    any case runs. The file and constant are read-independent from the floor decoder,
    transition machine, schemas, hash-vector consumer, poison generator, 13-role matrix, and
    copied-issuer matrix. Every fixture recomputes unaffected enclosing digests and
    signatures and reaches only its named canonical, framing, transition, authority, or C/Q
    binding check.

  - `sc002-activation-receipt-negative-case-ids.txt` and
    `sc002-census-negative-case-ids.txt` contain exactly the ordered 61 and 45 ids in
    `data-model.md`. Separately authored literal arrays must equal both files. Receipt and
    census encoders, poison builders, and production validators read neither expectation.
    Every malformed census case, including resolution-leaf inclusion, invalid complete-body
    overflow, copied cross-incident failure commitment, and raw `01ff` authority, reaches
    only its named check.

  - `sc002-domain-hash-vectors-v1.json` is the shared closed oracle for exactly nineteen
    typed digest ids and one disposition-signature id in `data-model.md`. Receipt locator,
    incident-id, retired-census, primary-evidence, residue, and disposition tests
    independently reconstruct semantic preimages and compare against this same file. No
    second raw receipt hash or duplicated expected digest is accepted.

  - `sc002-sidecar-lock-case-ids.txt` contains the literal complete case set for actor pairs
    formed from writers `importer`, `cleanup`, `incident-recover`, `disposition-request`,
    `incident-apply`, and `successor-admit`, plus `retention-guard`: every writer/cleanup and cleanup/writer pair,
    distinct cleanup/cleanup, and every writer/retention and retention/writer pair; same and
    different inputs wherever both actors admit them; both first-owner orders; and every
    reachable latch `temp-created`, `temp-file-synced`,
    `quarantine-renamed`, `retirement-renamed`, `incident-preimage-published`,
    `incident-anchor-published`,
    `incident-metadata-published`,
    `incident-payload-renamed`, `incident-payload-synced`, `incident-residue-staged`,
    `incident-residue-finalized`, `incident-status-published`,
    `resolution-evidence-published`, `incident-resolution-published`,
    `successor-freeze-published`, `disposition-request-published`, and
    `successor-status-published`. Each expected id is
    `sc002-lock/<first>-then-<second>/<same|different>/<latch>`. The fixture explicitly omits
    unreachable combinations and pins that omission list. Tests compare exact set equality,
    not only a count. Every nonblocking live-owner loser must observe zero namespace opens,
    zero namespace mutations, and `critical_section_max = 1`; after release exactly one retry
    may advance after opening fresh fds and recensoring under the lock. This includes
    cleanup/cleanup overlap against the same live owner, two different candidate leaves, and
    every incident/successor live owner; no cleanup may retain a pre-lock namespace fd or
    observation, and no test can construct/serialize/clone/rebuild `SidecarCleanupOwner`.

  **Serialized broker-wire boundary:** T589 does not edit `broker_wire.rs`, the broker
  dispatcher contract, StoreSync DTOs, broker protocol metadata, or their generated
  artifacts. T592 owns the Zone-audit drain op, the host-generation transition op, both
  StoreSync DTOs, every producer and consumer, and the single coordinated broker protocol
  transition. This keeps no intermediate commit with a new DTO and stale callers.
  T589's `command.rs` ownership covers the
  `wave validate-import --sc002-receipt PATH` parser plus the exact
  `sc002-incident-inspect`, `sc002-incident-recover`, `sc002-disposition-request`,
  `sc002-incident-apply`, and
  `sc002-successor-admit`
  parser/synopsis/catalogue/dispatch/help surface from `data-model.md`. The receipt option
  must appear exactly where the importer accepts it and remain absent from unrelated
  subcommands. Incident commands use stable lowercase 64-hex incident/disposition IDs and
  closed exits `0|2|3|4`; exact replay of an already durable transition returns `0` without
  a write while a changed binding returns `4`. Inspect exits `0` for every validated
  `recovery-resumable`, `recovery-irreconcilable`, or terminal primary/resolution incident
  state. A valid-state exit `4` emits the same stable status projection; invalid CLI syntax
  or a noncanonical caller id is `2`, a missing stable id is `3`, and stored anchor/metadata/status
  corruption is an inspectable irreconcilable state. Human output is
  the exact ordered thirteen-line projection in `data-model.md`, including bounded IDs as
  data, nullable IDs rendered as `none`, the closed cause and remediation values, and only
  the static next-command noun
  `sc002-incident-recover`, `sc002-disposition-request`, `sc002-incident-apply`,
  `sc002-successor-admit`, or `none`. It contains no flags,
  interpolated argv, path, executable, shell fragment, or free-form guidance. The
  version-1 JSON is the distinct 17-field `Sc002IncidentCliStatusV1` projection with
  immutable `incidentKind`, required `cause`, exact residue census, nullable typed
  resolution-evidence kind/digest, and a required final remediation enum derived from durable
  status plus the locked disposition census and no `nextCommand`/guidance field; persisted
  19-field `Sc002IncidentStatusV1` includes the complete structured incident preimage,
  its immutable locator, and exact incident-id preimage hex and has no
  remediation field. The original
  cleanup refusal and every later refusal expose the same stable incident id, cause, and
  remediation as bounded data. Inspect projects every validated
  metadata/source/payload/residue/status/frozen-primary-evidence/resolution state. Recover accepts no alternate path, identity,
  disposition, successor, or deletion selector and resumes only an exact recoverable
  metadata-bound `recovery-resumable` protocol under the shared lock; it is an idempotent
  exit-0 no-write after its durable target and exits `4` without mutation if a fresh census
  is irreconcilable. `sc002-disposition-request` first derives and durably freezes one clean
  successor triplet, publishes the canonical unsigned authority request, and cannot mint or
  self-sign a disposition. Apply consumes the authenticated disposition and the same
  successor snapshot from
  `recovery-irreconcilable`, moves every representable current leaf through the closed
  temporary, cleanup-quarantine, payload, retired-source, or retired-existing-destination
  residue slot and publishes `mismatch-retained`, or binds the complete frozen
  primary-evidence census or identity-bearing bounded-failure commitment and publishes
  separate resolution `disposition-validated` when names, anchor/metadata, primary status, or the
  census itself are unusable or unstable. The   frozen scope recursively enumerates every descendant and excludes every resolution,
  resolution-evidence, disposition, request, freeze, receipt, and successor leaf. A raw `01ff` sentinel,
  copied failure commitment, or changed scope cannot authorize apply or successor admission.
  It never fabricates a residue or edits a conflicting primary branch. Every prescribed
  command either advances to its advertised terminal or returns the same stable projection
  after a concurrent transition. T589
  additionally owns `tests/golden/delivery/sc002-incident-id-v1.json`. It defines exactly
  one independently recomputed vector for each closed `Sc002IncidentKindV1`:
  `retirement-id-collision`, `retirement-census-exhausted`,
  `retirement-census-invalid`, and `identity-ambiguity`. The accepted external Version 2
  delivery contract must pin this enum, each kind-specific domain-separated preimage, the
  exact structured `Sc002IncidentPreimageV1`, preimage-complete
  `Sc002IncidentAnchorV1`, path-complete
  `Sc002IncidentMetadataV1`, payload/residue plus append-only
  status, resolution-evidence, and resolution paths, every temporary write/file
  sync/no-replace/final reopen/payload-file/parent and every-ancestor sync, idempotent resumable recovery and
  irreconcilable resolution protocols, exact durable/resolution/CLI schemas and
  cause/remediation table, all primary and resolution branches, the retired and frozen
  primary-evidence recursive census byte grammars and vectors, pre-signing successor
  freeze/request/apply/admit triplet binding, the shared SC-002 typed domain-hash
  oracle, the complete receipt/census negative registries, and all four incident vectors before
  T589 dispatch.
  `sc002-incident-apply` consumes the
  exact `Sc002IncidentDispositionV1` record from `data-model.md`: at most 32,768 canonical
  JSON bytes with the exact 22-field order including `incidentKind`,
  `preimageLocator`, complete `incidentPreimage`, `incidentIdPreimageHex`, and nullable
  `resolutionEvidenceKind` plus `resolutionEvidenceSha256`, exact Version 2 delivery-contract digest,
  successor-freeze and disposition-request digests,
  pinned authority/key digests, and final Ed25519 signature over the domain-separated
  length-framed unsigned object. Once-open current-effective-uid `0600` single-link input is
  hashed before decode. One private nonconstructible, nonserializable, noncloneable
  `ValidatedSc002IncidentDisposition` is consumed by value; caller keys, direct DTO decode,
  or copied authority digests are ineligible. Apply rederives the successor triplet from the
  same snapshot used by the durable freeze/request and refuses post-signing substitution,
  then durably publishes the exact bytes at the
  disposition-id content address before advancing status. Successor admission reopens and
  revalidates the same preimage, freeze, request, disposition, and snapshot. It requires a fresh distinct triplet,
  copies no receipt, incident, retired, residue, status, or disposition evidence, never
  unblocks the incident candidate, never unlinks an incident, and creates no binding request or reservation
  release. For `adr046w5` it admits only T220's nonbinding replacement/evidence path while
  retaining the consumed request byte-for-byte. Focused parser,
  incident-preimage/anchor/metadata/durable-status/resolution/CLI-status/freeze/request/
  disposition schemas, recursive census-byte and
  incident-kind goldens,
  signed canonical disposition golden,
  signature, human/JSON golden, exit, crash/replay,
  stale/not-found/conflict, wrong-contract/authority/key/domain, malformed/noncanonical,
  tamper/cross-incident/cross-triplet replay, no-unlink, no-evidence-copy, and
  successor-admission tests stay synchronized with `evidence.rs`; the existing
  `changelog.d/resource-api-production.md` fragment names all five commands, exits
  `0|2|3|4`, the authenticated disposition, and fresh-successor requirement. No
  later task may add either surface through an unowned help or dispatch path.
  T589's schema ownership additionally includes
  `docs/reference/schemas/delivery/sc002-incident-preimage-v1.schema.json`,
  `docs/reference/schemas/delivery/sc002-incident-anchor-v1.schema.json`,
  `docs/reference/schemas/delivery/sc002-incident-metadata-v1.schema.json`, and
  `docs/reference/schemas/delivery/sc002-incident-resolution-v1.schema.json`; its human/JSON
  golden ownership includes every incident kind and all five remediation rows. This is
  implementation of the already accepted external Version 2 contract, not ownership of that
  normative amendment.
  **Finding-scoped Version 2 prerequisite correction:** the older T589 summary is superseded
  by `data-model.md` for incident recovery. The accepted external amendment must pin the
  typed domain-separated, length-framed canonical SC-002 receipt content hash; complete
  structured incident preimage with every kind-specific component, immutable preimage path,
  anchor, metadata, and durable status including exact incident-id preimage and
  preimage/anchor/metadata/source/payload locators;
  distinct `recovery-resumable` and `recovery-irreconcilable` states; exact cause,
  remediation, exit, and command-convergence tables; payload-fd sync before `parked`;
  durable precreation and bottom-up sync of every changed retirement, incident, status,
  resolution-evidence, and resolution ancestor; closed retired source/destination residue
  slots; the complete and identity-bearing recursively enumerated bounded-failure
  `CanonicalIncidentPrimaryEvidenceCensusV1` forms with a frozen scope that excludes
  resolution leaves; pre-signing durable successor freeze plus canonical disposition
  authority request; the same signed successor triplet at apply and admit; the append-only
  irreconcilable resolution branch; and primary or resolution successor admission without evidence deletion. A
  partial anchor/metadata temporary, nonidentical final anchor/metadata, zero-name census, retired-source
  case, uniquely repairable status prefix, and conflicting status branch must each have one
  stable id, inspect exit `0`, exact next command, and reachable terminal disposition or
  successor. Until that normative amendment, approvals, regenerated manifests, Gate 0
  receipt, and ancestor binding land, T589 remains blocked and this planning text is not
  implementation authority.
  **Typed SC-002 evidence ownership:** in `packages/xtask/src/delivery/evidence.rs`, retain
  schema-v2 `EvidenceRecord` and its decoder byte-for-byte and define the separately versioned
  `Sc002ActivationReceiptV1` exactly as specified in `data-model.md`. A passing
  `validation = "operator-nix-activation-cleanup"` import requires the explicit
  `--sc002-receipt PATH` input and forbids caller-supplied `--locator`; a failed record or
  any other validation forbids that input. Open the source exactly once with
  `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, require a regular single-link file owned by the current
  effective uid with mode exactly `0600`, read at most 16,385 bytes, hash before decode, and
  derive the unchanged record's `locator` from the exact typed domain-separated,
  length-framed `Sc002ActivationReceiptContentSha256V1` over canonical receipt bytes as the
  candidate-relative content address
  `evidence-sidecars/sc002/sha256/<typed-digest>.json`. The digest is only the typed
  domain-separated, length-framed value whose shared vector id is
  `activation-receipt-content`; no raw SHA-256 locator definition exists. Validate the
  decoded receipt and actual outer
  binding before publication. Beneath the held candidate-directory fd, create and verify
  current-effective-uid `0700` namespace directories, create a current-effective-uid `0600`
  temporary leaf with `O_CREAT|O_EXCL|O_CLOEXEC|O_NOFOLLOW`, write the exact validated bytes,
  `fsync` the file, and publish with `renameat2(RENAME_NOREPLACE)`. Before publishing the
  `EvidenceRecord`, `fsync` every held ancestor directory fd bottom-up from `sha256` through
  `sc002`, `evidence-sidecars`, and the candidate directory. Before creating or recovering a
  temp, every importer and cleanup worker must acquire the same verified candidate-scoped
  exclusive OFD write lock and hold it through publication or cleanup, parent `fsync`, the
  applicable census, and `EvidenceRecord` publication or return. No cleanup path has a
  second lock or lock-free fallback. The fixed regular single-link current-effective-uid
  `0600` lock leaf is opened with `O_CLOEXEC`, revalidated as one stable device/inode, and
  never replaced, renamed, or unlinked. Successful cleanup acquisition yields the sole
  private nonserializable `SidecarCleanupOwner`; every namespace open, rename, census, and
  return requires that owner, and it has no public fields, constructor, accessors, serde,
  clone/copy/default, conversion, or fd reconstruction. A loser obtains no owner, and
  restart obtains a fresh owner only after reopening, validating, and acquiring the lock.
  The live importer owns that OFD lock, so a
  nonblocking loser receiving `EAGAIN` or `EACCES` returns
  `sc002-sidecar-owner-live` before any namespace inspection or mutation; restart cleanup
  may proceed only after it acquires the released lock. The same lock serializes all
  same-leaf and different-leaf cleanup against every live importer, cleanup,
  incident-recover, incident-apply, successor-admit, and retention owner before namespace
  access; a loser retains no pre-lock fd or observation and the sole retry
  recensors with fresh fds. Cleanup enumerates only the reserved temporary and quarantine namespaces through the
  held leaf-parent fd. It opens and verifies the regular single-link
  current-effective-uid `0600` temp, atomically moves that name with
  `renameat2(RENAME_NOREPLACE)` to a unique reserved quarantine name while holding the lock,
  reopens the quarantine leaf, and requires the same device/inode, owner, mode, link count,
  digest, and bytes as the pre-move fd. Never call `unlinkat` on a sidecar data leaf: Linux
  has no inode-qualified unlink, so check-then-unlink retains a name/inode race. A verified
  inode is not inferred from a checked name. Every temp-to-quarantine,
  quarantine-to-retired, and source-to-incident rename is the name-consuming operation and
  is followed by reopening the destination name and revalidating the actual moved inode. A
  replacement moved by the rename is quarantined as non-authorizing incident residue; it is
  never unlinked or counted as the pre-move inode. A verified orphan derives the
  domain-separated candidate/content/device/inode-bound
  `Sc002RetirementIdV1` and is moved no-replace into durable
  `evidence-sidecars/sc002/retired/sha256/<content-digest>/<retirement-id>.bin`, reopened,
  revalidated, and file-and-directory synced after every missing destination ancestor is
  durably precreated and every changed source/destination ancestor is synced bottom-up
  through the candidate directory. Two identical orphan leaves with distinct
  inodes must retire under distinct ids. A destination `EEXIST` is never success while the
  source still exists: preserve the existing destination and move the source through the
  `retirement-id-collision` incident transition, deriving its id from separately observed
  source and existing-destination identity digests. The retired census permits at most 64 exact regular single-link
  current-effective-uid `0600` leaves and at most 1,048,576 bytes, with each leaf at most
  16,384 bytes; overflow transitions the source to `retirement-census-exhausted` with the
  source identity, valid pre-add census digest, and current/prospective counts, while an
  unknown entry or path/identity/digest mismatch transitions it to
  `retirement-census-invalid` with the source identity and bounded observed-census digest.
  Neither census kind fabricates a second identity tuple. Each incident first persists the
  exact immutable structured `Sc002IncidentPreimageV1`, including every applicable
  collision, census/count, or ambiguity component, then the exact
  immutable `Sc002IncidentAnchorV1`, then the exact
  immutable `Sc002IncidentMetadataV1`, one metadata-bound payload under
  `incidents/payload/sha256/<incident-id>.bin`, and a contiguous append-only status prefix
  under `incidents/status/sha256/<incident-id>/`. Metadata contains every kind-specific
  preimage component, the exact preimage bytes, metadata/source/payload locators, and closed
  source slot, and nulls every inapplicable field: the parked
  candidate/content/snapshot triplet, source content digest and locator, payload locator and
  identity digest, plus collision retirement/source/destination identities, census digest
  and current/prospective counts, or ambiguity stage/before/after identities as selected by
  the closed kind. The immutable `incidentKind` matches its recomputed domain-separated id.
  Publish the preimage, anchor, and then metadata create-exclusively and sync each leaf plus every
  changed ancestor through the candidate directory before the metadata-bound payload rename;
  sync both old and new
  parents and every changed ancestor after the rename; reopen, revalidate, and `fsync` the
  payload fd; then create-exclusively
  publish and leaf/ancestor-sync each append-only status. Every durable primary status
  repeats the complete structured preimage, its immutable locator, and
  `incidentIdPreimageHex`; the CLI projection omits all three. Existing metadata, payload, or
  status is idempotent only after an fd-relative reopen proves exact bytes and binding.
  Recovery classifies every nonterminal census as exactly `recovery-resumable` or
  `recovery-irreconcilable`. Recover resumes only the uniquely named contiguous crash prefix
  and the next unsynced payload/ancestor step. Missing, duplicate, skipped, cross-kind,
  nonidentical, source-plus-payload, anchor/metadata conflict, or conflicting-status state remains
  blocked without unlink and is inspectable under the stable id. Authenticated apply may
  retain representable evidence through the closed five-slot residue protocol or bind the
  complete recursively enumerated frozen primary-evidence census or an identity-bearing
  bounded-failure commitment,
  publish and sync those exact canonical bytes outside that scope, and append the separate
  resolution `disposition-validated` record; it never edits primary bytes or requires
  nonempty residue. Resolution persists the same complete structured preimage, locator,
  preimage hex, evidence kind, typed digest,
  derived locator, and nullable bounded-failure cause. The scope recursively records every
  directory and regular-file descendant and excludes resolution, disposition-request,
  disposition, freeze, and successor leaves; its bounded-failure form binds the fixed root,
  canonical failing-path digest, saturated counts, and before/after recursive identities.
  Raw `01ff` is detection-only poison.
  Valid retired residue is immutable and non-authorizing and does not block retry or close.
  Implement the sole private `CandidateRetentionOwner` in
  `packages/xtask/src/delivery/storage.rs` as a zero-mutation whole-scope retention guard.
  With the same lock held, its full census proves terminal `merged|abandoned-unmerged`
  delivery state, terminal request/reservation/panel/seal/eligibility/merge transitions,
  every incident absent or `successor-admitted`, every retained external reference
  resolvable, empty ephemeral namespaces, exact bounded durable namespaces, and an immutable
  canonical candidate root with all request, panel-record, evidence-record, receipt, seal,
  eligibility, merge, incident preimage/anchor/metadata/payload/residue/status,
  resolution-evidence/resolution, successor-freeze, disposition-request, disposition, and
  successor-admission history. Its recursive census requires every durable record to repeat
  the same complete structured preimage and all kind-specific components. Verified orphans remain in
  the separately owned bounded `evidence-sidecars/sc002/retired` subtree. The owner never
  renames, tombstones, or deletes the candidate root and never automatically unlinks any
  candidate descendant. Any failed predicate performs zero mutation. The still-held lock
  then guards an exact empty census of both ephemeral reserved namespaces and the bounded
  durable census.

  An identity mismatch has one fail-closed terminal path: never restore the suspect to the
  temporary namespace or treat a mismatched retired name as verified. Publish and sync the
  preimage-complete immutable metadata, atomically move the metadata-bound currently named
  suspect with `renameat2(RENAME_NOREPLACE)` into durable candidate-relative
  `evidence-sidecars/sc002/incidents/payload/sha256/<incident-id>.bin`, outside both
  ephemeral namespaces, `fsync` the old and payload parents plus every changed ancestor,
  reopen the payload, prove the moved identity/digest/bytes, and `fsync` that fd before
  append-only publishing and syncing `parked` status. A replacement before rename, mismatch
  after rename, `ENOENT`, or nonidentical `EEXIST` is exactly
  `recovery-resumable` when one metadata-bound continuation remains, otherwise
  `recovery-irreconcilable`: preserve every named leaf and publish no parked status.
  Irreconcilable zero-residue, anchor/metadata-conflict, status-conflict, invalid-census, and
  unstable-census states advance only through the disposition-bound complete census or
  bounded-failure commitment and the append-only resolution branch.
  Neither variant is an alternate terminal state. Both nonterminal variants and terminal parked incidents block
  `EvidenceRecord` publication and every close stage and are never removed by automated
  cleanup. Ordinary winners and terminal parked incidents leave the two ephemeral namespaces
  empty; a nonterminal race may retain ephemeral residue but never claims terminal
  success. A retry after a
  crash may reuse an identical durable leaf only after reopening and revalidating its full
  type/owner/mode/link-count/device/inode/digest/bytes/decode/binding identity; it never
  replaces the leaf, and a different existing leaf refuses. On every reopen resolve it only
  beneath the held candidate-directory fd with
  `openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV)`, open
  once with `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, require a regular single-link leaf owned by the
  current effective uid with mode exactly `0600` and stable device/inode, hash before decode,
  and decode from that same
  fd. Repeat this resolution, hash, and identity check at import, durable reopen,
  panel-request/panel-attest, seal, and merge-eligibility. A failed operator record remains
  importable with no receipt and is ineligible for the closed evidence profile and every close
  stage. A failed record that references a positive receipt is malformed. Reject unknown fields
  or enum values, version or kind mismatch, encoded size above 16,384 bytes, locator or
  sidecar-content-digest mismatch, any sample
  census other than exactly one each for `Volume/acceptance-state`,
  `Network/acceptance-net`, and `Device/acceptance-tpm`, duplicate/unrelated identities,
  effect/Ready/selected-stop/progress identity mixing, unchecked or misordered monotonic
  ticks, a selected stop other than the later effect/Ready observation, zero or more than 32
  progress events per sample, a progress tick outside `(start, stop]`, elapsed mismatch or
  overflow, any elapsed value above 2,000,000,000 ns, and stale outer
  `candidate_id`/`content_id`/`snapshot_sha256` binding. The immutable snapshot resolves the
  commit/tree without changing `EvidenceRecord`. Give `EvidenceRecord`, receipts, and
  validation errors fixed redacted `Debug`;
  raw ticks, resource identities, host data, paths, commands, argv, and free-form text must
  not appear. Use one validator unchanged for evidence import, durable reopen,
  panel-request/panel-attest, seal, and merge-eligibility. Table-driven tests at every stage
  cover exact-size success, 16,385-byte refusal, a passed record with a missing or duplicate
  receipt, a receipt on a failed or wrong-validation record, a failed record with no receipt
  that imports but cannot close, retained schema-v2 fixture decoding, malformed or unknown
  version/kind/field/enum, locator or sidecar-content-digest mismatch,
  absent explicit input for a passing operator record, explicit input on a failed or other
  validation record, caller-supplied locator, wrong source or destination owner/mode,
  absolute/traversal/URL/symlink/
  hard-link locator refusal, replacement before hash, between hash and decode, and before
  every later reopen,
  crash before and after source hash/decode, OFD-lock acquisition, temp write, file sync,
  no-replace publication, each ancestor-directory sync, quarantine move/reopen, verified
  retirement move/reopen, retirement `EEXIST`, retired-census validation, incident move and
  preimage/anchor/metadata leaf sync, every preimage/anchor/metadata ancestor sync, old-parent and payload-parent sync,
  payload reopen, payload-file sync, parked-status leaf sync, every status and resolution
  ancestor sync, successor-freeze/disposition-request/disposition
  file/status/resolution publication, whole-scope retention guard,
  candidate-root/permanent-history preservation, cleanup-parent sync, ephemeral-residue census, and record
  publication;
  the exact every-writer/cleanup, cleanup/every-writer, cleanup/cleanup, every-writer/
  retention-guard, and retention-guard/every-writer actor-pair matrix for writers `importer`,
  `cleanup`, `incident-recover`, `disposition-request`, `incident-apply`, and
  `successor-admit`, with
  same/different inputs where applicable,
  each using independent opens of the one stable lock inode and both owner orderings at
  `temp-created`, `temp-file-synced`, `quarantine-renamed`, `retirement-renamed`,
  `incident-preimage-published`, `incident-anchor-published`,
  `incident-metadata-published`, `incident-payload-renamed`,
  `incident-residue-staged`, `incident-residue-finalized`, and
  `incident-status-published`, plus every reachable payload-sync and
  `incident-resolution-published`, `successor-freeze-published`,
  `disposition-request-published`, and `successor-status-published` latches; every
  nonblocking loser has zero namespace opens, zero
  namespace mutations, and `critical_section_max = 1`, while a blocking restart enters only
  after release and exactly one retry opens fresh fds, recensors, and linearizes after the
  winner; replacement before
  quarantine move, before reopen, and on both sides of every retirement/incident rename and
  reopen; two same-byte distinct-inode orphans produce two retirement ids;
  forced id collision, 65th leaf, 1,048,577th byte, malformed census, recursive descendant
  insertion/content mutation, wrong failure-path digest, unauthorized cleanup-owner
  construction/serialization/clone/fd reconstruction, unauthorized retention,
  candidate-root removal, permanent-history mutation, and failed whole-scope retention
  predicates preserve data and refuse; synchronized same-input
  retry, different-byte or wrong-binding races, bounded completion without deadlock, exact
  final census, empty ephemeral namespaces for ordinary terminals, no sidecar-data unlink,
  exact recomputation of all four records in
  `tests/golden/delivery/sc002-incident-id-v1.json`; independent encoding of its
  normal-empty, normal-sorted-mixed, and exact `01ff` over-bound census vectors; rejection of
  bad version/body/entry/observation/failure tags, framing, unsigned-byte ordering,
  unavailable sentinels, and partial over-bound prefixes; independent canonical
  complete zero-residue and mixed recursively enumerated primary-evidence census vectors plus
  identity-bearing bounded-failure vectors; exact structured incident-preimage,
  anchor/metadata, durable-status, resolution, freeze, request, disposition, and CLI-status
  schema separation plus every closed cause, both
  nonterminal state variants, all five
  deterministic remediation rows, and the exact thirteen-line human projection;
  preimage/anchor/metadata/payload-or-residue/contiguous-status-or-resolution/id kind
  agreement with every kind-specific component repeated byte-identically; idempotent recovery from
  every allowed directory-create/temporary-write/file-sync/no-replace/final-reopen/
  parent-sync/ancestor-sync/metadata/source/payload/payload-file-sync/residue/status/resolution crash prefix; refusal with
  all names preserved for
  source-plus-payload, neither source nor payload, status-without-payload, skipped status,
  nonidentical `EEXIST`, and post-rename identity mismatch; exact resumable versus
  irreconcilable classification; terminal resolution for zero names, retired-source
  locators, malformed final metadata, and branch-conflicting status; and durable `parked`,
  authenticated no-unlink `mismatch-retained`, or frozen-primary-evidence-bound resolution
  preservation plus
  publication and close denial for every
  identity-ambiguous, collision, exhausted-census, or invalid-census terminal,
  missing/duplicate/mixed/unrelated samples, effect/Ready identity disagreement, misordering
  and arithmetic overflow, stale binding, progress-free or overlong progress, and over-budget
  samples.
  **Source-floor contract consumption:** the separately accepted external
  `ADR-046-validation-and-delivery` Version 2 amendment, not T589 and not the compatibility
  producer, owns `SourceGenerationIdentityV1`, canonical JSON order/int/text/unknown policy,
  every length-framed digest and signature domain, strict
  `docs/reference/schemas/delivery/source-floor-v1/` schema, and every checked-in
  `tests/golden/delivery/source-floor-v1/` vector from `data-model.md`, including exact
  `hash-vectors-v1.json` with 15 digest and four signature entries. The separately named
  producer/installer and typed import/validation authority implement and install that
  contract. The validator first returns private nonserializable
  `AuthenticatedSourceFloorIssuerProvenance` after four pinned-key checks and consumes it by
  value into private `ValidatedSourceGenerationCompatibilityFloor`; T589 consumes only the
  latter and
  adds the local poison/meta-tests against the already accepted schemas and vectors.
  The separately authored literal 13-row test constant, production role registry, exact
  `role-artifact-matrix.tsv`, poison generator, and expected-id set are mutually
  read-independent and must agree; no expected cardinality reads another member's count.
  Poison case ids are
  exactly `source-floor/<class>/<role>` for the literal seven classes and literal 13
  canonical roles, exactly once each. The independent literal expected set contains 91 ids
  and may not call the generator. Each one-for-one substitution keeps
  array and declared cardinality 13, uses the exact class mutation in `data-model.md`, and
  recomputes member digest where applicable, manifest/proof/hash, census,
  installation/proof/hash, validation/proof/hash, validated-floor hash,
  exact-C/Q import/proof/hash, and aggregate identity in that order. Every test-only
  signature is valid and every canonical/schema check passes before the intended semantic
  refusal. A stale enclosing hash, bad signature, wrong cardinality, duplicate/missing case
  id, unvisited role, or early structural refusal fails the matrix itself. The separate five
  copied-issuer expected ids independently attack manifest, installation, validation,
  import, and all proofs; each uses copied expected digests plus an unpinned valid signing
  key and reaches only the intended pinned-key failure after all enclosing checks.
  The exact 20 issuer-authentication/capability and 21 hash-vector expected ids in
  `data-model.md` are separately literal, mutually read-independent from production and
  poison builders, and every case reaches only its named check. Direct decode, serialization,
  clone, and copied digest tuples cannot produce either private result.
  All 39 missing/stale-digest/cross-disposition cases must visit the production validator
  with cardinality 13 and fully recomputed enclosing receipts. The exact four
  `matrix-meta-negative-case-ids.txt` poisons prove production, fixture, visitor, and
  enclosing-receipt shrinkage cannot false-green. This matrix and its meta-poisons are bound
  to `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`; no other discovered runner or
  optional test counts.
  **Additional T589 strict-binding ownership:** `packages/xtask/src/delivery/{history_proof.rs,storage.rs}`
  joins T589's existing delivery file set. Add the wave-scoped ADR046 strict profile
  described under "Wave gate tasks". A canonical binding-request reservation record, reached
  through held directory fds rather than a candidate pathname, atomically admits the wave's
  one final candidate while pinning exact commit, tree, candidate, request digest, and round
  address. Publish it with create-exclusive temporary state, file `fsync`,
  `renameat2(RENAME_NOREPLACE)`, and wave-directory `fsync`; no overwrite or check-then-create
  path is permitted. Reservation, terminal-disposition, directory-owner, and transition
  objects and their errors use fixed redacted `Debug`. Operator-visible
  diagnostics are identifier-free typed errors with closed remediation actions; they never
  render program, wave, candidate, commit, tree, request digest, round address, path, or argv.
  Canary tests place distinct values in every field and prove none reaches `Debug`, error,
  log, or serialized diagnostic text.

  The wave can never reserve or request twice. Ordering is durable and replayable: the
  reservation is durable before panel-request publication, and unanimous or nonunanimous
  disposition retains the reservation, request, and records permanently for that
  program/wave. There is no release-for-retry transition and no successor admission path.
  Crash recovery replays each transition idempotently and exposes zero reservations only at
  crash points where publication was not durable, otherwise exactly one. Same-candidate
  retry, alternate-candidate request, or a post-request commit/history/evidence move is
  refused at panel, seal, and eligibility. The existing generic history-only-rebase proof
  remains usable only before the wave reservation or outside this strict profile. Nonbinding
  `/d2b-panel-round plan` phase rounds do not create the reservation and may iterate before
  the final candidate is selected. Seed the retained Wave 5 consumed request and its complete
  delivery directory as a fixture, snapshot every byte, run both a unanimous phase round and
  a finding-plus-rerun phase sequence, and require the delivery state to remain byte-identical.
  Neither sequence may create a binding reservation or request, replace or relabel the
  retained `panel-request.json`, alter its disposition, or mutate candidate history.

  Recovery owns orphan cleanup through the already held wave directory fd. It enumerates only
  the reserved create-exclusive temporary namespace, rejects symlinks and unexpected
  type/owner/mode/link-count/inode changes, removes a verified orphan with `unlinkat`, and
  `fsync`s the wave directory before retry. No joined path, candidate-relative cleanup, broad
  sweep, or cleanup outside that namespace is permitted.

  Table-driven and injected-filesystem tests in the owned modules must issue synchronized
  first requests for the same program/wave from different candidate directories and observe
  exactly one success, one durable canonical reservation, and typed refusals for every loser.
  The point-specific reservation oracle is: every crash before no-replace publication
  recovers zero committed reservations; a crash after no-replace publication but before the
  wave-directory `fsync` may recover zero or one; after that directory `fsync`, recovery must
  expose exactly one and every same-candidate or alternate-candidate request is refused.
  Every recovery/retry case leaves exactly one canonical reservation or none as allowed by
  that oracle, and zero temporary-file residue after durable cleanup.

  Inject crashes around temporary-file sync, no-replace publication, directory sync,
  panel-request publication, and terminal unanimous or nonunanimous disposition. At every
  restart prove idempotent transition ordering, zero or one reservation as permitted by the
  publication oracle, retained request/disposition records, and no retry, release, successor
  admission through binding state, or duplicate request. The separately authenticated SC-002
  incident flow may admit only a nonbinding fresh evidence candidate under the limits above;
  it cannot reach panel/request state. Also reject same-candidate second request, alternate
  candidate request, post-request byte-identical rebase, and evidence refresh at panel, seal,
  and eligibility.
  T589 consumes `adr046-candidate-recovery-prerequisite/v1` only after confirming that the
  accepted external generation is present on T589's own actual base. T008 remains the
  separate historical W2 entry attestation; T589 does not check or retroactively satisfy it.
  The external ADR, index, delivery tooling, `AGENTS.md`, and contributor guidance remain
  outside this feature-edit batch and T589 may not reinterpret or weaken them. This planning
  edit does not alter the validation/delivery specification. Its separate Version 2
  amendment, approval, generated manifests, and Gate 0 receipt must land before T589; T589's
  future implementation scope only consumes that exact accepted contract, with T220's later
  paired-artifact check.
  T589's earlier branch-cut done clause applies to T590, T591, and T594; T593 cuts from
  completed T592 rather than directly from T589 because it consumes T592's sealed broker
  operation, not because it owns a lockfile.

- [ ] T590 [P] [US1] **Install and recover the single-owner Zone resource policy without a bootstrap cycle.** Depends on T589. Owned files: `packages/d2b-resource-api/src/authz.rs`, `packages/d2b-core-controller/src/rbac.rs`, and new focused tests under `packages/d2b-resource-api/tests/production_policy.rs`. `ZoneResourceRuntime` owns each `PolicyBootstrapRead` and requests installation, but `d2b-resource-api` alone parses and compiles policy into the immutable `PolicySet` interpreted by `NativeAuthorizer`. For initial install and restart, consume the one-shot capability to read only this Zone's policy-input envelopes at the exact durable nonzero `policy_revision`; it has no public subject, general read/mutation operation, clone, copy, default, public construction, conversion, trait-based mint, reconstruction, or reuse path. A failed installation attempt consumes the capability. After installation, perform every normal policy read/update through an authenticated Resource API session. Authorize T589's `InspectOperation` only for the registrar-derived subject and exact Zone bound to the original mutation; a wrong subject, wrong Zone, or replay-binding mismatch returns the same non-observing result as an unknown operation and never exposes the original binding. On revision advance, compile the exact committed revision before atomic replacement, invalidate cached allows, and report ready only when installed revision and Zone UID equal live durable metadata. Refuse revision zero, stale/missing/cross-Zone/invalid policy, a caller claim, reusable bootstrap access, and any fallback to a constant or partial set. **Done when** focused tests cover first install, authenticated revision advance, restart recovery of the advanced revision, failed-attempt consumption, capability non-reuse, and same-subject/Zone operation inspection with wrong-subject/Zone indistinguishability; external compile-fail fixtures prove construction, field access, `Default`, `Clone`/`Copy`, `From`/`TryFrom`, conversion, and capability reconstruction are impossible; T589's trait-solver, roots.json, golden, and API-surface seals remain current; `make test-rust` runs the Rust and compile-fail/doctest companions and `make test-rust-api-surface` passes; and every failure leaves only the affected Zone unpublished, degraded, and denied.
- [ ] T591 [P] [US1] **Restore the D106 store boundary and make it exhaustive.** Depends on T589. Owned files: `packages/d2b-resource-store-redb/src/transaction.rs`, `packages/d2b-resource-store/tests/d106_policy.rs`, and `packages/d2b-contract-tests/tests/policy_resource_mutation_seal.rs`. Preserve T589's frozen policy-neutral transactional audit hook. Remove redb deserialization or ownership of `RoleSpec`, `RoleBindingSpec`, `PolicySet`, and all other RBAC DTOs. Move policy-shape interpretation to the Resource API policy owner while retaining policy-neutral canonical-envelope, installed-schema, structural, atomicity, revision, and seal checks in the store. Expand the guard from three hand-picked source files to the full store/redb crate source and dependency graph. The scan MUST enumerate a nonempty source set independently for each store crate and a nonempty resolved dependency set; an empty, missing, or filtered-away input is a failure. Add a hermetic poison fixture that injects both a forbidden RBAC DTO use and a forbidden Resource API dependency and proves the existing test-policy/fixture-contract path rejects them. **Done when** the policy test proves neither store crate depends on the Resource API or contains/imports/deserializes an RBAC policy DTO, the poison negative fails for the intended D106 reasons through existing `make test-policy` and fixture-contract gates, the native evaluator remains the only allow issuer, and authorized Role/RoleBinding mutations still pass through the sealed generic envelope path.
- [ ] T592 [US1] **Complete durable store identity recovery and the authoritative mutation-audit journal/export drain.** Depends on T591, not directly on T589. After T591 finishes its policy-neutral rewrite, T592 is the serialized owner of `packages/d2b-resource-store-redb/src/transaction.rs`. Other owned files: `packages/d2b-resource-store-redb/src/{lib.rs,actor.rs,audit.rs,migration.rs,backup.rs,tests.rs}`, `packages/d2b-audit/src/{lib.rs,sink.rs,record_types.rs,segment.rs,export.rs}`, `nixos-modules/{resources.nix,resources-bundle.nix}`, `tests/unit/nix/cases/resources-bundle-telemetry.nix`, and accepted normative specifications `docs/specs/ADR-046-telemetry-audit-and-support.md` and `docs/specs/ADR-046-resource-store-redb.md`. Separate immutable store/Zone identity from mutable policy, active-configuration, and controller revisions so reopen reads the latter from durable metadata after any advance. Bump the redb physical `schema_version` and the resource-store specification from Version 1 to Version 2; add permanent `audit_journal` key-space/value-kind `0x0b`/`0x000b`, keyed by `(operation_digest[32], mutation_ordinal:u32)`, and separate mutable `audit_export_state` `0x0c`/`0x000c`, with a staged migration and backup/restore coverage. Bump the audit specification from Version 2 to Version 3. In the same redb transaction as every privileged mutation, create immutable authoritative journal rows; export completion is separate mutable state and cannot clear or rewrite an unexported authoritative row. Keep raw operation, correlation, authoritative subject, Zone, resource identifiers, and validated trace input only in bounded private replay state, and remove derived `Debug` from every sensitive transaction/replay struct in favor of fixed redacted implementations. Audit row constructors accept typed fixed 32-byte digests derived with distinct `d2b:audit:<class>:v1` domains; they do not accept raw identifiers. Exclude raw trace context from journal rows, segments, and exports; when correlation is required, store only a domain-separated fixed trace digest. Cap each encoded journal/export record at 65536 bytes and reject overflow before mutation. Persist a replay-binding digest over the registrar-derived subject, Zone, canonical semantic request, target, verb, exact expected revision, operation ID, and idempotency data. Implement the durable backend for T589's typed operation-status request: same-ID status or resume MUST match that digest before returning pending/final state; wrong subject/Zone/binding is indistinguishable from unknown and never reapplies. The root-owned audit directory is opened and retained as a validated directory fd. `SegmentWriter`, sink, and export use only fd-relative `openat2`/`openat`/`unlinkat`, `O_NOFOLLOW|O_CLOEXEC`, create-exclusive rotation names, regular-file/root-owner/mode/link-count and inode revalidation, and no joined path. Export completion advances only after segment file `fsync` and directory `fsync`; prune uses `unlinkat` followed by directory `fsync`. Configure `audit.retentionDays` as the common segment and exported-journal retention, default 30 and range 1 through 3650; an authoritative row is prune-eligible only after its export completion has been durable for that interval. Add `audit.maxRecordsPerSegment`, default 65536 and range 1 through 1000000, while retaining `audit.maxSegmentBytes`, default 67108864 and range 1048576 through 1073741824. Missing, invalid, or unenforceable limits and any segment/journal prune or sync failure produce typed degraded Zone health and block publication. T220 coordinates generated manifests, reference pages, contract tests, schemas, and changelog treatment for both version bumps. **Done when** physical-schema migration/rollback and crash tests at mutation/journal commit, append, file sync, directory sync, completion, rotation, journal prune, and segment prune prove no committed privileged mutation lacks an immutable row; multi-mutation replay yields one exported record per ordinal; same-ID apply count stays one; cross-subject/Zone/request/target/verb/revision/idempotency and restart mismatches deny; raw identifier and raw trace canaries are absent from journal/segments/exports/errors/logs/metrics/spans and every `Debug`; fixed-digest constructor and 65536-byte record-limit negatives pass; configured defaults/bounds and post-export-only journal retention are enforced; every prune/sync failure degrades health; advanced revisions reopen; and production construction cannot select `NoopMutationAudit` or `enabled() == false`.
  **Additional T592 sole broker ownership:** `packages/d2b-priv-broker/src/{audit.rs,live_handlers.rs,runtime.rs}`
  and `packages/d2b-priv-broker/src/ops/{audit_op.rs,mod.rs}`, with focused wire/dispatcher and
  filesystem race tests in those owned files. The unprivileged Zone runtime owns drain
  sequencing, but the root broker alone owns `SegmentWriter` and performs append, rotation,
  export, and prune through T589's typed op. No daemon path opens or mutates the root-owned
  audit directory, and no new unit or service is permitted.
  **Additional T592 resource-bundle ownership and versioning:** T592 also solely owns
  `nixos-modules/{options-zones.nix,zone-resources.nix,zone-resources-json.nix}`,
  `packages/d2b-contracts/src/{generation_bundle.rs,v3/resource_bundle.rs}`,
  `packages/d2b-contracts/tests/generation_bundle.rs`,
  `packages/d2b-contract-tests/tests/policy_resource_bundle.rs` and its new poison fixture
  under `packages/d2b-contract-tests/tests/fixtures/resource_bundle_duplicate/`,
  `packages/d2b-resource-compiler/src/main.rs`,
  `packages/d2b-resource-compiler/tests/cli.rs`,
  `packages/xtask/src/zone_schema.rs`,
  `docs/reference/{resource-bundle-digest.md,resource-compiler.md}`,
  generated output `docs/reference/schemas/v3/resource-bundle.json`, and
  `changelog.d/resource-bundle-audit-carrier.md`. Define the one operator carrier as typed
  compiler-only `d2b.zones.<zone>.audit`, emitted as the required top-level `audit` object in
  `resource-bundle.json`, never as a ResourceSpec or Zone self-resource field. It contains
  exactly `retentionDays`, `maxRecordsPerSegment`, and `maxSegmentBytes` with the bounds above.
  Move the artifact from `schemaVersion: 3` / `bundleVersion: 1` to the only accepted pair
  `schemaVersion: 4` / `bundleVersion: 2`; v4 `contentHash` covers canonical
  `{audit,resources}` so an audit-only change creates a new generation identity. Rust and Nix
  parsers reject 3/1, every mixed pair, future pairs 5/2, 4/3, and 5/3, missing/unknown audit
  fields, ResourceSpec placement, and consumer-side default synthesis, while 4/2 is the
  positive control. The compiler entry point must accept the typed `audit` input, emit it at
  4/2, and verify the same canonical `{audit,resources}` preimage as the authoritative Rust
  contract.

  Keep `packages/d2b-contracts/src/generation_bundle.rs::ZoneBundle`, which is already the
  active crate-root contract and production-controller input, as the single authoritative
  envelope. Retire the duplicate full-envelope/version/hash implementation in
  `v3/resource_bundle.rs`; that module may retain only non-duplicated resource-item helpers
  needed by the authoritative contract. No alias may preserve two independently validated
  bundle envelopes. Add a nonempty structural and public-API guard that rejects a second
  bundle-envelope type or type alias, a second schema/bundle version authority, a second
  content-hash implementation or entry point, and any re-export that makes such a duplicate
  reachable. The poison fixture independently injects each forbidden class and must fail for
  the intended single-owner reason through the existing `make test-policy` and
  `make test-fixture-contracts` lanes; zero discovered source/API entries is failure. The
  existing `gen-zone-schemas` command in `zone_schema.rs` generates the committed schema from
  that authoritative type, and the schema, compiler CLI tests, contract tests, and inline
  contract tests pin the
  required `audit`, 4/2 versions, audit-only hash change, exact canonical preimage, and all
  old/mixed/future refusals. Preserve byte-identical empty `Zone.spec` and emit no Zone
  resource. T595 owns final `bundle-zones.nix` and daemon wiring; T220 owns generated output
  reconciliation, the coordinated reference/schema/contract-test set, fragment fold, and
  changelog verification. `docs/reference/resource-compiler.md` carries the exact explicit
  target-closure bootstrap command plus the post-publication
  `d2b-host-generation-deploy --from-reference` command; raw `nixos-rebuild` is not the
  documented entrypoint, and no runtime DTO, wire response, error, or remediation argv
  carries either command.
  **Additional T592 atomic broker-wire ownership:** include
  `packages/d2b-contracts/src/{lib.rs,broker_wire.rs}`,
  `packages/d2b-core/src/{privileges.rs,privileges_w3.rs}`,
  `packages/d2b-priv-broker/src/{bootstrap.rs,lib.rs,main.rs,protocol.rs,runtime.rs,live_handlers.rs,sys.rs,ops/mod.rs,ops/audit_op.rs}`,
  `packages/d2bd/src/{lib.rs,daemon_version.rs}`,
  `nixos-modules/privileges-json.nix`,
  `packages/xtask/src/main.rs`,
  `packages/d2b-priv-broker/tests/broker_protocol_compatibility.rs`,
  new focused `packages/d2b-contracts/tests/broker_wire_v5.rs` and
  `packages/d2b-priv-broker/tests/{store_sync_v5.rs,host_generation_handoff_v5.rs,host_generation_coordinator_v5.rs}`,
  `packages/d2b-contract-tests/tests/{policy_broker_schema.rs,policy_broker_dispositions.rs,policy_peer_pidfd_quarantine.rs,policy_privileges_doc.rs,privileges_parity.rs}`,
  new poison fixtures under
  `packages/d2b-contract-tests/tests/fixtures/peer_pidfd_quarantine/`,
  `docs/reference/{daemon-api.md,privileges.md,broker-w2-dispositions.md,store-sync.md}`,
  generated `docs/reference/schemas/v2/{wire-protocol.json,privileges.json}`,
  `tests/golden/api-surface/{capability-api.txt,hidden-public-api.txt,public-api.txt,workspace-metadata.json}`,
  and new exact serialization/fingerprint snapshots
  `tests/golden/broker-wire/{host-generation-handoff-target-v5.json,store-sync-request-v5.json,store-sync-response-v5.json,protocol-v5-capabilities.json}`,
  the standalone `packages/d2b-priv-broker/Cargo.lock`, and the workspace
  `packages/Cargo.lock`. The active generators are `gen-schemas` in
  `packages/xtask/src/main.rs` for both wire-protocol and privileges schemas and
  `gen-daemon-api` for the request/response catalogue. The Rust privilege catalogue,
  Nix-rendered matrix, generated schema enums, daemon API table, privileges reference,
  target-v5 compatibility disposition, target-v5 capability/API fingerprint,
  target serialization snapshots, and
  both lockfiles are one output set. `make test-drift`, the privileges parity test, and the
  nonempty broker-operation/schema policy test must all fail on an omitted target adoption row or
  stale output. This is one serialized commit after T591; no other task may land an
  intermediate StoreSync or handoff definition, privilege row, protocol bump, or caller.
  The external disposition must already have landed the exact nonempty 13-member
  `SourceGenerationCompatibilityFloorV1` census. Those source members are read-only inputs to
  T592. T592 refuses every `missing`, `duplicate`, `extra`, `empty`, `stale-generation`,
  `stale-digest`, and `cross-disposition` source member, plus a legacy or otherwise
  mismatched source contract, and owns only the target-v5 adoption row and outputs.
  T592 is the sole owner of both `packages/Cargo.lock` and
  `packages/d2b-priv-broker/Cargo.lock`. T593 consumes the dependency graph frozen by T592
  and may not edit, regenerate, or assume transferred ownership of either lockfile.

  In that commit, define the typed Zone-audit drain request/response and migrate
  `StoreSyncRequest` and `StoreSyncResponse` together with every repository producer,
  broker dispatcher/handler, completion path, exporter, test fixture, and schema/snapshot
  consumer. Every identifier field becomes its sealed domain-specific fixed digest or opaque
  typed handle, and every DTO, dispatcher wrapper, and wire error uses fixed redacted
  `Debug`. Remove parallel raw or derived-`Debug` forms. Bump
  `d2b_contracts::PROTOCOL_VERSION` exactly from 4 to 5 in the same commit. Update the
  previous-version compatibility fixture from 3 to 4, pin the complete v4 StoreSync shape,
  prove v5 round trips through the real daemon and broker producers/consumers, prove the
  changed shape is never treated as v4, and regenerate the broker operation catalogue,
  schema, fingerprints, reference tables, and serialization snapshots. A repository-wide
  StoreSync census is a done input, not an optional search: the set of definitions,
  constructors, matches, exporters, schemas, snapshots, and compatibility fixtures must be
  recorded and every member must be owned by this commit.

  **Unresolved installed-broker prerequisite:** T592 MUST NOT start or claim the 3/1
  source path until FR-070's accepted external disposition has landed and the source
  generation under test has atomically installed the exact nonempty 13-member
  `SourceGenerationCompatibilityFloorV1` census as part of the existing
  `d2b-priv-broker.service` lifecycle. Every role occurs once under one disposition and
  source generation; `missing`, `duplicate`, `extra`, `empty`, `stale-generation`,
  `stale-digest`, and `cross-disposition` members refuse. At committed HEAD
  the installed daemon/broker wire has
  no handoff operation, its catalogue fingerprint is the legacy protocol-4 set, and target
  `host-broker.nix` cannot change that executable before profile publication. T592 owns no
  source peer, source contract artifact, source-generation install step, or source apply
  object and may not substitute a target-only compatibility binary, new unit, runtime
  override, child, mutating entrypoint, daemon recovery path, or synthetic source image.

  After that prerequisite, T592 freezes only the target half of the transition. The external
  source contract is `BeginHostGenerationHandoffV1`, selectable only after both installed
  source peers match numeric protocol 4 and Hello `operation_catalogue_sha256` equal to the
  exact `source-handoff-v1` catalogue fingerprint. T592 implements the protocol-5
  `AdoptHostGenerationHandoffV1` target-broker
  operation and atomically updates only its target schema, operation/privilege catalogue,
  compatibility disposition, capability fingerprint, serialization snapshot, and
  positive/negative fixtures. Bare committed protocol 4 advertises a different catalogue
  fingerprint and refuses before fd transfer; the target-v5 row cannot be selected on
  protocol 4 or under the source fingerprint. No decoder aliases one row to the other or
  accepts an unversioned or fingerprint-unnegotiated handoff request. Together the externally
  frozen source row and T592's target row carry the closed
  `adopt`, `publish-system-profile`, `activate-target-broker`, `activate-target-daemon`,
  `publish-d2b-state`, `acknowledge`, `prepare-rollback`, `restore-d2b-state`,
  `rollback-system-profile`, `restore-source-services`, and `repair-reference` phases.
  The externally installed source-generation broker is the only protocol-4 executor for the
  pre-transfer phases of that typed state machine. Its ordinary `serve` process runs only
  under the existing `d2b-priv-broker.service`, is reached through the existing socket, and
  is restarted by that unit's existing `Restart=on-failure` lifecycle before ownership
  transfer; it is never a target-closure-only mode, standalone child, transient unit,
  template, path, timer, or additional service. T592 defines and tests only the target
  broker's authenticated adoption of that externally produced coordinator/capability and
  checks the exact external source catalogue fingerprint as a read-only prerequisite; it
  does not regenerate, implement, install, or claim the source row or actor. Both
  request forms carry only a broker-resolved opaque
  handoff intent and bounded
  expected transition identity; neither carries a profile, pointer, unit, path, command, argv,
  reference bytes, or caller-selected generation. The identity binds source and target
  system-generation digests, broker binary/generation digests, daemon binary/generation
  digests, source and target broker protocol versions, selected Hello version, exact source
  and target operation-catalogue/capability digests, bundle-pointer generations, complete
  bundle-set digests, stable
  reference digest, and deployment-intent digest. A replay with any different member is a
  distinct typed refusal.

  Initial handoff admission MUST traverse the existing public `d2bd` socket and its
  `SO_PEERCRED` plus current `d2b`-group lifecycle classification while the invoking operator
  is still unprivileged. For the installed 3/1 compatibility path, transfer the accepted
  public-socket evidence as exactly one owned capability attachment only after the installed
  source peers' authenticated Hello matches numeric protocol 4 and the exact
  `source-handoff-v1` operation-catalogue fingerprint. The compatibility receiver consumes
  that attachment into one sealed, non-serializable, non-`Clone`, non-`Copy`, nonfabricable
  durable handoff capability bound to the complete staged intent and transition identity. It must not
  accept a serialized uid/gid, root bit, provenance, daemon identity, role, caller
  classification, or other claim as evidence. The normal and compatibility broker may
  advance phases only by consuming that capability or a broker-issued phase attenuation of
  it. No token, digest, fd number, role claim, environment value, path, or argv can
  reconstruct or transfer authority. A daemon identity, successful Hello, broker-socket peer
  credential, effective uid 0, or target-closure provenance is an eligibility/integrity input
  only and never independently authorizes a phase.

  Before authorization returns success, the broker must require exactly one resolved target
  output, create and own its GC root, and durably pin its canonical store path, NAR hash,
  unprivileged deployment-executable digest, staged intent, and transition identity. It
  separately resolves from trusted installed-generation metadata and pins the canonical store
  identity and digest of the broker-managed privileged apply object. The caller-flake target
  executable runs only for unprivileged authorization and never under `sudo`. Privileged
  apply invokes only the installed pinned object, receives no flake URI, installable,
  reference path, target executable, command, or argv to reevaluate, and may perform no Nix
  eval, build, `nix run`, installable resolution, or symlink lookup. The broker reopens and
  verifies both pins and the GC root before any mutation. It obtains a connection-scoped peer
  pidfd directly from the accepted apply socket, binds that live process and its current
  executable store/NAR/digest identity to the installed-apply-object pin, and revalidates
  pidfd liveness, process start identity, and executable identity immediately before every
  privileged mutation. Peer exit, exec, numeric PID reuse, target/apply mismatch, changed
  installed symlink, missing or replaced GC root, digest mismatch, ambiguous identity, or
  cross-intent replay is denied. The pidfd and executable fds are closed with the connection;
  no pidfd, numeric PID, or proc path is serialized or persisted.

  Before the first privileged mutation, the capability-authorized normal or compatibility broker
  must durably create the broker-owned handoff coordinator and record its exact owner
  generation, pinned executable identity, existing-unit lifecycle identity, and phase. The
  compatibility mode under `d2b-priv-broker.service` retains ownership until the target
  protocol-5 broker authenticates and durably adopts it; ownership then transfers exactly
  once before target daemon activation. A compatibility-process crash before transfer is
  restarted by that existing unit and may reopen only the same coordinator with the matching
  capability and pin; after transfer only the existing
  target `d2b-priv-broker.service` may reopen it. The old owner, `d2bd`, Nix activation, and
  the deployment entrypoint cannot resume, roll back, or transfer it. No new unit is added.

  T595's caller-flake target-closure deployment entrypoint may run only unprivileged and may
  only validate inputs, build, verify, and durably stage the complete immutable closure,
  resolve one exact target store output while unprivileged, obtain initial authorization
  through the accepted public-socket evidence, require the broker-managed target-object,
  GC-root, and installed-apply-object pins, and submit the opaque intent to the installed
  protocol-5 broker or the existing broker service's compatibility mode. It has no profile,
  service-control, bootstrap, or rollback mutation code and cannot initiate rollback. The
  capability-authorized normal or compatibility broker is the sole owner of system-profile
  publication, NixOS broker/daemon service
  transition, 3/1 bootstrap, stock rollback, and source-service restoration, as well as the
  sole publisher and repair owner of the d2b bundle pointer and
  `/etc/d2b/host-generation-rebuild-ref`. It reads the
  reference only from the verified immutable target-closure input; publishes with
  create-exclusive temporary state, owner/mode/type/link-count checks, file sync, atomic
  rename, and parent-directory sync; audits only the fixed transition/reference digests; and
  restores the prior bytes or verified absence before acknowledging rollback preparation.
  Activation code and `d2bd` may stage or name only immutable inputs and opaque identities,
  never create, replace, repair, or remove the stable file.

  Extend target broker Hello to negotiate the numeric broker protocol, exact broker
  generation, and exact `operation_catalogue_sha256`, not only package semver text. The
  external source floor must already perform the corresponding numeric-protocol-4 plus exact
  `source-handoff-v1` catalogue negotiation on both installed source peers. The one target
  ordering is: the unprivileged operator passes
  the public-socket Admin check and transfers its accepted-socket evidence through
  `BeginHostGenerationHandoffV1` over that exactly negotiated authenticated source channel;
  the installed source broker consumes that exact owned fd and durably seals the complete
  staged intent and its handoff capability before any mutation; the capability-authorized
  normal/compatibility broker adopts both, durably records coordinator ownership, writes the
  immutable pre-mutation
  audit, publishes the stock profile, and transitions the target broker. The target broker
  durably accepts coordinator ownership before the target daemon transition; protocol-5
  `d2bd` starts and
  completes exact-generation Hello while explicitly unready; the broker verifies Hello and
  the daemon as the eligible holder of a phase attenuation; the daemon submits the opaque
  authenticated `publish-d2b-state` request with that attenuation; the broker durably
  publishes and audits pointer/reference state, including file and parent-directory sync;
  the daemon reopens the durable publication and only then performs ingestion and becomes
  ready. A publication request without the matching sealed capability, staged intent, and
  Hello is denied, and the broker must not publish d2b state before all three.
  The protocol-5 broker may serve the pinned legacy non-handoff protocol-4 catalogue while
  the source daemon is still active, but it never exposes
  `AdoptHostGenerationHandoffV1` on a protocol-4 selection and never pretends the legacy
  protocol-4 fingerprint supplied `BeginHostGenerationHandoffV1`. Only the externally
  installed source peers may advertise the exact source-handoff catalogue fingerprint.
  Transition to the target daemon requires a fresh protocol-5 connection. Broker restart
  reopens the coordinator through the existing service; target daemon startup or
  reconciliation failure is therefore recoverable without daemon ownership. Restart,
  rollback, and crash recovery re-derive both process generations and refuse a stale broker,
  stale daemon, capability drift, downgrade, skipped Hello, cross-generation
  acknowledgement, or wrong coordinator owner.

  Authorization begins only with Admin classification from the accepted public socket's
  `SO_PEERCRED` and current `d2b` group membership, then continues only through the sealed
  durable handoff capability. Broker-socket `SO_PEERCRED`, configured d2bd uid/gid, negotiated
  daemon generation, and Hello authenticate the eligible process but grant no authority on
  their own. Never trust `BrokerRequestEnvelope.caller_role` for this op. The canonical Rust
  and Nix privilege matrices require the matching capability for every phase. Explicit
  negatives deny missing, forged, copied, replayed, wrong-intent, wrong-phase, and
  cross-generation capabilities; AdminUid on the broker socket, LauncherUid, RootUid,
  HostShutdown, NotAuthorized, a remote/public-socket resource subject, a caller-claimed
  daemon role, daemon uid/gid/generation without the capability, and a request before Hello.
  Each phase emits an immutable broker audit record with typed action, outcome, and fixed
  transition digest before its mutation and another immutable outcome before
  acknowledgement. Compatibility mode may require effective uid 0 to execute its already
  authorized mutation, but euid 0 is never authorization; it also verifies its own binary
  and target-closure generation against the staged identity and accepts no caller-selected
  target. Phase skip, direct path/unit selection, rollback without matching durable
  publication, or reference repair without matching trusted input is denied. No new service
  or unit is added.

  Define typed `OpenPeerPidfdFromAcceptedSocket` in the same atomic broker transition. The
  caller passes exactly one accepted Unix socket with `SCM_RIGHTS`; no numeric PID, raw-fd
  integer, credential tuple, or path is serializable. The response transfers only the
  resulting `OwnedFd` pidfd with `FD_CLOEXEC`. T592 owns both ancillary receive paths in its
  already listed `packages/d2b-priv-broker/src/protocol.rs` and `packages/d2bd/src/lib.rs`
  scope: broker receipt of the accepted socket and daemon receipt of the returned pidfd. Both
  must call `recvmsg` with `MSG_CMSG_CLOEXEC`, reject `MSG_CTRUNC`, walk the complete control
  message set, take ownership of every nonnegative received fd before JSON/variant/index/type
  validation, require exactly one expected fd, verify `FD_CLOEXEC`, and close all received
  fds on every count, decode, type, index, or later invariant failure. No first-fd-wins or
  ignored-extra behavior is permitted. Use a safe dependency API if one satisfies
  the exact no-panic, exact-`optlen`, returned-fd ownership, and cleanup contract. Otherwise
  the raw `getsockopt(SO_PEERPIDFD)` wrapper lives only in the existing approved
  `d2b-priv-broker/src/sys.rs` FFI quarantine, under the narrowest item-level allowance and
  with a `SAFETY:` comment immediately justifying every unsafe block. No repository-authored
  unsafe is permitted in any other file or crate. The wrapper passes exact `c_int` length,
  validates returned `optlen`, takes ownership of every nonnegative returned fd before
  checking syscall outcome or later invariants, and closes it on all failure paths. The
  `nix` 0.31.3 `MaybeUninit`/assert wrapper, a new project-authored FFI crate, and any local
  session syscall fallback are forbidden.

  The `policy_peer_pidfd_quarantine` source guard must enumerate every repository-authored
  `SO_PEERPIDFD`, `getsockopt`, raw-fd ownership, and pidfd-acquisition site and require the
  set to be nonempty and exclusive to the safe dependency wrapper or approved broker
  `sys.rs` quarantine. It also requires an immediately preceding `SAFETY:` justification for
  every quarantined unsafe block. Independent poison fixtures place an unsafe wrapper in a
  second crate, add a second pidfd-acquisition site, remove or separate a `SAFETY:` comment,
  and import or call the forbidden `nix` 0.31.3 `PeerPidfd` `MaybeUninit`/assert wrapper.
  Each must fail for its intended reason through the single enforcing runner
  `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`. That runner owns this policy;
  `make test-policy` must not duplicate, invoke, or claim it.

  Focused tests independently poison every handoff identity member, Hello field, sealed
  capability member/phase, caller class, phase edge, and reference publication/repair check.
  They validate the exact nonempty 13-member
  `SourceGenerationCompatibilityFloorV1` role census from `data-model.md` and assert that
  both peers plus all eleven other members share one accepted disposition and source
  generation. Independently remove each role, duplicate and add a role, make a member empty,
  substitute a stale-generation or stale-digest member, and splice a valid member from
  another disposition; every poison refuses before fd transfer, authorization, or mutation.
  T592 consumes the source census read-only, then proves its target-v5 row, schema, catalogue,
  fingerprint, snapshot, and fixtures move together. Bare protocol 4, a source-peer
  fingerprint mismatch, either crossed protocol/catalogue selection, a missing source row,
  or an unversioned alias refuses. They prove public-socket Admin classification is required before
  `BeginHostGenerationHandoffV1` can transfer exactly one accepted-socket fd, the installed
  source broker alone consumes it into the durable capability, and root, provenance, daemon
  identity, broker credentials, or caller claims without that capability refuse. Target
  resolution returning zero or multiple outputs refuses before authorization.
  After authorization, independently substitute the target executable and installed apply
  object, replace the broker-managed GC root with a root for another store object, delete and
  recreate that root, and retarget the installed symlink; every case refuses before mutation
  while the original pinned tuple remains eligible.   On the apply connection, inject peer exit, exec to another executable, numeric PID reuse,
  start-identity mismatch, executable identity mismatch, and multiple plausible identities
  before the first mutation; every case refuses, closes the connection-scoped
  pidfd/executable fds, and persists neither. Then allow exactly
  `host-generation.source-bootstrap-publish` and its audit to become durable and, for each
  of the fourteen later ids in the independent closed registry from `quickstart.md`, inject
  every transition independently before the selected edge. The exact post-first fixture set
  has the 84 literal ids in T589's
  `tests/golden/delivery/host-generation-apply-peer-case-ids.txt` and is not derived from the
  typed production catalogue. Every case must
  refuse before that edge executes, retain the earlier committed audit,
  and report zero mutations for the selected edge and all successors. Distinct raw numeric
  PID, start identity, executable store path, derivation name, NAR identity, NAR hash, and
  executable digest canaries must occur zero times outside verifier-local kernel handles and
  bytes, including coordinator state, persistence, receipts/evidence, human, JSON, wire,
  error/`Display`, log, tracing event/span, metric name/label/value/exemplar, audit, panic,
  and every `Debug` output. A correlation field may carry only its typed fixed
  domain-separated digest; metrics carry no peer-identity label or value. Inject crashes
  before and
  after coordinator creation, profile publication, each broker/daemon service transition,
  both sides of compatibility-to-target-broker durable ownership transfer, Hello, reference
  temporary-file sync, rename, directory sync, pointer publication, acknowledgement,
  rollback preparation, prior-reference restoration, stock rollback, source-service
  restoration, and every pre-mutation/outcome audit durability boundary.
  Inject target broker startup failure and target daemon startup/reconciliation failure.
  Before transfer only the matching compatibility owner under the existing broker service
  may resume or roll back; after transfer
  restart the existing target broker service and prove it completes or rolls back without
  daemon or entrypoint ownership. Compatibility-broker crash cases cover before coordinator
  durability, after coordinator durability but before first mutation, after every
  pre-transfer mutation, and immediately before and after ownership transfer. Recovery
  exposes one matching broker/daemon/pointer/reference generation or refuses; it
  never accepts an unaudited direct mutation or leaves target reference bytes after prior
  state is restored. Peer-pidfd tests cover exact success, unsupported kernel, short and
  oversized `optlen` with and without a returned fd, syscall error with returned fd,
  missing CLOEXEC, closed peer, wrong caller, and both ancillary receive paths with
  missing/extra/truncated/malformed control data and failure after receipt. Descriptor-count
  stability and exec-leak probes run around every success and failure and prove every
  unexpected or error-path fd is closed.

  **Additional T592 serialized audit migration ownership:** include
  `packages/d2b-session/src/audit.rs`, `packages/d2b-bus/src/audit.rs`,
  `packages/d2b-core-controller/src/authz_audit.rs`, `packages/d2b/src/zone_audit.rs`,
  `packages/d2b-priv-broker/src/ops/{store_sync_audit.rs,store_sync.rs,store_sync_export.rs}`,
  new `packages/d2b-session/tests/audit_digest_contract.rs`,
  `packages/d2b-bus/tests/audit_digest_contract.rs`,
  `packages/d2b-core-controller/tests/authz_audit_digest_contract.rs`, and
  `packages/d2b/tests/zone_audit_digest_contract.rs`, plus
  `packages/{d2b-audit,d2b-session,d2b-bus,d2b-core-controller,d2b-resource-store-redb,d2b-priv-broker,d2b}/Cargo.toml`
  plus `packages/Cargo.lock`. Migrate every raw constructor, producer, exporter, CLI validator,
  and test to sealed domain-specific digest types; no raw identifier overload or conversion
  remains. Consume this task's exact typed `StoreSyncRequest` and `StoreSyncResponse`
  definitions in every StoreSync producer, broker dispatcher/handler, completion path,
  exporter, and schema/snapshot consumer; no producer may retain a parallel raw or
  derived-`Debug` DTO.
  Every sensitive journal/replay/audit/broker-drain DTO, error, and owning object,
  including this task's drain request, both StoreSync wire DTOs, all StoreSync audit
  producers and exporters,
  `SegmentWriter`, sink, exporter, held directory owner, and opaque storage handle owner, has
  a fixed redacted `Debug` implementation rather than derived field formatting. StoreSync
  request, completion, and export events accept only their own typed domain-separated fixed
  digests; no StoreSync path accepts a raw operation, subject, Zone, resource, correlation,
  trace, or host-path identifier. Present valid trace context becomes only a typed
  domain-separated trace digest, absence remains `None`, and malformed input is denied before
  mutation; no path may fabricate a digest for absence or relabel operation/correlation data
  as trace correlation. Focused and T598 end-to-end canaries must include raw identifiers,
  trace text, root path state, and opaque handle values across every StoreSync producer,
  wire schema/snapshot, record, segment, export, error, log, metric, span, CLI output, and
  `Debug`.

- [ ] T593 [US1] **Publish the authenticated Resource API and watch route.** Depends on T592. Owned files: `packages/d2b-bus/src/{router.rs,registry.rs,authorization.rs,operations.rs,streams.rs,session_seam_tests.rs,transport/unix.rs}`, `packages/d2b-resource-api/src/{adapter.rs,watch.rs}`, `packages/d2b-contracts/src/v3/services.rs`, `packages/d2b-session-unix/src/{lib.rs,adapter.rs,descriptor.rs,pidfd.rs,socket.rs,error.rs,subject.rs,zone_admission.rs}`, `packages/d2b-session-unix/tests/{subject_mapping.rs,unix_session.rs}` plus new compile-fail fixtures in that directory, `packages/d2b-bus/tests/{production_resource_route.rs,public_mint_surface.rs}`, and accepted normative specification `docs/specs/ADR-046-componentsession-and-bus.md`. T593 may not create or edit a project-authored FFI crate, `packages/d2b-priv-broker/src/sys.rs`, a Cargo manifest, or a lockfile; T592 has already frozen the only broker wire, FFI, and dependency boundary. Replace the unregistered production seam with a route whose registration consumes the authenticated ComponentSession admission. At Unix accept, transfer the accepted socket to T592's typed `OpenPeerPidfdFromAcceptedSocket` broker operation with `SCM_RIGHTS` and consume its returned `OwnedFd` pidfd; `pidfd_open(SO_PEERCRED.pid)` is forbidden. T593 must use T592's two receive helpers unchanged: both set `MSG_CMSG_CLOEXEC`, reject truncated control data, require exactly one expected fd, and close all excess or error-path fds. No request or session type carries a raw descriptor integer, credential tuple, or numeric PID. The session adapter, descriptor, bus Unix transport, and session seam must all consume the same accepted-socket evidence object; none may reacquire credentials, accept a caller-supplied verifier, or construct evidence from a credential tuple or numeric PID. Treat `SO_PEERPIDFD` support as part of the kernel floor and fail closed with an actionable unsupported-kernel error when the broker returns that typed refusal. Require `FD_CLOEXEC`, verify the `SO_PEERCRED` tuple, expected process generation/start identity, expected cgroup, and liveness against that exact fd, and consume all evidence into one private registrar issuer. Reject a dead fd, credential/generation/cgroup mismatch, ambiguous evidence, or any numeric-PID-only path. Remove the public `ZoneBootstrapIdentity::verify` path, its `Clone` implementation and identity/evidence accessors, the `VerifiedUnixPeer::credentials` accessor, caller-supplied verifier and credential constructors, and every direct or transitive re-export that permits external issuance; neither type may expose construction fields, `Clone`, `Copy`, `Default`, conversions, raw credentials, pidfd, generation, or cgroup evidence. `ZoneRegistrar` exclusively derives and propagates the subject from its private mapping; requests and stream frames carry no subject claim. Register exact-Zone ResourceService and controller routes; add T589's `InspectOperation` to the closed service/method catalogue, authorization map, and router; admit watch replay/live delivery through ZoneBus; and expose one registration/readiness observation from actual owned handles. Bump accepted `ADR-046-componentsession-and-bus` from Version 1 to Version 2 and normatively pin accepted-socket transfer to the typed broker operation, private registrar issuance, consumed evidence, and the sealed public surface; T593 updates source-level mint, compile-fail, adapter, transport, and session-seam seals, T605 serialized after T593 regenerates the shared API snapshots, and T220 coordinates generated manifests, references, tests, and changelog treatment. **Done when** same-Zone authenticated Get/List/Watch/InspectOperation reaches the real service; cross-Zone, self-named, unregistered, reused-admission, direct-WatchService, missing/extra/truncated/malformed ancillary fd, post-receive decode failure, numeric-PID-only, post-credential PID reuse, dead-pidfd, credential/generation/cgroup mismatch, unsupported `SO_PEERPIDFD`, and ambiguity paths are denied with stable descriptor counts and no exec leak; existing adapter, descriptor, Unix transport, subject-mapping, Unix-session, and session-seam tests use accepted-socket evidence and reject all caller-supplied verifier/credential paths; external compile-fail/API-surface checks prove no public verifier, constructor, clone, credential/evidence accessor, conversion, re-export, or alternate issuer survives; source policy proves `d2b-session-unix` retains workspace `unsafe_code = "forbid"` with no local syscall/raw-fd fallback or project-authored FFI dependency; and neither `UnregisteredBusAdapter` nor a fixed endpoint can satisfy production publication.
- [ ] T594 [P] [US1] **Bind controller fan-in, effects, and cleanup to the durable replay/adoption ledger.** Depends on T589. Owned files: `packages/d2b-core-controller/src/{runtime.rs,resource_store.rs,provider_effects.rs,cleanup.rs,watches.rs,controllers.rs}`, `packages/d2b-controller-toolkit/src/{context.rs,runner.rs}`, and their existing focused unit tests. Register the production endpoint, consume admitted watch frames into the bounded fan-in, and record every post-commit effect intent before `EffectPort`. Bind each ledger entry to Zone, controller generation, resource UID, committed revision, operation id, and effect ordinal; reuse that key for idempotent dispatch/adoption. On restart relist and adopt/replay pending entries before cleanup. Complete cleanup only by compare-and-set on the same UID and exact nonzero expected revision. **Done when** unit crash-window tests prove no effect before commit or ledger durability, replay/adoption after every later crash point, no lost cleanup intent, and denial of stale, zero, wrong-UID, wrong-generation, or ambiguous completion.
- [ ] T605 [US1] **Correct and pin the system-core Zone handler contract.** Depends on T593. Sole writable ownership: `packages/d2b-contracts/src/v3/zone.rs`; compiler-regenerated public and private snapshots under `tests/golden/api-surface/`, regenerated only with `make api-surface-pin`; the existing lowest-layer guard `packages/d2b-contract-tests/tests/policy_contracts.rs`; governing normative specifications `docs/specs/providers/ADR-046-provider-system-core.md` and `docs/specs/ADR-046-resources-zone-control.md`; and paired reference page `docs/reference/resource-plane-runtime.md` (adapt). Add `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`; the one exact serialized spelling is `system-core-host` and `system-core-user`, matching the committed kebab-case rule. Bump both governing specification `Version` values and state explicitly that internal/telemetry `handler` labels remain `system_core_host` and `system_core_user` while those underscore values are forbidden in serialized `Zone.status.handlers[]`. The Zone status-handler contract MUST accept exactly one record with each serialized name, phase, and `lastReconciledAt`, reject duplicate or missing records, underscore/wrong-name substitution, and preserve `ZoneHandlerName::ProviderLifecycle` as a distinct allowed value that cannot substitute for either. Treat `packages/xtask/src/zone_schema.rs`, `docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json`, downstream T595/T599 consumers, and integrator-owned generated spec manifests as read-only inputs: because `ZoneSpec` is unchanged, generator execution MUST leave the committed desired-state schema byte-identical. **Done when** focused `d2b-contracts` tests prove both exact wire round-trips, underscore rejection, exactly-one-each list acceptance, duplicate/missing/wrong-name rejection, and `ProviderLifecycle` preservation/non-substitution; both normative specs and version metadata, the targeted guard, paired reference page, and public/private API snapshots pin the same pre-consumer distinction plus T593's removal of public peer/bootstrap issuance and evidence access after `make api-surface-pin`; the Zone desired schema is byte-identical before and after its existing generator; and the targeted contract plus `make test-rust-api-surface` pass. T605 does not wait for or attest to T595/T599 output and does not run the full `make test-drift`; T595 owns the emitter, T599 owns later consumer reconciliation, and T220 owns generated-manifest reconciliation plus the final drift gate.
- [ ] T595 [US1] **Compose the production Zone runtime and one readiness projection.** Depends explicitly on T590, T592, T594, and T605; T591 and T593 are transitive prerequisites through T592 and T605. It also refuses unless FR-070's external source-generation compatibility floor was accepted and installed before T589; T595's target `host-broker.nix` edit cannot retroactively supply the source service's executable. Sole owned files: `packages/d2bd/src/resource_runtime.rs`, `packages/d2bd/src/lib.rs`, `packages/d2bd/Cargo.toml`, new `packages/d2b/src/bin/d2b-host-generation-deploy.rs`, `packages/d2b/Cargo.toml`, `nixos-modules/{bundle-zones.nix,host-daemon.nix,host-broker.nix,options-site.nix}`, new `tests/unit/nix/cases/host-generation-rebuild-ref.nix`, new `tests/host-integration/host-generation-handoff.nix`, accepted normative specification `docs/specs/ADR-046-nix-configuration.md`, and focused unit tests inside `resource_runtime.rs`. Replace hard-coded mutable revision identities and independent booleans with the recovered store metadata and owned handles from T590-T594. On daemon startup, ingest every installed `zones/<zone>/resource-bundle.json` generation before publication. Publish a deterministic bundle path/content trigger from `bundle-zones.nix`. `d2bd` requests only T592's typed handoff phases and never writes a system profile, generation pointer, stable reference, systemd unit, or rollback target directly. The target-closure deployment entrypoint may only validate parameterized inputs, resolve and verify exactly one target store output and executable while unprivileged, build and verify the complete closure, durably stage immutable transition bytes, transfer the accepted public-socket authorization evidence only after the externally installed source peers negotiate numeric protocol 4 plus the exact `source-handoff-v1` catalogue fingerprint, require the broker-managed GC root plus separate target-object and installed-apply-object pins, probe capability, and submit one opaque intent. The caller-flake target executable performs only the unprivileged authorization. Privileged apply invokes only the separately broker-resolved immutable apply object from trusted installed-generation metadata and performs no Nix eval, build, `nix run`, installable resolution, or symlink lookup. It receives no flake URI, installable, reference, target executable, command, or argv to reevaluate, and `--apply-authorized-handoff` carries no intent selector or authority token. Under the coordinator lock, the broker permits exactly zero or one durable nonterminal intent per source generation, refuses a second authorization while one exists, and atomically claims only the sole `authorized-pending` intent for one accepted apply connection. Zero, multiple, already-claimed, concurrent, and terminal intent selection refuses before mutation. A pre-mutation disconnect may return that same intent to pending only after a durable zero-mutation proof; after mutation, only coordinator replay of the same intent may bind a replacement connection from the same pinned apply object after proving the old peer dead. The broker binds the accepted apply connection's direct peer pidfd and live executable store/NAR/digest identity to that pin and revalidates liveness, start identity, and current executable immediately before every mutation; peer exit, exec, PID reuse, mismatch, or ambiguity refuses, and the connection-scoped pidfd is never persisted. The entrypoint captures at most 16,384 Nix evaluator/builder stderr bytes in memory and never writes them to a file; exceeding that ceiling is a fail-closed stage failure. It drops all raw bytes before return after deriving only a fixed internal digest/size if needed and emits the closed identifier-free typed stage error with remediation `rebuild-host-generation`; no raw line reaches human, JSON, wire, log, audit, metric, span, or `Debug`. It contains no profile publication, service control, 3/1 bootstrap mutation, or rollback code and is never a rollback initiator. Before transfer, the externally installed source-generation compatibility broker performs every privileged profile, service, bootstrap, publication, repair, and rollback phase by consuming the matching sealed durable handoff capability and both exact pins; after transfer, T592's target broker owns the remaining phases. Both emit immutable pre-mutation and outcome audit. Daemon identity, bootstrap euid 0, target-closure provenance, broker credentials, and caller claims never authorize independently. Target broker activation precedes target daemon activation. The target daemon performs a fresh exact-generation protocol-5 Hello while explicitly unready, then presents only its broker-issued phase attenuation with the opaque authenticated publish request; broker publication and its audit are file-and-directory durable before daemon ingestion and readiness. The externally installed source broker's durable coordinator exists before the first mutation and records `d2b-priv-broker.service` as the sole lifecycle owner. That existing service starts/restarts its installed source broker before transfer; ownership then transfers exactly once to the target broker before daemon activation. Before transfer only that compatibility owner may reopen it; after transfer the existing target broker service reopens it across broker restart or target-daemon startup failure. `d2bd.service` reports readiness and presents an attenuation but never owns or initiates recovery; the entrypoint is never a supervisor. It remains unready until one complete source or target generation tuple is durable. No new unit, timer, path unit, runtime override, or singleton service is added. Establish the verified Unix ComponentSession through the registrar using the pidfd returned by T592's typed broker operation for the accepted socket; after daemon restart rediscover the peer and acquire a new peer pidfd rather than opening one from or accepting a persisted numeric PID. Consume `PolicyBootstrapRead` for the first `NativeAuthorizer` install, switch all later policy access to the authenticated Resource API, register ResourceService and controller endpoints, admit the watch cursor, expose T589's typed `InspectOperation` through the production daemon/client path, start authoritative-journal export and retention enforcement, recover controller effects, and only then publish the Zone. Bump accepted `ADR-046-nix-configuration` from Version 2 to Version 3 and normatively pin the parameterized validation and public-socket authorization step, separate target-object and installed-apply-object pins, private Nix-stderr handling, build/stage/request-only entrypoint, capability-authorized source/target broker mutation ownership, existing-unit pre-transfer supervision, broker-before-daemon ordering, durable-intent/Hello-unready/authenticated-request/durable-publication/ingestion ordering, broker-owned coordinator recovery, stable-reference ownership, plus T592's bounded `audit.retentionDays`, `audit.maxRecordsPerSegment`, and `audit.maxSegmentBytes` option/compiler schema; T220 coordinates generated manifests, reference pages, tests, schemas, and changelog treatment. Consume T605's exact `system-core-host` and `system-core-user` variants to project exactly one record of each from the live owned `HostReconciler` and `UserReconciler` health handles for the `d2b-core-controller`-owned `Provider/system-core` registration. `ProviderLifecycle` is distinct and cannot satisfy either record. Do not wait for other Wave 6 Provider dossiers. Public resource and operation-status requests must traverse that session/router path and must never promote `SO_PEERCRED` role to a Resource API subject. Refactor startup and shutdown to visit every Zone and return a per-Zone report: a failed Zone stays unpublished and actionable, while unrelated Zones continue; close aggregates errors and never drops later runtimes. **Done when** initial boot and a deployment-entrypoint switch that adds, changes, or removes a resource bundle cause ingestion without a manual restart/reload; identical deployments produce no duplicate effect; same-ID operation inspection reaches T592's durable backend and wrong-binding remains unobservable; no public path constructs a subject; restart obtains fresh accepted-socket-derived peer pidfds; `require_ready` derives every member from live owned production handles rather than a boolean; invalid audit configuration or retention/prune health blocks only the affected Zone; duplicate/missing/wrong-name required records or `ProviderLifecycle` substitution degrade only that Zone; partial readiness is impossible; advanced revisions reopen; one Zone's open/close failure does not abort or discard another; and source-policy tests prove that the entrypoint only validates/builds/stages/authorizes/submits, privileged handoff mutations exist only in the capability-authorized externally installed source broker before transfer and T592's target broker after transfer, target/apply/GC-root substitutions, apply-peer identity failures, and raw-Nix-stderr canaries refuse without leakage, euid0/daemon identity/provenance/caller claims alone refuse, the existing broker service is the only pre-transfer lifecycle owner, and the broker-owned coordinator is the sole durable rollback-resume initiator before and after its exact ownership transfer.
  **Additional T595 source-floor and apply-peer closure:** consume only the exact nonempty
  13-member `SourceGenerationCompatibilityFloorV1` census and refuse every `missing`,
  `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and
  `cross-disposition` member before fd transfer or mutation. Revalidate apply-peer identity
  immediately before every privileged mutation. Tests race two authorizations, race two
  apply connections, inject a corrupt two-pending-intent census, disconnect before and after
  the first mutation, and invoke apply after terminal completion. They require exactly one
  claim winner only with one pending intent, no caller selector or token, zero mutations by
  refused contenders, same-intent coordinator replay only after mutation, and no terminal
  replay. Tests use the exact transition ids `peer-exit`, `peer-exec`, `peer-pid-reuse`,
  `peer-start-identity-mismatch`, `peer-executable-identity-mismatch`, and
  `peer-identity-ambiguity`. They use the independent closed 15-id mutation-edge registry
  in `quickstart.md`, beginning with `host-generation.source-bootstrap-publish` and ending
  with `host-generation.rollback-source-daemon-service`; production self-enumeration is not
  the expected set. Each transition is injected in a fresh six-case pre-first run. Then each
  is injected after the first mutation and audit are durable and immediately before each of
  the exact fourteen later edges, yielding exactly 84 distinct
  `apply-peer/post-first/<edge>/<transition>` cases. T589's literal
  `tests/golden/delivery/host-generation-apply-peer-case-ids.txt` fixture shares no generator
  with production and must match all six pre-first ids and all 84 post-first ids against the
  separately authored `host-generation-mutation-edge-ids.txt` and a third separately literal
  15-id test constant. All three mutation-edge meta-poisons and all 13 ids in
  `host-generation-post-first-negative-case-ids.txt`, including the explicit missing
  verification-hook case, must reach their intended invariant.
  Unknown, duplicate, reordered, missing, or unvisited edges fail. The selected edge and all
  successors remain unexecuted and the durable prefix plus first audit is unchanged.
  Outside transient verifier-local kernel handles and bytes, raw peer pidfd number, PID,
  start identity, socket uid/gid, cgroup/proc path, executable store path, derivation name,
  NAR identity/hash, content digest, or device/inode/mount identity are forbidden in
  coordinator state, receipts/evidence, human, JSON, wire, error/`Display`, log, tracing
  event/span, metric name/label/value/exemplar, audit, panic, and `Debug`. The exact fifteen
  literals in `host-generation-apply-peer-forbidden-values.tsv` are injected independently
  and occur zero times on every captured surface; only the fixture and private injection
  buffer are excluded. Only the exact fixed-binary process-instance and
  executable-identity correlation digest preimages from `data-model.md` are allowed;
  independently reconstructed wrong-domain/framing/endian/order/cross-class negatives must
  refuse. Metrics carry no raw or digested peer-identity label or value.
  **Additional T595 bundle wiring:** `bundle-zones.nix` reads only
  `d2b.zones.<zone>.audit`, passes the typed values to T592's v4/v2 renderer, and never
  synthesizes a Zone resource or writes audit policy into `Zone.spec` or another
  ResourceSpec. The daemon accepts only the exact 4/2 pair, verifies `contentHash` over
  canonical `{audit,resources}`, carries the three typed limits into the Zone audit owner,
  and rejects missing, old/mixed, future 5/2, 4/3, and 5/3, malformed, or unenforceable policy
  before publication.
  An audit-only Nix change must alter the generation identity and trigger the existing
  `d2bd.service` continuation path.
  **T595 installed-host migration and rollback:** the target configuration exposes
  `system.build.d2bHostGenerationDeploy`, so an installed 3/1 host obtains the new entrypoint
  from the target closure rather than an old binary or target-generated reference file. The
  same target derivation installs `/run/current-system/sw/bin/d2b-host-generation-deploy` for
  post-publication use; the target installable and installed binary execute one
  implementation rather than two deployment paths. The
  entrypoint accepts only its embedded validated target identity, builds every declared Zone
  to 4/2, verifies the complete immutable closure, and creates one file-and-directory-durable
  transition intent. Unprivileged preflight resolves exactly one canonical target store
  output and deployment executable. While still unprivileged it must traverse the installed
  public socket and transfer the accepted authorization evidence only after the externally
  installed source daemon and broker have authenticated and negotiated numeric protocol 4
  plus the exact `source-handoff-v1` operation-catalogue fingerprint. The source broker
  consumes that evidence, creates the durable capability, owns a GC root plus the exact
  target store/NAR/deployment-executable pin, and separately resolves and pins the immutable
  apply object from trusted installed-generation metadata before authorization succeeds; no
  authority token is emitted. The caller-flake deployment executable never runs under
  `sudo`. The privileged command invokes only the separately pinned installed apply object,
  which performs no Nix reevaluation and submits only the opaque intent identity. Its
  accepted connection must remain bound through a direct peer pidfd to the same live process
  start and executable store/NAR/digest identity; exit, exec, PID reuse, mismatch, or
  ambiguity refuses before mutation, and no pidfd is persisted. Exact protocol 5 plus the
  sealed handoff capability submits to the target broker after transfer. Bare protocol 4 or
  any source-peer catalogue-fingerprint mismatch refuses before fd transfer or
  authorization. Only the externally installed source compatibility peers under the existing
  lifecycle may consume the accepted-socket evidence, mint the capability, and reopen it
  after that unit restarts the actor before transfer. Any other downgrade, target/apply
  executable or symlink substitution, missing pin, capability mismatch, source-generation
  mismatch, caller-selected path/unit/generation, missing public-socket Admin evidence, or
  non-root compatibility executor refuses before mutation. Root execution, target
  provenance, daemon identity, and caller claims alone also refuse.

  The capability-authorized externally installed source broker owns stock NixOS
  system-profile publication, target broker/daemon service transitions, 3/1 bootstrap, and
  pre-transfer stock rollback; the target broker owns only phases after durable transfer.
  `host-broker.nix` and `host-daemon.nix` require target broker activation before target
  daemon start. The new broker adopts and audits the staged identity. The target daemon then
  performs fresh protocol-5 Hello while unready. That Hello must match
  the handoff's target broker binary/generation, daemon binary/generation, selected protocol,
  catalogue digest, pointer generation, bundle-set digests, and reference digest before
  the daemon may present its broker-issued phase attenuation in the authenticated opaque
  publication request. The daemon identity and Hello do not authorize without that
  attenuation. The broker publishes the complete d2b
  bundle pointer and stable reference with file and directory durability and immutable audit;
  only then may ingestion or readiness proceed. An already acknowledged deployment is
  idempotent.

  A failed build or staging step leaves the source profile and services untouched. Failure
  after stock publication but before target readiness makes the broker-owned durable
  coordinator reopen the handoff. Before ownership transfer only the matching compatibility
  owner under the existing broker service may resume or roll back; afterward the existing target
  `d2b-priv-broker.service` reopens it after restart and proceeds without a live target
  daemon. The broker durably records rollback
  preparation, restores the
  prior d2b pointer plus exact prior reference bytes or verified absence, performs stock
  rollback, and restores source services. Killing the deployment entrypoint at every crash
  point does not stop this recovery, because the existing broker unit, not the entrypoint, is
  the lifecycle owner. A pre-transfer compatibility-process death is restarted by that same
  unit and reopens the durable coordinator. Crash
  recovery resumes at transition-intent, capability, and coordinator durability,
  profile publication, each broker/daemon service transition, coordinator ownership
  transfer, target Hello, d2b
  pointer/reference publication, readiness, rollback preparation, pointer/reference
  restoration, stock rollback, or source-service restoration without mixing generations or
  duplicating ingestion/effect. Every privileged mutation has immutable pre-mutation and
  outcome audit. Static and executable negatives reject entrypoint mutation or rollback
  initiation, another profile/service initiator, direct `d2bd` or activation
  pointer/reference writes, direct activation reference repair, caller-supplied
  generation/unit/path targets, and any privileged transition that lacks its record and
  audit.

  Add required `d2b.site.hostGenerationRebuildRef` with no default. Its type is
  `lib.types.strMatching "^[A-Za-z0-9+._~:/?@%=&,-]+#[A-Za-z0-9][A-Za-z0-9_-]{0,63}$"`
  plus an exact 2048-byte UTF-8 limit. It accepts exactly one nonempty ASCII
  `<flake-ref>#<configuration-name>`; the selector is 1-64 bytes, begins alphanumeric, and
  thereafter permits only alphanumerics, `_`, and `-`. The option description calls it an
  opaque rebuild locator, provides no fixed target example, and points to the parameterized
  validated procedure in `quickstart.md`. Missing, empty, 2049-byte,
  multiline, control, whitespace, selector-free, empty-selector, extra-`#`, slash/dot
  selector, or 65-byte selector values fail evaluation. Nix writes the exact value only into
  the immutable target closure. T592's broker publishes
  `/etc/d2b/host-generation-rebuild-ref` atomically as a regular `root:d2bd` `0640` file,
  records only its fixed digest, and owns all repair and rollback restoration. Neither
  activation nor `d2bd` writes it, and runtime output never renders the value or stable path.

  The Type-1 `host-generation-rebuild-ref.nix` case covers a normal value, exact 2048-byte
  success, 2049-byte failure, missing required option, every malformed line/character/hash
  form, selectors of 64 and 65 bytes, and missing/empty/invalid selectors. Run
  `make nix-unit-pin` after adding the auto-discovered case and include `make test-nix-unit`
  evidence. The Type-10 `host-generation-handoff.nix` test remains independently required for
  real NixOS profile, systemd, broker, file-durability, Hello, crash, and rollback behavior.

  Version refusal is an identifier-free typed error with fixed redacted `Debug` and the one
  closed remediation action `rebuild-host-generation`. Runtime human/JSON/wire output carries
  no command, argv, shell fragment, host/Zone identifier, or path. T592-owned reference
  documentation gives a runnable parameterized target-closure procedure for a 3/1
  host. It requires nonempty `D2B_HOST_FLAKE_REF` and
  `D2B_HOST_CONFIGURATION`, enables fail-fast shell behavior, validates the exact ASCII
  flake-ref grammar, the 1-64 byte configuration selector, and the composed 2048-byte bound,
  verifies the target evaluation's `d2b.site.hostGenerationRebuildRef` equals their exact
  composition, and constructs
  `${D2B_HOST_FLAKE_REF}#nixosConfigurations.${D2B_HOST_CONFIGURATION}.config.system.build.d2bHostGenerationDeploy`;
  it contains no fixed illustrative target. Every failed preflight exits before either the
  unprivileged public-socket authorization invocation or `sudo`. Runnable documentation sends
  Nix eval/build stderr directly to `/dev/null`, creates no diagnostic file, and emits only
  the fixed stage-specific failures shown in `quickstart.md`. The production entrypoint uses
  only a 16,384-byte in-memory ceiling, treats overflow as a fail-closed stage failure, drops
  all raw bytes, and emits only the closed `rebuild-host-generation` remediation. The
  procedure resolves the installable once with unprivileged
  `nix build --no-link --print-out-paths`, requires one canonical store output and executable,
  and runs that exact executable once unprivileged with `--authorize-handoff`. Accepted
  public-socket evidence durably seals authority and the broker-managed target-object pin
  without emitting a token. It then resolves the separately pinned installed apply object
  from trusted installed-generation metadata and runs only that object under `sudo` with
  `--apply-authorized-handoff`; the privileged command contains no `nix` invocation, target
  executable, URI, installable, or reference and refuses without the exact durable
  capability, both pins, and matching live connection peer pidfd/executable identity. Only
  after successful broker
  publication, it gives the exact
  installed `d2b-host-generation-deploy --from-reference
  /etc/d2b/host-generation-rebuild-ref` authorization/apply pair. It also gives a runnable
  rollback procedure parameterized by validated prior flake/configuration values, using the
  same split unprivileged target authorization then installed-apply-object
  capability-consumption pair, and the
  automatic recovery checks after a failed transition. Raw `nixos-rebuild` is not the
  documented entrypoint, and prior verified absence requires repeating the parameterized
  explicit bootstrap rather than creating the file. Focused canaries place distinct secrets,
  host identifiers, store paths, and arbitrary text in evaluator and builder stderr and prove
  that only the fixed error class/remediation appears in every output surface.
- [ ] T604 [P] [US1] **Prove exact-candidate operator activation through effect and cleanup.** Depends on T595 and is file-disjoint with T596-T599. Its 3/1 starting generation MUST be the independently installed source-generation compatibility floor accepted under FR-070; constructing that actor from F or starting from committed protocol 4 without the operation is ineligible. Sole owned files: new `packages/d2b-contract-tests/tests/resource_operator_activation.rs`, new `packages/d2bd/tests/resource_operator_activation.rs`, new `tests/host-integration/resource-operator-activation.nix`, existing `tests/host-integration/daemon-restart-vm-survival.nix`, new `tests/golden/delivery/host-generation-unit-census-case-ids.txt`, and only those checks' host-integration discovery/build recipe in `Makefile`. The fixture-backed contract test proves that an operator declaration emits the exact pinned `zones/<zone>/resource-bundle.json` generation and that removing one declaration emits the corresponding generation delta; run it through `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`. The Type-3 d2bd test is the lowest feasible production-boundary test: consume those exact generation bytes through the daemon startup/change-ingestion entry and production store/controller path, never through a direct ResourceService/WatchService call, and run it through `make test-rust`. The Type-10 test is additionally required because NixOS activation, systemd, broker mutation, and real owned host effects cannot be proved below `runNixOSTest`; run it only through the public heavy-gated `make test-host-integration` target. Preserve that existing target rather than adding a top-level gate, but make empty `vmChecks` discovery fail and emit the enumerated and built attr names. T604 evidence is ineligible if the target skips, discovers an empty set, omits, or does not build either `vmChecks.x86_64-linux.resource-operator-activation` or `vmChecks.x86_64-linux.daemon-restart-vm-survival`; a non-x86 skip is never passing evidence. In the positive host test, declare Zone `acceptance` with exactly these three acceptance resources and the support resources they require: (1) `Volume/acceptance-state` selects `Provider/volume-local`, whose `volume-local-provider` install has `controllerExecutionRef = Host/host-system` and the sole `state-root` local-path source policy for `state`; the Volume uses that opaque policy, one private `0700` no-follow root owned/grouped by `User/d2bd`, one controller view, and no attachment/quota; (2) `Network/acceptance-net` selects `Provider/network-local`, whose `provider-network-local` install has `controllerExecutionRef = Host/host-system`; the Network uses LAN `10.20.0.0/24`, uplink `192.0.2.0/30`, all four mandatory host-blocklist CIDRs, east-west denied, empty DNS forwarders and attachments, mDNS disabled, and the `net-vm-base` nixos-system artifact; (3) `Device/acceptance-tpm` selects `Provider/device-tpm`, whose `d2b-provider-device-tpm` install has `controllerExecutionRef = Host/host-system` and `logLevel = 20`; the Device is an exclusive emulated Device owned by `Guest/acceptance-vm`, with empty selector and `device-tpm.d2bus.org/Device/spec` version `1.0.0` settings `{ logLevel = 20; }`. Switch through T595's public target-closure deployment entrypoint rather than raw `nixos-rebuild`. Require the Volume's broker-provisioned/adopted root and identity marker plus layout readback and `Ready`/`Current`; the Network's two real derived bridges, IPv6-suppression readback, ownership-scoped firewall projection, ready config Volume/net-VM/agent dependencies, true `FabricReady`/`FirewallReady`/`ConfigVolumeReady`/`NetVmReady`/`DhcpReady`, and ready bridge phases; and the Device's controller-managed TPM state Volume/marker, mandatory flush, live broker-supervised swtpm Process, typed TPM Endpoint, and `Ready`/`Current` present/healthy status. Network-owned Guest dependencies prove only Network readiness and do not satisfy Guest acceptance. This is acceptance scope only; Network implementation remains owned by Wave 4. A status-only path or actionable refusal is ineligible for the positive story; refusal cases run separately as negative tests. T604 also owns SC-002 measurement collection in this Type-10 test, not implementation of the measured production path. Require the emitted single-Zone generation to contain 10 to 20 resources. Emit exactly one separately encoded `Sc002ActivationReceiptV1` as an external validation output: a regular single-link file owned by the current effective uid with mode exactly `0600`. T604 does not publish it under the candidate directory and does not construct the `EvidenceRecord` locator. T600 must pass this exact file through T589's `wave validate-import --sc002-receipt PATH`; that importer hashes before decode, derives `evidence-sidecars/sc002/sha256/<typed-digest>.json`, validates the actual outer `candidate_id`/`content_id`/`snapshot_sha256` triplet, and durably installs the current-effective-uid `0600` candidate leaf beneath current-effective-uid `0700` directories before publishing the unchanged schema-v2 `EvidenceRecord`. It has schema version 1, encoded size at most 16,384 bytes, one monotonic transition-intent start tick, and exactly one typed `ResourceIdentity` sample keyed to each of `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`. In every sample, the effect and production watch/read-model `Ready` observations must both name that exact sample identity; selected stop and all 1-32 progress observations repeat it too. The selected stop equals the later effect/Ready tick; checked elapsed nanoseconds equal stop minus start and are at most 2,000,000,000; and every progress tick is strictly after start and no later than stop. Do not add a test-only progress path, private hook, sleep, timeout, threshold weakening, or exclusion. Missing/unknown fields, wrong version/kind/size, wrong source owner/mode/link count, an absolute/traversal/URL/symlink/hard-link locator, replacement before hash/decode/reopen, crash or race that exposes a record before a file-and-directory-durable sidecar, a missing/duplicate/mixed/unrelated identity, effect/Ready identity disagreement, wrong event ordering, absent progress, stale binding, or an over-threshold sample leaves T604 incomplete; remediation belongs to the existing production-path owner under FR-030, after which the integrator freezes a replacement provisional candidate before any binding request and reruns exact-candidate evidence. Repeat an identical deployment and prove no duplicate ingestion or effect. Remove only `Device/acceptance-tpm`, deploy the next emitted generation without a manual `systemctl restart`, `reload`, private RPC, or test-only hook, and require its finalizer to set swtpm stopped, wait for terminal phase, delete the swtpm Process, delete any non-terminal flush `EphemeralProcess`, preserve the same controller-created TPM state-Volume identity and marker, release its Volume references, clear, allow the Device row to disappear, and leave the Endpoint unresolvable. Prove `Volume/acceptance-state`, `Network/acceptance-net`, their effects/identities, and unrelated resources remain ready, intact, and unrecreated. Guest runtime-effect ownership is deferred specifically to Wave 6 `Provider/runtime-cloud-hypervisor` and its T384/T479/T480 acceptance: Guest emission, ingestion, status, or refusal is not positive T604 evidence and cannot close FR-072 or SC-034. Bind all test records and both bundle content hashes to the same exact candidate. Run the FR-075 public lifecycle case on that candidate and require nonempty enumeration and a successful no-skip build of `vmChecks.x86_64-linux.daemon-restart-vm-survival` in addition to the T604 attr. Require its public `Ready` before daemon restart, continued reachability, `Stopped` after public stop, same runner PID/start-time through a newly acquired pidfd, numeric PID reuse/pidfd mismatch/multiple-plausible-runner quarantine negatives with no adoption/signal/cleanup, and a full `systemctl list-units --all` enumeration of the loaded `d2b*`/`microvm*` namespace that excludes only canonical `d2b.slice` and then requires exactly the three ADR-0015 lifecycle units. Querying only the expected names is ineligible. **Done when** declaration and removal generations are pinned; startup and both public deployments automatically reach production daemon ingestion; T589's typed SC-002 importer accepts the explicit external `0600` input, completes no-replace file-and-directory-durable candidate publication before record publication, and its validator accepts the content-addressed sidecar and exact closed three-resource census at every reopen; the exact three resources and Providers/configs match `spec.md`; every named positive effect/readiness predicate passes; the identical deployment is idempotent; exact Device cleanup and TPM state preservation complete in dependency-safe order; the acceptance Volume/Network, unrelated resources, and durable identities are unchanged; FR-075 continuity passes in full; no implementation ownership or Guest success is claimed; no extra unit exists; all three public gates pass on the same candidate; and host evidence records exact enumeration and successful no-skip builds of both `vmChecks.x86_64-linux.resource-operator-activation` and `vmChecks.x86_64-linux.daemon-restart-vm-survival`. Passing T604 proves only the Wave 5 partial US1 production-plane checkpoint, not full US1 completion.
  **T604 handoff and unit-census correction:** the positive 3/1 fixture starts only from the
  independently accepted and installed exact nonempty 13-member
  `SourceGenerationCompatibilityFloorV1` census. It requires every closed role exactly once,
  one common accepted disposition and source generation, canonical framing/hashes, and an
  authenticated issuer proof at every transition. Its generator visits every one of the 13
  canonical role/artifact pairs for each of `missing`, `duplicate`, `extra`, `empty`,
  `stale-generation`, `stale-digest`, and `cross-disposition`: exactly 91 poison cases before
  fd transfer, authorization, or mutation. The independent
  `role-artifact-matrix.tsv` contains the exact 13 role/artifact pairs and is compared with
  a separately authored literal 13-row test constant plus production, generator, and
  expected-id registries before any poison can count. No expected count is derived from
  production or the fixture. Case ids and mutations are exactly the
  `source-floor/<class>/<role>` registry pinned in T589 above; the independent literal
  `tests/golden/delivery/source-floor-v1/poison-case-ids.txt` fixture contains all 91 and
  calls no generator. Each case keeps vector and
  declared cardinality 13 through the specified one-for-one substitution, then recomputes
  member digest where applicable, manifest/proof/hash, census, installation/proof/hash,
  validation/proof/hash, validated-floor hash, import/proof/hash, and aggregate identity in
  order. Every signature and enclosing hash is valid before the intended semantic refusal;
  overlapping set cases assert their complete error set. A stale enclosing hash, bad
  signature, wrong cardinality, duplicate/missing id, early structural refusal, or unvisited
  role fails the matrix. All 39 missing/stale-digest/cross-disposition cases reach the
  production validator with cardinality 13 and fully recomputed enclosing receipts, and all
  four matrix-meta poisons fail through
  `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`. Separately run all five independent copied-issuer cases for
  manifest, installation,
  validation, import, and all proofs. Each copies both expected authority/key digests, uses
  the right signature domain with an unpinned valid key, recomputes enclosing hashes, and
  preserves unaffected proofs; it must fail only under the disposition-pinned verifier.
  Independently reconstruct all 15 digest and four signature records in accepted Version 2
  `hash-vectors-v1.json`. Run the exact 20 issuer-authentication/capability and 21
  hash-vector negative registries. The validator must produce private
  `AuthenticatedSourceFloorIssuerProvenance` before semantic floor validation and consume it
  into the final private floor result; copied valid signed chains rebound to another C/Q or
  source generation, wrong issuer key, noncanonical encoding, wrong frame/domain,
  schema/golden drift, and direct receipt consumption refuse. Bare committed
  protocol 4 or either source peer advertising
  any other catalogue fingerprint is a separate required refusal, never the positive start.
  The caller-flake target executable
  runs only unprivileged; `sudo` invokes the separately pinned installed apply object, which
  receives no flake URI, installable, reference, target executable, command, or argv to
  reevaluate. Apply has no selector or token. Race two authorization commands and two apply
  connections, inject two pending intents, disconnect before and after the first mutation,
  and retry after completion. Require sole-pending-intent atomic selection, refusal of zero,
  multiple, concurrent, or terminal selection with zero mutation, pre-mutation claim release
  only after durable zero-mutation proof, and post-mutation replay of only the same intent
  through the same pinned apply object. Its accepted connection must prove a direct
  connection-scoped peer pidfd and
  live executable store/NAR/digest match before every mutation; exit, exec, PID reuse,
  start-identity mismatch, executable mismatch, or ambiguity refuses and no pidfd persists.
  Enumerate the exact six transition ids from `quickstart.md` in fresh runs before the first
  mutation. Use that file's independent closed 15-id edge registry. Then allow exactly
  `host-generation.source-bootstrap-publish` and its audit to become durable and inject each
  transition independently immediately before each of the exact fourteen later edges.
  Require all 84 literal `apply-peer/post-first/<edge>/<transition>` ids exactly once,
  refusal before the selected edge, zero selected or successor mutations, preservation of
  the entire durable prefix and first audit, and every exact negative from
  `host-generation-post-first-negative-case-ids.txt`. A separately authored literal
  15-edge test constant, the separate `host-generation-mutation-edge-ids.txt`, and
  production order must all be exactly equal; production never imports either test
  expectation. The three edge meta-poisons and all 15 post-first negatives, including
  verification-hook removal, must fail. Unknown/duplicate/missing/reordered/empty/skip cases and any structural
  failure before the intended negative do not count. The
  `tests/golden/delivery/host-generation-apply-peer-case-ids.txt` also contains the exact six
  pre-first ids and does not derive from production enumeration. Add explicit zero-output and
  multi-output resolution refusals. After
  authorization, independently substitute the target executable and apply object, replace
  and delete/recreate the broker-managed GC root, and retarget the installed symlink; every
  negative refuses before mutation. The acceptance verifies the external source peers and
  atomic source artifact set against their accepted disposition, then pins T592's separate
  target-v5 operation/schema/catalogue/fingerprint/snapshot/fixture set; T592 does not
  regenerate or own the source set. Exactly-one-fd public-socket evidence transfer occurs
  only after source catalogue negotiation and ends in the broker-sealed capability; root,
  provenance, daemon identity, broker credentials, and caller claims never substitute. For
  FR-075, enumerate the complete loaded `d2b*`/`microvm*` namespace, exclude
  exactly canonical `d2b.slice`, and compare the sorted remainder with the required three
  lifecycle units. A nonzero `systemctl list-units --all` result is fatal before filtering; no later
  pipeline stage may convert failed enumeration into an empty or successful census. No other
  slice, target, service, socket, timer, path, or template is filtered.
  The independently authored
  `tests/golden/delivery/host-generation-unit-census-case-ids.txt` contains exactly these 15
  newline-terminated ids in this order:
  `unit-census/positive`, `unit-census/enumeration-error`, `unit-census/empty`,
  `unit-census/missing-d2bd-service`, `unit-census/missing-broker-socket`,
  `unit-census/missing-broker-service`, `unit-census/unexpected-d2b-service`,
  `unit-census/unexpected-d2b-socket`, `unit-census/unexpected-d2b-slice`,
  `unit-census/unexpected-d2b-path`, `unit-census/unexpected-d2b-timer`,
  `unit-census/unexpected-microvm-template`,
  `unit-census/unexpected-microvm-instance`, `unit-census/malformed-row`, and
  `unit-census/skip-marker`. A separately authored literal 15-id test constant must equal
  this file before execution. The test reads that file only as an expected set and may not
  derive it or the literal constant from the production filter or expected-unit array. Each unexpected unit is
  loaded in a separate poison case as exactly `d2b-unexpected.service`,
  `d2b-unexpected.socket`, `d2b-unexpected.slice`, `d2b-unexpected.path`,
  `d2b-unexpected.timer`, `microvm@.service`, or `microvm@unexpected.service`; it survives
  the sole `d2b.slice` exclusion and fails exact equality. Each missing case removes exactly
  its named required unit. Enumeration failure, empty output, a row with no unit-name field,
  and a literal `SKIP` result also fail rather than becoming an empty or ineligible success.
  Every NixOS test setup exception, missing prerequisite, unsupported host result, empty
  discovery, and `SKIP` marker is fatal; none may be translated into a passing or ineligible
  result. All 15 ids must be visited exactly once in the no-skip
  `make test-host-integration` result. T604 is the sole prospective evidence owner for the existing daemon-restart case,
  including `Ready`, `Stopped`, full-namespace exact-three-unit set equality, fresh-pidfd adoption, and
  PID-reuse/mismatch/ambiguity negatives.
  Outside transient verifier-local kernel handles and bytes, raw apply-peer pidfd number,
  numeric PID, start identity, socket uid/gid, cgroup/proc path, executable store path,
  derivation name, NAR identity/hash, content digest, or device/inode/mount identity
  are absent from coordinator state, receipts/evidence, human, JSON, wire, error/`Display`,
  log, tracing event/span, metric name/label/value/exemplar, audit, panic, and every `Debug`
  surface. The exact fifteen-row
  `host-generation-apply-peer-forbidden-values.tsv` fixture from T589 injects one literal per
  class independently into every pre-first and post-first scenario. Only that fixture and the
  test's private injection buffer are excluded from captured-surface scans; no path, prefix,
  process, or free-form allowlist is permitted.   Require only the independently computed fixed-binary process-instance or
  executable-identity correlation digest from `data-model.md` where allowed, with wrong
  domain/framing/endian/order/cross-class negatives. Metrics carry neither raw nor
  digested peer identity in names, help, labels, values, or exemplars. Exact registry
  equality and visit-set equality fail on a missing, duplicate, unknown, changed, or
  unvisited canary.
  T604's fixture and production-boundary legs also pin the exact v4/v2 top-level `audit`
  object, prove that changing only one audit option changes `contentHash` and reaches the
  daemon, and reject 3/1, every mixed version, future 5/2, 4/3, and 5/3, missing/unknown audit
  fields, a Zone resource carrying audit, and any nonempty `Zone.spec`. The host leg starts
  from an installed 3/1 generation carrying the accepted external compatibility floor: its
  source daemon and broker already implement the source-side typed handoff, negotiate numeric
  protocol 4 plus the exact `source-handoff-v1` catalogue fingerprint, and the broker package
  is pinned by the existing service. A source generation without that exact negotiated
  fingerprint must refuse and cannot be the positive control. First execute the documented parameterized bootstrap with
  validated flake/configuration inputs and prove it does not read the absent stable reference.
  Reject empty, malformed, over-bound, mismatched, or nonexistent target parameters before
  public-socket authorization or `sudo`. Resolve one exact executable store object
  unprivileged and authorize that exact target object. Separately resolve and pin the
  installed apply object from trusted installed-generation metadata, then prove privileged
  apply invokes only that object with no Nix reevaluation. Authorize target executable A,
  substitute target executable B, substitute the apply object, and change the installed
  symlink before later apply; every substitution must refuse before mutation while A remains
  eligible. Force apply-peer exit, exec, numeric PID reuse, executable mismatch, and ambiguous
  identity and require refusal before mutation with no persistent pidfd. Require initial
  Admin evidence through the existing public socket, transfer of that accepted-socket evidence
  only after exact source catalogue negotiation, consumption into the sealed durable handoff
  capability, broker-managed target and apply pins, and transition-intent/coordinator
  durability before mutation. Require the
  existing `d2b-priv-broker.service` to start and restart its installed source broker before
  transfer, no authority token in
  output/argv/environment, no profile/service/rollback mutation in the entrypoint, and
  capability-authorized broker-only audited stock
  profile publication and broker/daemon service transition; target broker activation before
  target daemon activation; target daemon Hello while unready; the authenticated
  phase-attenuated authenticated publication request; durable broker publication of the
  pointer and stable reference; and only then ingestion/readiness. Then execute the exact
  stable-reference authorization/apply pair for an identical deployment and for declaration
  removal. Also execute the parameterized prior-target rollback authorization/apply pair and
  verify broker-coordinator recovery after killing the deployment entrypoint during a failed
  transition.

  Inject crashes before and after compile/stage, executable pin and GC-root durability,
  transition-intent file and directory sync, compatibility-broker adoption, coordinator
  durability, stock profile publication, broker service transition, both sides of durable
  compatibility-to-target-broker ownership transfer,
  daemon service transition, daemon Hello, phase-attenuated authenticated publication request, reference
  temporary-file sync, rename and directory sync, pointer publication, readiness
  acknowledgement, rollback preparation, prior pointer/reference restoration, stock rollback,
  and source-service restoration. At every post-staging point kill the deployment entrypoint.
  Before transfer, kill both entrypoint and compatibility process in turn and prove the
  existing `d2b-priv-broker.service` restarts only the matching pinned owner, which resumes or
  rolls back. After transfer, restart the same service and prove the target broker owner
  autonomously resumes or rolls back without a live daemon, a new supervising unit, or a
  surviving entrypoint.
  Inject target broker startup failure, target daemon startup/reconciliation failure,
  source/target broker-generation mismatch, protocol downgrade, missing capability, skipped
  Hello, wrong catalogue digest, daemon-generation mismatch, broker/daemon restart in each
  ordering, direct reference mutation, and deployment-entrypoint crash. Replay or rollback
  must always leave matching system/module/compiler/broker/daemon/pointer/bundle/reference
  generations, with the previous reference bytes or verified absence restored before source
  services return. Each profile/service/bootstrap/rollback mutation requires immutable broker
  pre-mutation and outcome audit; each pointer/reference publish, repair, rollback, replay,
  and refusal requires its immutable broker audit row. Exactly one logical ingestion/effect
  and no temporary, handoff, or reference residue may remain.

  Broker authorization tests accept only an exact phase attenuation of the sealed durable
  handoff capability created from the transferred accepted-socket Admin evidence. They deny a missing,
  forged, copied, replayed, wrong-intent, wrong-phase, or cross-generation capability;
  AdminUid on the broker socket, LauncherUid, RootUid, HostShutdown, NotAuthorized,
  caller-claimed daemon, daemon uid/gid/generation without the capability, remote initiation,
  euid0-only bootstrap, and request-before-Hello.
  Static and executable tests verify that the deployment entrypoint can only
  validate/build/stage/authorize/submit and that only the capability-authorized externally
  installed source broker before transfer and T592's target broker after transfer can perform
  stock profile/service/bootstrap/rollback mutation or publish/repair d2b pointer and stable
  reference state. Only their broker-owned coordinator can initiate
  rollback resumption; the daemon, entrypoint, and Nix activation cannot. A 4/2 daemon
  presented with 3/1 returns only the identifier-free `rebuild-host-generation` action.
  Empty, malformed, missing, unreadable, or changed reference values refuse, and
  host/flake/configuration values remain absent from runtime errors and captured diagnostics.
  Put distinct secret, credential, host, store-path, and arbitrary text canaries in Nix
  evaluator and builder stderr. Exact human/JSON/wire errors, logs, audit, metrics, spans,
  and every `Debug` must contain only the fixed error class/remediation and none of the raw
  stderr canaries.
- [ ] T596 [P] [US1] **Add authenticated publication, watch, readiness, and Zone-isolation acceptance coverage.** Depends on T595. Sole owned file: new `packages/d2bd/tests/resource_plane_authenticated.rs`. Enter through the production daemon Unix session boundary, registrar, ZoneBus route, ResourceService, store, and controller endpoint. Consume T605's contract evidence and cover authoritative same-Zone Get/List/Watch, cross-Zone denial and audit, caller-supplied subject rejection, consumed-admission reuse, partial-readiness non-publication, exact `Provider/system-core` registration ownership, and an actual `Zone.status.handlers[]` list containing exactly one `system-core-host` and one `system-core-user` record with `phase` and `lastReconciledAt`, backed by active, initialized, current handlers. Prove ComponentSession admission is bound to the accepted peer's live pidfd and expected generation/cgroup evidence; after daemon restart require a newly opened pidfd for the rediscovered peer. Reject numeric-PID-only admission, stale evidence after numeric PID reuse, start-time/generation/cgroup mismatch, dead peer/`ESRCH`, and multiple plausible peers. Reject duplicate, missing, underscore/wrong-name required records and `provider-lifecycle` substitution. Run the three-Zone open/close matrix with failures in the first and middle positions; remove the Provider registration and each required list record in turn and prove only that Zone degrades. No Wave 6 dossier is required. The test must assert every Zone was visited and later healthy Zones remain operable. Direct service calls, `ProductionWatchHarness`, fake endpoints, status-only Provider substitutes, and readiness mutation helpers are forbidden in this file. **Done when** all cases pass against production owners, fresh-pidfd and every PID-reuse/mismatch/`ESRCH`/ambiguity negative pass, the emitted list shape matches T605, and removing or corrupting any required readiness owner makes the affected Zone return its specific actionable refusal.
- [ ] T597 [P] [US1] **Add restart effect-replay and cleanup-revision acceptance coverage.** Depends on T595. Sole owned files: new `packages/d2bd/tests/resource_plane_restart.rs` and new `packages/d2b-core-controller/tests/effect_replay.rs`. Crash after generation commit, after ledger durability, after effect dispatch, after adoption, and before completion; reopen through the broker-owned store path and assert each outstanding effect is replayed or adopted exactly once. Exercise pending cleanup across restart and reject zero, stale, wrong-UID, wrong-controller-generation, and ambiguous completion without changing durable state. **Done when** the matrix observes zero lost intents, zero duplicate logical effects, and adopt-before-cleanup ordering in every case.
- [ ] T598 [P] [US1] **Add authoritative audit, pending-result, replay-binding, retention, and redaction acceptance coverage.** Depends on T595. Sole owned file: new `packages/d2bd/tests/resource_plane_audit.rs`. Mutate through the authenticated production Resource API, including a multi-mutation batch; crash at every mutation/journal commit, segment append, file sync, directory sync, export-completion, rotation, journal-prune, and segment-prune boundary; reopen; and compare immutable authoritative journal rows with exported logical records by fixed operation digest plus mutation ordinal. Include sink unavailable, disabled callback, incomplete export, hash-chain mismatch, duplicate replay, record oversize, invalid/default/boundary audit configuration, post-export journal retention, early-journal-prune refusal, and prune/sync-failure typed-health negatives. Prove the journal row commits transactionally with the privileged mutation before any effect is success-shaped; segment export and its completion cursor are separate and cannot rewrite or delete an unexported row; an exported row becomes deletion-eligible only after durable completion plus `audit.retentionDays`. After committed export-pending state, require `CommittedPendingAudit` through T589's `PendingAuditStatus` protobuf field, including `DeleteResponse` and batch ordinals, with the exact canonical `ResourceStatus` composite and no ordinary success or rollback claim. Inspect the same operation through T589's typed ResourceService method and T592 durable backend before and after restart only with an exact replay-binding match to the original registrar-derived subject, Zone, canonical semantic request, target, verb, expected revision, and idempotency data; prove cross-subject, cross-Zone, altered-request/target/verb/revision/idempotency, and restart mismatches are denied and audited without observation or reapplication. Retry a different ID and prove normal revision/conflict behavior. Inject distinct raw operation, correlation, subject, Zone, resource, and trace canaries; require only typed domain-separated fixed digests in journal rows, audit segments, and exports, and require no raw canary in errors, logs, metrics, spans, or redacted `Debug`. **Done when** every committed privileged mutation has an immutable authoritative row at commit, ordinary success waits for segment file and directory durability plus completion durability, multi-mutation restart yields exactly one export per ordinal, same-ID apply count is one, all replay-binding mismatches deny, the exact composite round-trips through every mutation response and `InspectOperation`, all raw canaries remain absent, fixed-digest constructor and record-size limits hold, configured segment and journal retention limits prune correctly, every prune/sync failure degrades health, status observability is stable across restart, and every audit/export failure leaves the affected Zone unpublished with an actionable typed refusal.
  The redaction matrix must enter through every migrated producer named by T592 and through
  T592's broker drain request. It covers valid-present, absent, and malformed trace context:
  present yields only the typed trace digest, absent stays absent, and malformed refuses
  before mutation, with no fabrication or cross-class relabel. Distinct root-path and opaque
  storage-handle canaries must also remain absent from fixed `Debug` output for every
  sensitive DTO, error, `SegmentWriter`, sink, exporter, directory owner, and broker owner.
- [ ] T599 [P] [US1] **Reconcile Wave 5 CLI and reference promises with emitted behavior (FR-019, FR-074).** Depends on T595. Sole owned files: `packages/d2b/src/{dispatch.rs,resource.rs,context.rs}`, `packages/d2b-contracts/src/cli_output.rs`, accepted normative specification `docs/specs/ADR-046-cli-and-operations.md`, `docs/reference/{zone-cli-contract.md,desktop-wrapper.md,companion-contracts.md,cli-contract.md,components-audio.md,components-usbip.md,components-usb-security-key.md,resource-client.md}`, `packages/d2b-contract-tests/tests/{policy_cli_consumers.rs,policy_docs.rs}`, focused CLI DTO/schema tests in the owning crates, and task-local `changelog.d/cli-operation-recovery.md` for T220 to fold. Implement the recovery contract in `contracts/operator-cli.md` only through T589's typed store/ResourceService request and response, T590 authorization, T593 method catalogue/router, and T595 daemon/client path; an in-memory map or CLI-only synthesized result is forbidden. Every mutating generic and typed resource verb accepts `--operation-id <OPAQUE_ID>`; the ID is exactly 16 bytes rendered as lowercase 32-hex; an initial call emits it; an exact retry reuses the original operation/idempotency binding; and `d2b op inspect --operation-id <OPAQUE_ID> [--zone <ZONE>] [--watch]` remains the accepted status command rather than creating a competing command. Own the versioned operation-recovery DTO in `cli_output.rs` and its generated `JsonSchema` checks. Bump accepted `ADR-046-cli-and-operations` from Version 1 to Version 2 and coordinate a deliberate breaking amendment: assign pending exit 75 and replay-mismatch exit 76 for resource mutations/inspection, retain the existing meanings for unrelated exec commands, require `zoneRef` and `schemaVersion: 2` in every recovery success/error JSON envelope, add the exact closed remediation-action enum, and update the stable error-class and exit tables. Migration guidance must tell Version 1 consumers to require `schemaVersion`, upgrade parsing before using recovery, treat a missing or `1` version as the old 0/1/2 contract, and never reinterpret or silently migrate an arbitrary Version 1 operation ID; the v3 clean cutover has no persisted Version 1 recovery-state import. Human and JSON remediation may contain only a closed action such as `inspect-operation`, `retry-identical-operation`, `start-new-operation`, `wait-for-audit-export`, or `verify-operation-context`; it must never embed Zone or operation IDs in executable text, argv arrays, shell fragments, or free-form remediation. Raw Zone and operation ID appear only in their bounded `zoneRef` and `operationId` status fields. Pin mutation and inspection exits plus exact human/JSON pending/final/not-found/refusal shapes, mandatory envelope fields, DTO schema, ID bounds, action enum, and absence of executable remediation vectors. Compare exact `d2b --help`, subcommand help, JSON output, capability keys, typed refusals, and public wire fields. Resource status documentation must expose committed-pending-audit through T589's additive protobuf status field and the exact `ResourceStatus.phase`, `outcome.code`, `update.state`, and `update.operation_id` composite; never claim success or rollback. Reconcile every downstream status consumer owned by this task with T605's paired contract and T595's emitted `Zone.status.handlers[]`: system-core readiness is attributed to `Provider/system-core` plus exactly one `system-core-host` and one `system-core-user`; underscore labels and `provider-lifecycle` cannot substitute. Candidate absence of a command or field is a defect, not permission to delete its promise, unless the same change follows the explicit parity or FR-042 retirement path with replacement, migration guidance, owner, release treatment, and contract tests. Do not add a fallback or claim companion verification. **Done when** every documented desktop-wrapper, companion, audio, USB, security-key, and resource operation is present in emitted behavior or has an approved parity/retirement record; operation inspection reaches the durable backend; pending-audit recovery matches the Version 2 amendment; exact tests cover Version 1 migration refusal, required `zoneRef`/`schemaVersion`, IDs, exits, all remediation actions, and no Zone/ID-bearing argv or executable remediation; T595's emitter and all T599-owned consumers match T605's exact names and non-substitution rule; and focused docs/DTO/schema/contract checks are clean. T220 reconciles the accepted-spec version into generated manifests, verifies paired references/tests/schema and release treatment, folds the fragment, and runs the full drift gate.
  Preserve the accepted `op inspect` controls as
  `[--watch] [--deadline <DURATION> | --no-deadline]`: test each flag, their mutual-exclusion
  refusal, default-deadline behavior, and signal cancellation with no deadline. Human recovery
  narrows the preceding shared-remediation clause: JSON alone carries a closed action. Human
  mode instead
  renders the exact safe static `d2b op inspect` guidance from `contracts/operator-cli.md`
  without flags, identifiers, argv, or shell text; machine output retains only the closed
  remediation-action enum and never gains a free-form guidance field.
- [ ] T220 [US1] `adr046w5` CONVERGE + PHASE-PANEL + FREEZE - depends on T596-T599 and T604. Before exact-candidate evidence, merge every slice branch into the wave integration branch. Reconcile the accepted-spec version changes owned by T589 (`resource-api-and-authorization` only), T592 (`resource-store-redb` and `telemetry-audit-and-support`), T593 (`componentsession-and-bus`), T595 (`nix-configuration`), T599 (`cli-and-operations`), and T605 (both system-core governing specs) into the integrator-owned generated spec manifests. Separately revalidate that the externally owned accepted `ADR-046-validation-and-delivery` Version 2 amendment, both required approvals, Gate 0 receipt, and regenerated spec-set/work-item/implementation-graph artifacts were already complete on an ancestor of T589's base and remain byte-consistent; T220 must not defer, perform, or claim that pre-T589 transition. Verify each amendment's paired reference, contract/API/DTO test, generated schema or explicit no-schema-impact proof, and migration guidance where applicable. Require the exact fourteen-row owner/path fragment set declared under this wave's dependency map, including T603's mandatory `changelog.d/delivery-resume-reconciliation.md`; a missing, duplicate, differently named, or cross-owned path fails closed. Fold exactly those fourteen fragments only after the amendment matrix is complete; T589's existing `resource-api-production.md` fragment must include the SC-002 incident recovery surface without adding a fifteenth fragment. Verify T605's API snapshots include T593's sealed registrar surface, and verify T595's emitter and T599's consumers against T605. Run T589's hermetic `adr046w5` evidence-profile suite and require the exact eight-record positive plus missing, extra, duplicate, unknown, wrong-lane, and conflated negatives to prove panel-request/panel-attest, seal, and merge-eligibility all invoke the same validator. Run the separately versioned typed SC-002 receipt suite and prove the same validator is invoked at import, durable reopen, panel-request/panel-attest, seal, and merge-eligibility; require the exact closed three-resource positive, exact-size boundary success, all 61 independently pinned receipt negatives, and all 45 independently pinned malformed census negatives. Pin all five SC-002 incident commands in parser/catalogue/help, exact thirteen-line human and distinct 17-field version-1 CLI JSON goldens, stable IDs, every closed cause, closed exits `0|2|3|4`, the 19-field durable status and separate resolution schemas, the five-value deterministic remediation table for both recovery variants and authenticated mismatch retention, `parked`, `mismatch-retained`, and frozen-primary-evidence resolution terminal branches, the complete retired and recursively enumerated primary-evidence census grammars/vectors, the identity-bearing bounded-failure commitment, the shared nineteen-digest/one-signature SC-002 oracle, four incident-id vectors, exact 22-field `Sc002IncidentDispositionV1` schema, canonical successor-freeze/request/signed-disposition goldens, pre-signing successor triplet and exact apply/admit reuse, Version 2 contract/authority/key binding, private by-value validator, malformed/noncanonical/unsigned/wrong-domain/tamper/replay/copied-commitment/post-resolution-mutation/post-signing-successor-substitution negatives, incident-candidate denial, fresh successor admission, retained-request byte identity, and no binding-request/reservation-release/evidence-copy/unlink behavior. Then, after W4's reported seal and merge are externally confirmed or corrected, require the
untouched external Network sole-opt-in contradiction to be resolved by the versioned
correction/migration and exact four-case evidence required by T070/T071, or by an
authoritative external disposition preserving sole Network opt-in and leaving double opt-in
prospectively unimplemented. No local matrix, checked status, or Wave 5 acceptance result
unblocks T220. Only then rebase the Wave 5 integration branch onto the updated `v3` and record
that exact `v3` commit as the panel-base eligibility proof. Run integration tests, full `make test-drift`, and CI on the converged tree, and resolve every content-changing result. Open or update one PR against `v3` and identify a clean provisional candidate. Run the nonbinding `/d2b-panel-round plan` phase surface against its exact commit/tree, implementation base, and feature snapshot. This process review creates no delivery `panel-request.json` and no binding reservation. A finding routes only its scoped fix through T220, reruns validation, and requires a delta/full-context phase-plan round against the replacement provisional candidate; iterate until all ten roles sign off with zero recommendations. Only then freeze that clean HEAD and tree as final F and retain its unanimous phase receipt. T220 MUST NOT invoke a binding `/d2b-panel-round work` request, panel-attest, or seal. After F is frozen, any content change or rebase invalidates F and requires T220, the phase review, and T600-T602 to rerun. Their completion still does not authorize T219; the accepted external disposition of Wave 5's retained binding request remains mandatory.
  **T220 incident-contract correction:** T220 pins distinct resumable and irreconcilable
  states, the exact 22-field disposition, complete persisted structured
  preimage/anchor/metadata/status/resolution
  contract with every kind-specific component and immutable preimage path, typed canonical
  receipt locator, payload-file and all-ancestor durability, the
  separate resolution-evidence object and resolution branch, complete frozen
  recursively enumerated primary-evidence census plus identity-bearing bounded-failure
  commitment, pre-signing successor freeze and canonical disposition request, exact
  freeze/request/triplet reuse at apply and admit, and command
  convergence for zero-name, retired-source, anchor/metadata-conflict, status-conflict,
  invalid-census, and unstable-census fixtures. The primary scope recursively binds every
  descendant plus the canonical failure-path digest and excludes
  resolution/request/disposition/freeze leaves; raw `01ff`, copied commitments, and
  post-resolution primary mutations block admission.
  T220 also revalidates the external Version 2 ownership split: that amendment owns the
  source-floor canonical JSON/digest/domain/framing policy, strict schemas, and checked-in
  golden vectors, while the separately named compatibility authorities own only production,
  installation, validation, and import of conforming objects. Any schema/vector emitted or
  silently redefined by the compatibility implementation instead of the accepted amendment
  blocks convergence. For SC-002, exact human goldens are the ordered thirteen-line projection
  in `data-model.md`; summary prose or a variable command rendering is not a golden.
  The SC-002 suite also proves retained schema-v2 `EvidenceRecord` fixtures decode
  byte-identically. It must recompute the exact four kind-specific records in
  `tests/golden/delivery/sc002-incident-id-v1.json`; independently encode its
  normal-empty, normal-sorted-mixed, and exact `01ff` retired-census vectors; consume the
  shared nineteen-digest/one-signature SC-002 oracle; reject census version,
  body, framing, ordering, tag, unavailable-tuple, partial-sentinel, kind-domain,
  tuple-order, stage, census-evidence, durable/CLI schema, cause/remediation derivation, and
  status-kind mismatches; and prove each immutable incident metadata object reconstructs its
  complete preimage/id/path and every incident has exactly one valid payload, residue, or
  canonical complete/bounded-failure primary-evidence commitment plus one contiguous append-only primary or resolution
  branch. A failed operator record imports without a receipt but cannot satisfy the
  closed profile or any later stage, and a passed record requires exactly one matching
  receipt at canonical candidate-relative locator
  `evidence-sidecars/sc002/sha256/<typed-digest>.json`, where the digest is the shared
  `activation-receipt-content` vector and never a raw hash. The import suite supplies the external
  receipt only through `--sc002-receipt`, rejects caller-supplied `--locator`, checks exact
  current-effective-uid ownership plus source/destination mode `0600` and candidate-directory
  mode `0700`, hashes before decode, and proves create-exclusive temp, file `fsync`,
  no-replace publication, bottom-up `fsync` of every ancestor directory from `sha256` through
  the candidate directory, one candidate-scoped exclusive OFD lock shared by importer and
  cleanup, one stable never-replaced lock inode, restart cleanup only after lock acquisition,
  and live-owner `EAGAIN|EACCES` refusal before namespace access. Verified orphans use
  identity-preserving quarantine/reopen/no-replace retirement into
  `retired/sha256/<content-digest>/<retirement-id>.bin`, never per-leaf unlink, with
  payload/parent/all-changed-ancestor sync and empty ephemeral namespaces before ordinary
  record publication. Two
  same-byte distinct-inode orphans receive distinct ids. Forced destination collision,
  65th-leaf, 1,048,577-byte, malformed-census, unauthorized-retention-owner, candidate-root
  removal, permanent-history mutation, and failed whole-scope-retention fixtures preserve
  data, transition to incident where specified, and refuse. T589's private owner performs
  only the zero-mutation terminal/reference/lock/ephemeral/bounded-durable whole-scope
  retention guard. Identity ambiguity has three terminal shapes: preimage/path-complete
  metadata plus a fully revalidated and file-synced payload and `parked`; authenticated
  no-unlink durable residue retention plus `mismatch-retained`; or an authenticated complete
  census or identity-bearing bounded-failure frozen-primary-evidence resolution when zero
  names, anchor/metadata conflict, conflicting primary status, or an invalid/unstable census makes
  the primary branch irreconcilable. A rename/reopen race remains
  inspectable as exactly `recovery-resumable` or `recovery-irreconcilable`, preserves all
  names, carries the same stable id plus closed cause/remediation, and blocks publication and
  close until its advertised command reaches terminal disposition or successor. Retired
  source locators and branch-aware status repair are explicit fixtures. Crash injection
  covers every retirement, incident preimage/anchor/metadata/payload/payload-file-sync/
  residue/status/resolution-evidence/resolution, successor-freeze, disposition-request,
  disposition, recursive census-encoding, status-projection, and whole-scope-retention boundary.
  Every importer, cleanup, incident-recover, incident-apply, successor-admit, and
  retention-guard overlap with cleanup, plus every writer/retention pair,
  uses independently opened descriptions and both owner
  orderings at temp creation, file sync, quarantine move, retirement move, incident metadata
  publication, payload move/file sync, residue stage/finalize, base-status publication, and
  resolution publication. Every
  live-owner loser has zero namespace opens/mutations and `critical_section_max = 1`; the
  sole post-release retry opens fresh fds and recensors under lock. Replacement runs on both sides
  of every rename/reopen; synchronized same-input retry
  and different-byte or wrong-binding races prove bounded completion, no deadlock, no
  sidecar-data unlink, and exact final census. Ordinary terminals leave empty ephemeral
  namespaces and no more than 64 retired leaves or 1,048,576 retired bytes; ambiguous,
  collision, overflow, and malformed-census terminals retain incident residue. The suite
  also pins `command.rs`
  synopsis, catalogue, dispatch, and help exposure for `--sc002-receipt` and all five
  incident commands. The validator must hash the same opened
  canonical bytes with the typed domain-separated, length-framed receipt hash before decode
  at every stage, match the actual
  `candidate_id`/`content_id`/`snapshot_sha256` triplet, reject
  absolute/traversal/URL/symlink/hard-link and replacement-race fixtures, and reject a receipt
  on a failed record.
  T220's coordinated matrix explicitly includes the resource-bundle 4/2 emitter and Rust
  consumer, required top-level audit schema, canonical `{audit,resources}` digest reference,
  generated `resource-bundle.json` schema, old/mixed/future 5/2, 4/3, and 5/3 plus
  missing-placement negatives, audit-only generation test, and the Type-1 rebuild-reference
  grammar/boundary/refusal case after `make nix-unit-pin`. It includes the target-closure
  deployment entrypoint; an independently accepted and atomically installed source
  generation with the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1`
  census and authenticated issuer chain. Its poison generator must visit all 13 canonical
  role/artifact pairs for all seven `missing`, `duplicate`, `extra`, `empty`,
  `stale-generation`, `stale-digest`, and `cross-disposition` classes: exactly 91 cases.
  The separately literal 13-row constant, production registry, exact
  `role-artifact-matrix.tsv`, poison generator, and expected ids are mutually
  read-independent and must agree without shared counts. The exact ids are
  `source-floor/<class>/<role>`. Each keeps vector/declared cardinality 13 by the class-specific
  one-for-one substitution and recomputes every enclosing digest/proof in the order pinned
  by `data-model.md`, so all signatures and canonical envelopes are valid before the
  semantic poison is selected; set-overlap cases assert their complete error set. All 39
  missing/stale-digest/cross-disposition cases preserve cardinality and valid enclosing
  receipts, and all four matrix-meta poisons fail through the enforcing fixture-contract
  runner.
  The accepted `hash-vectors-v1.json` exact 15 digest and four signature entries are
  reconstructed from semantic inputs, including domain bytes and fixed-width frames.
  Missing/wrong issuer proof and the exact five copied-issuer cases for manifest,
  installation, validation, import, and all proofs,
  copied signed chain against another C/Q or source generation,
  noncanonical field order/integer/text, duplicate/unknown field, wrong frame/domain, schema,
  and golden-vector negatives also refuse under the exact 20-case authentication/capability,
  21-case vector, and 32-case receipt registries. T589 must invoke the
  disposition-pinned validator, obtain the private nonserializable authenticated-issuer
  result, and consume it by value into the private validated-floor result; direct receipt
  decode and copied authority/key digests are ineligible. Also
  require refusal of bare
  protocol 4 or a mismatched source-peer fingerprint; that external source set remains distinct from
  T592-owned target-v5 operation/schema/catalogue/fingerprint/snapshot/fixture outputs; exact target-object,
  GC-root, and installed-apply-object pins; caller-flake execution only while unprivileged;
  privileged no-URI/no-reference reevaluation; zero-output and multi-output resolution
  refusal; target/apply executable, GC-root, and symlink substitution refusal; apply-peer
  direct pidfd/executable binding plus the six-transition cross-product before the first
  mutation and, after exactly the first mutation and audit are durable, before each of the
  fourteen later ids in the independent closed 15-edge registry. All six pre-first and 84
  literal post-first ids must occur exactly once, the separately literal 15-id constant and
  fixture must match production order, all three edge meta-poisons must fail, and all 13
  post-first negative ids must reach their intended checks;
  zero selected and successor mutations follow refusal, no pidfd persists, and the durable
  prefix plus first audit is unchanged. The exact fifteen-row forbidden-value registry injects
  pidfd/PID/start/socket-uid/socket-gid/cgroup/proc-path/store-path/derivation/NAR-identity/
  NAR-hash/executable-content/device/inode/mount canaries
  independently. Every literal is absent from coordinator state, receipt/evidence, human, JSON,
  wire, error/`Display`, log, tracing event/span, metric name/label/value/exemplar, audit,
  panic, and `Debug`; only the fixture and private injection buffer are scan exclusions.
  Only the exact canonical process-instance and executable-identity correlation digests are
  retained where allowed, and metrics
  carry no raw or digested peer identity; production-only
  16,384-byte in-memory Nix stderr ceiling with overflow refusal, `/dev/null` in runnable
  documentation, and fixed-error canaries;
  validation/build/stage/public-socket-authorization/opaque-request-only entrypoint;
  exactly-one-fd accepted-socket authorization evidence transfer only after exact source
  catalogue negotiation; capability-authorized installed-source-broker-before-transfer and
  target-broker-after-transfer-only profile, service, 3/1 bootstrap, and rollback mutation;
  existing-unit pre-transfer start/restart ownership; target
  broker-before-daemon ordering; target daemon
  exact-generation protocol-5 Hello while unready; public-socket Admin authorization and
  sealed durable handoff capability; phase-attenuated authenticated publication request;
  broker-durable d2b pointer/reference publication, audit, repair, and prior
  bytes-or-absence restoration; external source-artifact atomicity plus T592 target-v5
  generated wire/privileges schemas, catalogue, reference, parity/policy tests, drift test,
  and standalone broker lockfile; every identity/authz/direct
  bypass negative; broker-coordinator ownership before first mutation, durable
  compatibility-to-target-broker ownership transfer, broker and daemon startup-failure
  rollback, entrypoint death, and every compatibility-broker crash boundary through only the
  existing broker unit; closed
  identifier-free `rebuild-host-generation` runtime action;
  parameterized fail-closed migration/stable-reference/rollback authorization/apply commands;
  Nix/daemon wiring;
  and changelog fragment. It also runs T589's strict
  binding positive, point-specific reservation durability oracle, fd-relative orphan cleanup,
  synchronized cross-candidate first-request race, crash transitions through panel-request
  publication and terminal disposition, same-candidate second-request,
  alternate-candidate request, reservation-release, successor-admission, and post-request
  history-only-rebase negatives at panel, seal, and merge-eligibility. Generic pre-request
  history-proof tests remain green, nonbinding plan-phase rounds create no reservation, and
  no terminal binding result permits a second request. Seed the retained Wave 5 consumed
  request and complete delivery directory, run unanimous and finding-plus-rerun nonbinding
  phase sequences, and require byte-identical delivery state with no reservation, request,
  request-disposition, or candidate mutation. The pidfd quarantine source policy, including
  the forbidden `nix` `PeerPidfd` poison, must fail for the intended reasons through its sole
  enforcing runner `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`; T220 must also
  prove `make test-policy` does not duplicate that runner. Both ancillary receive paths must
  pass `MSG_CMSG_CLOEXEC`, exact-one-fd, truncation, excess/error-fd closure, descriptor-count,
  and exec-leak tests.
- [ ] T600 [US1] **Capture exact-candidate production-boundary evidence.** Depends on T220 and runs read-only after F is frozen. Owns no repository files; import delivery evidence records only. T600 exclusively owns these five closed `EvidenceRecord.validation` identifiers: `production-session-watch`, `effect-replay-cleanup`, `audit-drain-replay`, `system-core-handler-contract`, and `operator-nix-activation-cleanup`. Run the authenticated same-Zone/cross-Zone watch matrix through T593 and T592's typed `OpenPeerPidfdFromAcceptedSocket` broker operation, with accepted-socket and pidfd `SCM_RIGHTS`, a safe dependency or the approved broker `sys.rs` FFI quarantine, one private registrar issuer, a fresh restart peer pidfd, and unsupported/missing-or-extra-fd/dead/numeric-only/reuse/credential/generation/cgroup/ambiguity refusals. Both ancillary receive paths must set `MSG_CMSG_CLOEXEC`, reject truncation, own all received fds immediately, require exactly one expected fd, close all excess/error-path fds, and pass descriptor-count plus exec-leak tests. Prove no repository-authored unsafe exists outside the approved quarantine, every quarantined unsafe block has an immediate `SAFETY:` justification, and the single enforcing `make test-fixture-contracts` pidfd policy rejects the forbidden `nix` `PeerPidfd` wrapper without a duplicate `make test-policy` runner. Import T605's final API-surface and compile-fail seals proving public bootstrap/peer issuance and evidence access are absent. Run restart crash-window/effect replay and cleanup stale/zero/wrong-UID negatives. Run the authoritative journal/export matrix at every commit, append, file-sync, directory-sync, completion, rotation, journal-prune, and segment-prune boundary, including multi-mutation ordinals; typed durable `InspectOperation`; replay-binding cross-subject/Zone/request/restart denials; fixed-digest constructor and record-size limits; post-export journal and segment retention; prune/sync health; raw identifier/trace canaries; the exact protobuf-represented `ResourceStatus` pending composite; and same-ID/different-ID behavior. Also run the exact `Provider/system-core` registration/handler-health matrix, three-Zone startup/close isolation, and T604's public activation-to-effect-and-cleanup result. The operator evidence must show that `make test-host-integration` neither skipped nor discovered an empty set, enumerated and successfully built both `vmChecks.x86_64-linux.resource-operator-activation` and `vmChecks.x86_64-linux.daemon-restart-vm-survival`, and recorded no `SKIP` result. For `operator-nix-activation-cleanup`, T600 MUST pass T604's exact external current-effective-uid `0600` receipt file through T589's `wave validate-import --sc002-receipt PATH` and MUST NOT supply `--locator`. T589's importer once-opens and computes the typed digest before decode, derives `evidence-sidecars/sc002/sha256/<typed-digest>.json`, installs the exact bytes beneath the held candidate dirfd as a current-effective-uid `0600` leaf under current-effective-uid `0700` directories with create-exclusive temp, file `fsync`, `renameat2(RENAME_NOREPLACE)`, and destination-directory `fsync`, while one verified
candidate-scoped exclusive OFD lock is shared by every importer and cleanup worker and
excludes every same-leaf or different-leaf live-owner cleanup, incident command, and
retention check before namespace access through the fixed stable lock inode. The receipt
address is the exact typed domain-separated, length-framed hash of canonical receipt bytes.
Verified ordinary orphans use identity-preserving quarantine/reopen/no-replace retirement
into
`evidence-sidecars/sc002/retired/sha256/<content-digest>/<retirement-id>.bin`, never
per-leaf unlink, then sync the leaf and directories and census both ephemeral namespaces
empty before the unchanged schema-v2 `EvidenceRecord` is published. Every new retirement
ancestor is durably precreated and every changed source/destination ancestor is synced
bottom-up. The retirement id binds
the candidate/content/device/inode identity, so two same-byte distinct-inode orphans receive
distinct names. A destination collision, more than 64 leaves, more than 1,048,576 bytes, or
an invalid retired census transitions the source to incident and blocks publication without
overwrite, reuse, or deletion. T589's private candidate-retention owner performs only the
exact zero-mutation terminal whole-scope retention guard; retired leaves and the canonical
candidate root remain retained. Identity ambiguity
moves the metadata-bound currently named suspect no-replace to durable
`evidence-sidecars/sc002/incidents/payload/sha256/<incident-id>.bin`, syncs both parents
and every changed ancestor, reopens and verifies the moved inode, `fsync`s that payload fd,
then append-only publishes and syncs `parked` status. A rename/reopen race is exactly
`recovery-resumable` or `recovery-irreconcilable`, preserves every still-named leaf,
publishes no parked status, and blocks publication and close. Inspect exposes the stable
incident id, closed cause, exact state variant, deterministic remediation, and state in exact
thirteen-line human and JSON projections. Recover accepts no alternate path/identity or
deletion request, is advertised only for a resumable prefix, and returns the same closed
exits `0|2|3|4`. For an irreconcilable cause, authenticated apply retains representable
names through the closed five residue slots and `mismatch-retained`, or binds the complete
frozen primary-evidence census or identity-bearing bounded-failure commitment and publishes
separate resolution `disposition-validated` for absent names, anchor/metadata conflict,
conflicting primary status, or an invalid/unstable census. The frozen scope excludes
resolution leaves and raw `01ff` is non-authorizing. Every branch reaches fresh
successor admission without editing or deleting primary evidence. No SC-002 cleanup
path unlinks a sidecar data leaf. Crash retry may reuse only an identical fully
revalidated durable leaf; different bytes or binding refuse. At every reopen, resolve beneath
the held candidate dirfd and verify the canonical bytes against the typed locator digest before decode
from that fd. Require schema version 1, correct kind, at most 16,384 encoded bytes, exact outer
`candidate_id`/`content_id`/`snapshot_sha256` triplet, one common monotonic start, and exactly
one typed same-identity effect/Ready/selected-stop/progress sample for each of
`Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`. Every effect
and Ready observation must name the same sample resource identity. Every selected stop,
checked elapsed value, 1-32 progress observations, and <=2,000 ms assertion must validate;
missing, malformed, misordered, stale, wrong-record, progress-free, over-budget,
absolute/traversal/URL/symlink/hard-link locator, replacement before quarantine move, before
reopen, or on either side of retirement, live-owner cleanup, retirement collision/census
failure, identical-orphan name collision, candidate-root deletion, permanent-history
mutation, unauthorized retention mutation,
any writer-cleanup, cleanup-writer, cleanup-cleanup, writer-retention, or retention-writer
same/different-input overlap failure, live-owner namespace access, or
`critical_section_max != 1`, or any mismatch against T589's independent
`tests/golden/delivery/sc002-sidecar-lock-case-ids.txt`, any durable
incident without a valid cause/status/residue-or-resolution projection, missing-sample, duplicate-sample, mixed-identity,
effect/Ready-disagreeing, or unrelated-sample evidence is rejected. A failed operator record
imports without a receipt but cannot count among T600's five passing records or satisfy any
close stage; a failed record with a positive receipt is malformed. The same operator record
binds T595's crash-replayable generation handoff; parameterized fail-closed 3/1 compatibility
through the independently accepted source generation whose exact nonempty 13-member
`SourceGenerationCompatibilityFloorV1` census and issuer chain pass through the
disposition-pinned validator's private authenticated-issuer result and then its separate
private validated-floor result. The poison matrix visits all 13
roles for all seven `missing`, `duplicate`, `extra`, `empty`, `stale-generation`,
`stale-digest`, and `cross-disposition` classes with cardinality 13, recomputed enclosing
hashes, valid test signatures, and exact visited count 91. Copied authority digests/proofs,
chain rebinding, noncanonical encoding, wrong frame/domain, and schema/golden drift refuse
without producing `AuthenticatedSourceFloorIssuerProvenance`.
The independent literal 13-row constant, role/artifact fixture, four matrix-meta negatives,
five copied-issuer ids, 20 issuer-authentication/capability ids, 21 hash-vector ids, and exact 15
digest/four signature vectors must also pass before import;
exact target
store-object authorization and broker pin plus a separately pinned installed apply object
followed by no-reevaluation apply only through that object; target/apply/symlink substitution
refusal; apply-connection direct peer-pidfd/executable binding with
the six transition classes refused before the first mutation and, after exactly the first
mutation/audit are durable, at every exact later mutation edge in `quickstart.md`. The
independent `tests/golden/delivery/host-generation-apply-peer-case-ids.txt` expected set
binds a separately literal 15-edge constant, independently pinned 15-edge registry, six
pre-first ids, and exactly 84 `apply-peer/post-first/<edge>/<transition>` ids. All three
edge meta-poisons and 15 post-first negatives must fail. No selected or successor mutation follows refusal, the
durable prefix is unchanged, and no pidfd persists. All fifteen raw apply-peer canaries
from the closed forbidden-value registry are absent from coordinator state,
receipt/evidence, human, JSON, wire, error/`Display`, log, tracing event/span, metric
name/label/value/exemplar, audit, panic, and `Debug`; only the two exact canonical typed
correlation digests are permitted, and no
peer-identity metric label or value;
validation/build/stage/public-socket-authorization/opaque-request-only entrypoint; accepted
public-socket evidence transferred as exactly one fd only after exact source catalogue
negotiation and consumed by the installed source broker into the sealed nonfabricable durable
handoff capability; capability-authorized source-broker-before-transfer/
target-broker-after-transfer-only profile, service, bootstrap, publication, repair, and
rollback mutation with immutable pre-mutation/outcome audit; broker-owned coordinator before
first mutation, existing-service start/restart ownership before transfer, durable
compatibility-to-target-broker ownership transfer, entrypoint and compatibility-process death
recovery, target broker and daemon startup-failure recovery, and compatibility crash
boundaries without daemon recovery ownership or a new unit; broker start, daemon Hello while
unready, phase-attenuated authenticated publication request, durable publication,
ingestion/readiness ordering; daemon-identity/euid0/provenance/caller-claim refusals; runnable
fail-closed migration/stable-reference/rollback authorization/apply commands; fixed Nix
failure output with raw stderr canaries absent; restoration of prior reference bytes or
absence; absence of reference values from diagnostics; matching broker/daemon/pointer
generations; identical-deployment no-duplicate behavior; and FR-075 Ready/Stopped,
fresh-pidfd, PID-reuse/mismatch/ambiguity quarantine, and exact set equality between the full
loaded `d2b*`/`microvm*` namespace and the three required units on F after excluding only
canonical `d2b.slice`, with exact-set execution of T604's independent
`host-generation-unit-census-case-ids.txt` positive, enumeration, empty, missing-required,
unexpected service/socket/slice/path/timer/template/instance, malformed-row, and skip-marker
cases.
Import T605's exact wire round-trip, underscore and duplicate/missing/wrong-name rejection,
`ProviderLifecycle` non-substitution, current API snapshot, paired normative/reference/version
result, targeted contract test, and unchanged desired-schema drift evidence. Every record
must name F, F's tree, and the production entry point. Reject direct
ResourceService/WatchService calls, `ProductionWatchHarness`, fixed/fake endpoints,
constructed subjects, numeric-PID-only identity, status-only Provider/readiness substitutes,
manually set readiness, skipped or empty-discovery host output, evidence from an earlier
commit, an unknown identifier, or a duplicate identifier. **Done when** T600 emits exactly its
five assigned identifiers once each for F; the operator record identifies emitted bundles;
the typed SC-002 content address, triplet, and census pass at every reopen; exact Provider
resources/configs and positive effects/readiness pass for the exact three acceptance
identities; exact Device swtpm/flush cleanup, unresolvable Endpoint, and same-identity TPM
state-Volume preservation pass; the acceptance Volume/Network and unrelated resources remain
ready, identity-stable, and unrecreated; Guest remains explicitly deferred with no Wave 5
Guest-success claim; FR-075 and both exact VM attrs pass; the handler-contract record is
candidate-bound and complete; same-ID audit retry applies once and replay-binding mismatches
deny; durable operation inspection survives restart; no raw identifier, trace canary, host
rebuild reference, or Nix stderr canary escapes; every peer-pidfd/unsafe-boundary/API-seal,
handoff-capability/authz, protocol-generation, executable-substitution, and
privileged-transition-bypass negative passes; file/directory durability and retention/prune
health hold; malformed status fields cannot round-trip; and every command passed.
  **T600 imported-evidence correction:** the SC-002 importer `fsync`s every held ancestor
  directory from `sha256` through the candidate directory before record publication. Every
  no-replace loser and restart cleanup first acquires the same verified candidate-scoped
  exclusive OFD lock on the fixed stable lock inode as the importer and cannot inspect or
  mutate a live importer or cleanup owner's namespace. Same-leaf and different-leaf cleanup
  overlap is serialized by that lock before namespace access; only one post-release retry
  opens fresh fds and recensors. Verified ordinary orphans use identity-preserving
  quarantine/reopen/no-replace durable retirement under distinct candidate/content/inode-
  bound ids plus payload and every changed ancestor sync and leave both ephemeral namespaces empty. The bounded
  retirement census and private zero-mutation whole-scope retention guard validate; two identical
  orphan leaves survive under distinct ids. No sidecar data leaf is unlinked. Identity
  ambiguity, retirement collision, or census failure uses the preimage-complete immutable
  metadata, payload-quarantine, append-only-status protocol and blocks publication and every
  close stage with intentional residue. A raced move is exactly resumable or irreconcilable
  with every still-named leaf preserved. Recover is advertised only for the former; the
  latter uses authenticated residue retention or the frozen-primary-evidence-bound resolution
  branch, including zero-name, retired-source, anchor/metadata-conflict, and status-conflict
  cases, and reaches fresh successor without primary mutation. The handoff evidence validates the exact
  nonempty 13-member `SourceGenerationCompatibilityFloorV1` census separately from T592's
  target-v5 adoption row and target artifacts.
  It proves the caller-flake target executable ran only unprivileged, the exact installed
  broker-managed apply object ran under `sudo` with no URI/reference reevaluation, zero-output
  and multi-output target resolution refused, and target/apply/GC-root/symlink substitutions
  plus the six-transition cross-product refused before the first mutation and after the first
  durable mutation/audit at each of the fourteen exact later mutation edges, with all 84
  literal case ids visited once, the independent 15-edge literal/fixture/production equality,
  all three edge meta-poisons and all 15 post-first negatives, no selected or successor
  mutation, no persisted pidfd, all fifteen raw apply-peer canaries absent from every
  state/output surface, only the two canonical typed correlation digests, and no
  peer-identity metric label or value.
  FR-075 exact set equality is computed only after excluding canonical
  `d2b.slice` from the complete loaded namespace; no other unit is filtered, and injected
  unexpected-slice and unexpected-service cases each fail.
  In T600's opening shorthand, "import delivery evidence records only" excludes repository
  writes, not sidecar ingestion: for the operator validation T600 imports both the
  `EvidenceRecord` fields and T604's explicit receipt input into external candidate delivery
  state through T589's single importer. T600 never writes the candidate sidecar directly.
  `audit-drain-replay` also binds the valid-present/absent/malformed trace-context matrix, typed
  trace-digest/no-fabrication result, every migrated producer and StoreSync request/response
  wire snapshot, fixed redacted broker/audit owner `Debug`, and raw identifier, path, plus
  opaque-handle canary absence. The operator record pins the
  top-level audit carrier, exact 4/2 pair, audit-only generation change, empty `Zone.spec`, and
  rejection of old/mixed/missing or ResourceSpec-carried policy.
- [ ] T601 [US1] **Capture exact-candidate RSS, owner fan-in, removal, and reference evidence.** Depends on T220 and runs read-only in parallel with T600 subject to the heavy-gate limit. Owns no repository files; import delivery evidence records only. T601 exclusively owns these three closed `EvidenceRecord.validation` identifiers: `resource-plane-rss-owner-fanin`, `wave5-removal-proofs`, and `cli-reference-conformance`. Measure the full daemon-owned publication path at 10,000 resources and 100 authenticated watches with no baseline subtraction; prove one store owner, one policy owner, one ResourceService route, one controller endpoint/fan-in, and one authoritative audit journal/export owner per Zone. The `Provider/system-core` registration and handler records belong only to T600's `system-core-handler-contract`. Re-run every manifest-label W5 removal proof at F instead of citing `removal-proof-w5.md`'s historical `a7f4a6a4` snapshot. Compare emitted CLI/help/JSON/wire behavior with all T599 pages, including the accepted Version 2 amendment and migration guidance, exact 16-byte/lowercase-32-hex IDs, same-ID retry and typed durable status command, exits, mandatory `zoneRef`/`schemaVersion: 2`, DTO/schema, human/JSON forms, closed remediation actions, Version 1 non-migration, and absence of any Zone/ID-bearing argv or executable remediation. Do not re-emit T600's operator-activation or handler-contract kinds. **Done when** T601 emits exactly its three assigned identifiers once each for F; RSS is <=24,576 KiB, owner counts are exactly one, all current removal-proof predicates are true, Version 2 docs/DTO/schema/migration/release treatment match emitted behavior, and every record names F and F's tree.
  `cli-reference-conformance` must exercise accepted `op inspect --deadline`,
  `op inspect --no-deadline`, their mutual-exclusion refusal, and cancellation; compare the
  exact identifier-free human `d2b op inspect` guidance separately from the unchanged closed
  JSON action enum.
- [ ] T602 [US1] **`adr046w5` PRODUCTION-PLANE CHECKPOINT CONVERGENCE - mechanically unblock T219.** Depends on T600 and T601. Owns no implementation files and cannot substitute prose inspection for T589's checked-in validators. Revalidate exactly one T072 disposition: checked T072 with its exact contemporaneous receipt, including the first-dispatch Wave 5 plan-panel receipt, or unchecked T072 plus the sole passing `historical-entry-remediation-t072` record originally bound to A/P0; require A to be an ancestor of B, C, and F, and do not check T072 or infer historical plan compliance or implementation completion from the remedial disposition. Verify T603, every T589-T599 task, T604, T605, and T220 are complete, `tasks.md` shows T073-T218 and T603 checked, and T220's latest unanimous nonbinding phase-panel receipt binds exact final F, its implementation base, and the current feature snapshot with zero recommendations. Validate immutable authorization receipt R against opaque project sentinel `7f6d0beab0ce4c13a89f6865d5ac42e2`, Git-discovered root, relative feature path, resume base B, tree B, and pre-edit snapshot P; validate progress receipt E against R's digest, authorized post-edit snapshot Q, dedicated checkbox commit C, exact parent `C^ = B`, and exact 147-token `B..C` diff; require C to be an ancestor of final candidate F. Do not compare R or E to final HEAD as though either were final-candidate evidence. Invoke T589's `adr046w5` closed-evidence profile over the imported records and require its multiset of `(lane, validation)` pairs to equal the `plan.md` table byte-for-byte: exactly the five T600 identifiers and three T601 identifiers, each at its assigned lane and exactly once. Require T220's hermetic evidence that the same validators guard import, durable reopen, panel-request/panel-attest, seal, and merge-eligibility; that missing, extra, duplicate, unknown, wrong-lane, and conflated record fixtures fail; and that SC-002 absent explicit input for a passing record, explicit input on a failed or wrong-validation record, caller-supplied locator, wrong source/destination owner or mode, 16,385-byte input, crash before and after OFD-lock acquisition/file sync/no-replace/directory sync/quarantine/retirement/incident-move/incident-sync/disposition-publication/whole-scope-retention-guard/cleanup-parent sync/ephemeral-and-durable-census/record publication, live-owner cleanup, importer-cleanup and cleanup-cleanup same/different-input overlap, replacement before quarantine move, before reopen, and on both sides of retirement, two identical orphan leaves, forced retirement collision, retirement census overflow/corruption, candidate-root deletion, permanent-history mutation, unauthorized retention mutation, durable incident persistence and close denial, same-name different-byte or wrong-binding races, passed-record-missing-receipt, malformed/unknown version, kind, field, or enum, malformed/noncanonical/unsigned/wrong-contract/wrong-authority/wrong-key/wrong-domain/tampered/replayed incident disposition, misordered, stale, progress-free, over-budget, missing-sample, duplicate-sample, mixed-identity, effect/Ready-disagreeing, and unrelated-sample fixtures fail at import, durable reopen, panel-request/panel-attest, seal, and merge-eligibility. Require all eight records to bind F and F's tree after T220. Reopen `operator-nix-activation-cleanup` through T589's typed receipt validator and require schema version 1, correct kind, <=16,384 bytes, the exact three-resource census, one common transition-intent start, same-identity effect/Ready/selected-stop/progress observations, checked elapsed values <=2,000,000,000 ns, and 1-32 correctly ordered progress events per sample. The same record must prove the exact spec-pinned Providers/configs and owned effect/Ready pair for `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`; exact Device cleanup, unresolvable Endpoint, and TPM-state identity preservation; unchanged ready acceptance Volume/Network and unrelated resources; explicit Guest deferral;
validation/build/stage/public-socket-authorization/request-only entrypoint; accepted external
compatibility-floor identity and exact nonempty 13-member
`SourceGenerationCompatibilityFloorV1` census with every role once under one disposition and
source generation plus `missing`, `duplicate`, `extra`, `empty`, `stale-generation`,
`stale-digest`, and `cross-disposition` poison refusal; initial public-socket Admin evidence transferred as
exactly one fd only after both source peers negotiate that exact fingerprint and consumed by
that installed source broker; sealed durable handoff capability plus separate broker-managed
target-object and installed-apply-object pins; no-reevaluation privileged apply only through
the installed apply object plus target/apply/symlink substitution and apply-peer
exit/exec/PID-reuse/start-identity/executable-identity/ambiguity refusal before the first
mutation and in all 84 literal cases across the fourteen exact later mutation-edge ids,
zero selected or successor mutations after refusal,
no persisted pidfd, all fifteen raw apply-peer canaries absent from every output surface,
only the typed process-instance and executable-identity correlation digests outside metrics,
and no peer-identity metric label;
capability-authorized installed-source-broker-before-transfer and
target-broker-after-transfer-only audited profile/service/bootstrap/publication/rollback;
staged-intent and pre-mutation coordinator
durability; existing broker-service start/restart ownership before transfer; exact durable
ownership transfer; entrypoint and compatibility-process death replay; Hello-unready,
phase-attenuated authenticated publish, durable publication, then ingestion/readiness;
fixed-error Nix stderr redaction canaries; broker-coordinator rollback resumption after
entrypoint crash; parameterized fail-closed migration/rollback coverage; and FR-075
Ready/Stopped, fresh-pidfd, PID-reuse/mismatch/ambiguity quarantine, full loaded
`d2b*`/`microvm*` namespace enumeration with exact-three-unit set equality after excluding
only canonical `d2b.slice`, including unexpected-slice and unexpected-service negatives,
no-skip continuity. Require T592/T593 peer admission through
`OpenPeerPidfdFromAcceptedSocket` using a safe dependency or the approved broker FFI
quarantine, including unsupported/dead/reuse/credential/generation/cgroup/ambiguity
negatives, `MSG_CMSG_CLOEXEC` on both receive paths, truncation and exact-one-fd checks,
closure of every excess/error fd, descriptor-count and exec-leak tests, no repository unsafe
outside that quarantine, per-block `SAFETY:`, and no new FFI crate or session fallback.
Require the one enforcing `make test-fixture-contracts` pidfd policy and its forbidden
`nix` `PeerPidfd` poison, with no duplicate `make test-policy` runner. Require T605/T595/T599
coordinated contract evidence only under `system-core-handler-contract` and no ineligible
direct/fake boundary. Require HEAD exactly equals F, `git diff --cached --exit-code` and
`git diff --exit-code` are empty, and
`git status --porcelain=v1 --untracked-files=all` reports no
staged, unstaged, or non-ignored untracked path. RSS and owner counts; sealed registrar API;
exact system-core registration and unique handler records; transactional authoritative
audit; fd-anchored durability and retention health; fixed-digest/redacted-`Debug` seals;
protobuf pending status; durable `InspectOperation`; same-ID no-reapply and replay-binding
denials; current removal predicates; exact Version 2 CLI recovery/docs; fourteen-fragment
fold including T603; and targeted contract/API/drift gates must all pass. Any missing,
duplicate, mixed, unrelated, malformed, misordered, stale, wrong-candidate, progress-free, or
over-budget SC-002 sample, or any other false conjunct, blocks T219 and names the failed
remediation. T602 and T219 close only this partial checkpoint; neither may mark US1 complete.
Historical validation remains historical and is not reclassified.
  For SC-002, the outer schema-v2 `EvidenceRecord` remains unchanged. T602 accepts a failed
  operator record with no receipt as an imported failure only, never as one of the eight
  passing close records. The passing operator record must resolve exactly one
  `Sc002ActivationReceiptV1` through canonical candidate-relative content address
  `evidence-sidecars/sc002/sha256/<typed-digest>.json`. At every stage, hash the exact opened bytes
  before decode from the same fd and match the actual outer
  `candidate_id`/`content_id`/`snapshot_sha256` triplet. Absolute, traversal, URL, symlink,
  hard-link, replacement-race, missing, or duplicate receipt state, and any positive receipt
  on a failed record, blocks T219.
  T602 additionally requires T589's strict-binding suite to prove synchronized first requests
  across candidate directories for one program/wave yield exactly one success and one durable
  fd-anchored reservation. The point oracle is zero before no-replace publication, zero or one
  after publication but before wave-directory `fsync`, and exactly one after directory
  `fsync`, followed by permanent refusal of every same-candidate or alternate-candidate
  request. Fd-relative orphan cleanup must leave zero temporary residue and durably sync the
  directory. Crash/restart injection around panel-request publication and terminal unanimous
  or nonunanimous disposition must preserve idempotent ordering, zero or one reservation as
  allowed by the publication oracle, retained request/disposition records, and no retry,
  release, successor admission, or duplicate request. Same-candidate second request,
  alternate-candidate request, and post-request byte-identical history rebase/evidence refresh
  each fail at all three stages. The generic pre-request rebase proof and repeatable
  nonbinding phase-panel path still pass before reservation. A retained-state fixture must
  seed the already consumed Wave 5 request, run unanimous and finding-plus-rerun phase
  sequences, and prove the complete delivery state is byte-identical afterward: zero new
  reservations or requests and no mutation, deletion, rename, or reclassification of the
  retained request, disposition, or candidate bytes.

  T602 interprets every handoff shorthand above through the corrected split: the accepted
  external floor atomically owns the exact nonempty 13-member
  `SourceGenerationCompatibilityFloorV1` census and authenticated receipt chain. T589 invokes
  the disposition-pinned validator, obtains private nonserializable
  `AuthenticatedSourceFloorIssuerProvenance`, and consumes it into the private validated-floor
  result; a copied authority-digest/receipt chain is never authority and produces neither
  result. The matrix visits all 13 roles for all
  seven poison classes under the exact `source-floor/<class>/<role>` ids with array and
  declared cardinality 13, the complete ordered enclosing-hash/signature recomputation,
  independently pinned valid test keys, exact independent 13-row role/artifact matrix, and
  exact literal count 91. A separately authored literal 13-row constant agrees with the
  fixture, production registry, and visitor; all 39 missing/stale-digest/cross-disposition
  cases preserve cardinality and enclosing receipts, and all four matrix-meta poisons fail
  through the fixture-contract runner. The five copied-issuer cases fail only at their pinned-key checks,
  the 20 issuer-authentication/capability and 21 hash-vector negatives reach only their named checks,
  and every one of the accepted 15 digest and four signature vectors is independently
  reconstructed; the separately literal 32-id receipt/transition negative registry is exact
  and every id reaches only its named check; canonical Version 2-owned schemas/golden vectors
  and wrong encoding/frame/domain/proof negatives also pass. T592 consumes that set read-only and owns only the target-v5 adoption half and target artifacts. The
  caller-flake target executable runs only unprivileged, and the separately pinned installed
  apply object gets no URI or reference to reevaluate. The evidence includes
  zero-output/multi-output refusal, independent target/apply/GC-root/symlink substitution
  negatives, and all six apply-peer transitions before the first mutation plus the full
  post-first cross-product at the fourteen exact later edges from the independent 15-id
  registry, with all six pre-first and 84 post-first literal case ids visited exactly once,
  the separate literal constant and mutation-edge fixture equal to production order, all
  three edge meta-poisons and all 15 post-first negative ids reaching their intended checks,
  no persistent pidfd, no later mutation after refusal, only the two canonical typed
  correlation digests, and every literal in the fifteen-row apply-peer forbidden-value
  registry
  absent from every state/output surface and from metrics. Its SC-002 crash matrix includes every ancestor-directory
  sync plus payload-file sync, the one stable candidate OFD-lock inode, live-owner refusal
  before namespace access for cleanup overlap with every importer, cleanup, incident-recover,
  incident-apply, successor-admit, and retention live owner,
  identity-preserving ordinary quarantine/reopen/no-replace durable retirement, immutable
  incident metadata/preimage/path publication, payload move/reopen/file-sync, residue staging/finalization,
  append-only status and resolution publication, and every leaf/parent/ancestor sync. Replacement runs on
  both sides of every rename/reopen.
  Same/different-input every-writer/cleanup, cleanup/every-writer, cleanup/cleanup,
  every-writer/retention, and retention/every-writer overlaps use independently opened lock
  descriptions and both owner
  orderings at every named latch; live-owner losers have zero namespace access and
  `critical_section_max = 1`, and the sole post-release retry uses fresh fds and a new locked
  census. It also covers two same-byte
  distinct-inode orphans retiring under distinct ids, forced retirement `EEXIST`, the
  64-leaf/1,048,576-byte bounds, malformed retired census, the private retention-owner seal,
  and every zero-mutation whole-scope retention predicate and candidate-root/permanent-history
  preservation boundary. Ordinary terminals leave empty ephemeral namespaces and a valid
  bounded durable census. Incident terminals retain exact metadata plus either a revalidated
  file-synced payload and `parked`, exact residues and `mismatch-retained`, or a complete
  census or bounded-failure frozen-primary-evidence resolution, with an append-only branch outside those
  namespaces. A name/reopen race is exactly `recovery-resumable` or
  `recovery-irreconcilable`, preserves every name, publishes no parked status, and blocks
  close until its advertised recovery or authenticated resolution reaches a terminal.
  Zero-name, retired-source, anchor/metadata-conflict, and status-conflict fixtures prove terminal
  disposition and successor reachability. Inspect must expose the same stable incident id, exact cause,
  state variant, deterministic remediation, and thirteen-line/JSON projection before apply, while successor
  admission remains bound to the authenticated disposition and a fresh distinct triplet.
  The exact independent 61-id receipt and 45-id malformed-census registries must match their
  separately literal constants and be visited once each. Invalid/unstable census fixtures
  produce the identity-bearing bounded-failure commitment; raw `01ff`, resolution-leaf
  inclusion, copied cross-incident commitment, and post-resolution primary mutation block
  apply or successor admission. The shared nineteen-digest/one-signature SC-002 oracle is
  the only locator/incident/resolution/disposition expected-byte source.
  No SC-002 sidecar data leaf is unlinked. This correction supersedes every earlier T602
  cleanup-by-deletion shorthand.
  Its full unit
  census excludes exactly canonical `d2b.slice` before exact-three comparison and nothing
  else; exact-set validation against T604's independently literal 15-id constant and pinned
  15-id unit-census fixture requires every
  enumeration, empty, missing-required, unexpected lifecycle kind, malformed-row, and
  skip-marker poison to fail.

  In T602's title, "mechanically unblock" means only satisfy T219's internal
  exact-candidate-evidence dependency. It does not dispose the retained Wave 5 delivery
  request or authorize T219 to request, attest, seal, or merge. Any reference above to T219
  closing the partial checkpoint remains conditional on the separate accepted external
  disposition.

- [ ] T219 [US1] `adr046w5` EXTERNAL DISPOSITION GATE + CONDITIONAL CLOSE - depends on T602 and on the external delivery-contract/tooling owner first landing the contract and typed validator for `Wave5RetainedRequestDispositionV1`. That external owner and validator are prerequisites outside this feature; T219 does not produce, install, or self-validate them. The validator must import exactly one version-1 record with every field and authority binding from `data-model.md`, including candidate `d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`, snapshot `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`, the hashed byte-preserved `panel-request.json`, zero attestations, no seal, the accepted FR-036/T072 predecessors, and exact F/commit/tree. T219 is non-authorizing before that typed import: do not dispatch pre-request lanes, issue another `/d2b-panel-round work` request, import a replacement request, panel-attest, seal, register a merge target, pass merge eligibility, or merge. The T603 A/P0 and B/P reviews and T220's iterative reviews are nonbinding `/d2b-panel-round plan` phase rounds: they create no delivery request or reservation and cannot replace or relabel the consumed request. T219 has only the imported external-disposition path. Its closed action transitions are: `remain-blocked` stays blocked; `abandon-without-merge` becomes terminal `abandoned-unmerged` and cannot advance W6 or release; `recover-panel-without-new-request` enters `panel-pending` on the external recovery-attestation surface linked to the retained request. That final action still requires all ten roster roles to attest exact F/commit/tree/disposition with `signoff = true` iff recommendations are empty before `panel-satisfied`, seal eligibility, merge eligibility, or byte-identical-F merge. A missing role, recommendation, disagreement, stale binding, reduced roster, or attempted waiver enters terminal `panel-refused`; the record itself never supplies sign-off or constitutional authority and never permits a second request. Revalidate exactly one T072 historical/current remedial disposition, A-to-B-to-C-to-F ancestry, T603's phase-plan chain, T220's latest unanimous phase-plan receipt, W2-W4 external delivery adjudications, and all T602 evidence. This feature task never silently deletes, reclassifies, reissues, or frees the consumed request. F and the `adr046w5` candidate/delivery history remain immutable.

  The refusal is actionable and closed: `adr046w5 binding request already consumed; obtain an
  accepted external delivery-contract/tooling disposition naming the retained request, exact
  F, and one closed action`. It must recommend only that external disposition,
  never a replacement candidate, second request, feature-local status edit, or force flag.

  Mechanically confirm T603's immutable B/P authorization, exact B-to-C checkbox transition, C-to-F ancestry, unique fragment, and clean exact F identity. Require pidfd-bound registrar/ZoneBus publication through T592's typed `OpenPeerPidfdFromAcceptedSocket` broker operation using a safe dependency or approved `sys.rs` FFI quarantine, with `MSG_CMSG_CLOEXEC` on both receive paths, truncation/exact-one-fd checks, all excess/error fds closed, descriptor-count and exec-leak proof, no repository unsafe outside that quarantine, per-block `SAFETY:`, fresh restart pidfd, unsupported/missing-or-extra-fd/reuse/mismatch/`ESRCH`/ambiguity denials, the single enforcing `make test-fixture-contracts` pidfd policy including its forbidden `nix` `PeerPidfd` poison, no duplicate `make test-policy` runner, and no new FFI crate or session fallback; private sealed one-shot policy bootstrap then authenticated revision flow; registered controller endpoint; admitted watch; durable effect/adoption and cleanup ledger; transactional immutable audit authority, separate export completion, replay-binding denials, fixed identifier digests, retention health, and the exact `ResourceStatus` committed-pending-audit composite on every mutation response including delete. Require T604's public declaration/removal switches without manual restart; exact Provider/config and same-resource effect/Ready evidence for `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`; exact Device cleanup, unresolvable Endpoint, and same-identity TPM state preservation; ready and unrecreated acceptance Volume/Network and unrelated resources; explicit Guest deferral; validation/build/stage/public-socket-authorization/opaque-request-only deployment entrypoint; accepted-socket Admin evidence transferred over the authenticated protocol-4 channel; sealed durable handoff capability and broker-managed exact executable store-object pin; no-reevaluation apply plus executable/symlink substitution refusal; capability-authorized normal/compatibility-broker-only audited profile/service/3/1-bootstrap/publication/rollback mutations; existing broker-service start/restart ownership and durable coordinator before first mutation; entrypoint and compatibility-process death recovery; target daemon Hello while unready before phase-attenuated authenticated publication; durable broker publication before ingestion/readiness; broker-coordinator recovery across target broker/daemon failure; fixed Nix error output with raw stderr canaries absent; runnable parameterized fail-closed migration/rollback procedures; and candidate-bound FR-075 Ready/Stopped, fresh-pidfd, PID reuse/mismatch/ambiguity quarantine, exact-three-unit, no-skip continuity. Reopen the sole referenced SC-002 receipt through T589's same import/durable-reopen/panel-request/panel-attest/seal/eligibility validator and   require canonical candidate-relative content address `evidence-sidecars/sc002/sha256/<typed-digest>.json`, hash-before-decode from the same fd at every stage, actual outer `candidate_id`/`content_id`/`snapshot_sha256` triplet binding, absolute/traversal/URL/symlink/hard-link/replacement refusal, schema version 1, <=16,384 bytes, the closed three-resource census, same-identity effect/Ready/selected-stop/progress observations, checked elapsed <=2,000,000,000 ns, and 1-32 ordered progress events per sample. Require the coordinated T605 contract, T595 emitter, T599 consumers, fourteen-fragment fold, generated manifests, targeted contract tests, current API snapshots, paired reference, and byte-identical Zone desired-schema result. The readiness projection must contain the exact `Provider/system-core` registration plus exactly one `system-core-host` and one `system-core-user` handler record with phase/timestamp; duplicates, missing/underscore/wrong names, or `ProviderLifecycle` substitution fail. Also confirm per-Zone failure isolation, D106 store neutrality, exact-tip RSS/owner fan-in, current removal proofs, and exact CLI retry/status/reference consistency (FR-066-FR-075, SC-030-SC-035). A fabricable or missing handoff capability, executable substitution, daemon-identity/euid0/provenance/caller-claim authority, readiness flag, numeric-PID-only admission, status-only substitute, direct/fake boundary, disabled audit owner, mutable/missing authoritative row, ordinary success for pending export, unaudited privileged transition, entrypoint rollback dependence, raw Nix stderr leak, incomplete effect/cleanup/continuity evidence, any missing/duplicate/mixed/unrelated/malformed/misordered/stale/wrong-candidate/progress-free/over-budget SC-002 sample, a Wave 5 Guest-success claim, incomplete coordinated contract evidence, stale receipt/evidence, dirty candidate, or fictitious `ResourceUpdateStatus` phase/code blocks the gate. Confirm docs, decision-register conformance, and removal proofs (FR-019, FR-047, FR-023). Merge only when eligibility confirms F byte-for-byte; then rebase, clean worktrees/branches/targets, run `nix-collect-garbage`, and audit residue. From round 9, LOW/MEDIUM may be deferred; CRITICAL/HIGH never (FR-051, FR-052). At close, review registers and log friction (FR-053).
  In T219's shorthand above, "accepted-socket Admin evidence" means exactly one fd consumed
  through independently accepted source-generation peers only after they negotiate numeric
  protocol 4 plus the exact `source-handoff-v1` operation-catalogue fingerprint. Bare
  committed protocol 4 and a peer-fingerprint mismatch refuse. The external disposition owns
  the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census and
  authenticated issuer chain atomically. T589 invokes the disposition-pinned validator and
  consumes private authenticated issuer provenance into the separate private validated-floor
  result. The independently literal 13-row constant,
  independent role/artifact fixture, exact 91-case all-role poison matrix, four matrix-meta
  negatives, exact five-case copied-issuer matrix, exact 20-case issuer-authentication/
  capability and 21-case hash-vector registries, independently
  reconstructed 15 digest/four signature vectors, canonical schemas/goldens, and
  `missing`/`duplicate`/`extra`/`empty`/`stale-generation`/`stale-digest`/
  `cross-disposition` refusals must pass; all 39 named-role
  missing/stale-digest/cross-disposition cases retain cardinality 13 and valid enclosing
  receipts. T592 consumes that set read-only; its only handoff ownership
  is `AdoptHostGenerationHandoffV1` and the target-v5 artifact set. The caller-flake target
  executable runs only unprivileged; the separately pinned installed apply object runs under
  `sudo` with no URI or reference to reevaluate. Target/apply/GC-root/symlink substitution,
  zero-output/multi-output resolution, and every one of the six apply-peer identity
  transitions refuses before the first mutation and in the full post-first cross-product at
  each of the fourteen exact later mutation edges. The exact post-first set is the 84
  literal `apply-peer/post-first/<edge>/<transition>` ids, the mutation-edge fixture pins all
  15 production ids independently from a literal 15-id constant, all three edge
  meta-poisons fail, and every one of the 15 post-first negatives reaches its intended
  check; no
  selected or successor mutation occurs, its peer pidfd
  is connection-scoped and never persisted, and the durable prefix plus first audit is
  unchanged. Every literal in the closed fifteen-row apply-peer forbidden-value registry is
  injected independently and never
  escapes any coordinator,
  receipt/evidence, human, JSON, wire, error/`Display`, log, tracing event/span, metric
  name/label/value/exemplar, audit, panic, or `Debug`   surface. Only the exact canonical process-instance and executable-identity correlation
  digests are permitted for their class, and metrics carry no raw or digested
  peer-identity label or value;
  "normal/compatibility-broker-only" means that installed source broker before transfer and
  the target broker after transfer; and "exact-three-unit" means exact set equality after
  enumerating the full loaded `d2b*`/`microvm*` namespace and excluding exactly canonical
  `d2b.slice`, never a query limited to expected names and never filtering another lifecycle
  unit, and injected unexpected-slice and unexpected-service cases each fail. Reopening
  SC-002 also requires the T589 import proof: explicit current-effective-uid
  `0600` source, hash-before-decode, current-effective-uid `0600` no-replace destination under
  current-effective-uid `0700` dirfds, every ancestor-directory sync, one verified
  stable candidate-scoped OFD lock inode shared by importer and cleanup, live-owner refusal
  before namespace access, identity-preserving quarantine/reopen/no-replace durable
  retirement with both directory syncs for verified orphans, no sidecar-data unlink, empty
  ephemeral namespaces on ordinary terminals, and durable file-synced-payload `parked`,
  authenticated residue-backed `mismatch-retained`, frozen-primary-evidence-bound irreconcilable
  resolution, or inspectable stable-id resumable/irreconcilable state with closed
  cause/remediation and publication/close denial, plus the complete
  crash/replacement/overlap matrix.
  Here "then rebase" means that **W6 rebases its own branch onto updated `v3` only after a
  successful byte-identical Wave 5 merge**. Final F and the `adr046w5` candidate and delivery
  history remain immutable and are never rebased.

**Checkpoint `adr046w5`**: internal convergence does not close the wave. Close remains blocked
until the external disposition is accepted and every expressly authorized close condition
passes. After a successful byte-identical merge, W6 alone rebases onto updated `v3`; this
checkpoint claims no rebase or mutation of F or `adr046w5` history.

---

## Wave W6: All 27 Provider dossiers in five file-disjoint families

**Requirements**: see spec-coverage.md traceability tables | **Story**: US2 | **Work items**: 258 | **Parallel groups**: 29

- [ ] T221 [US2] W6 PLAN PANEL + ENTRY - first require FR-036's accepted external constitution amendment to be an ancestor of the exact W6 entry base. Then, before any W6 implementation lane is dispatched, confirm Gate 0 passed, destinations are uncontended, the stack is proposed against the exact named parent commit, the heavy-gate semaphore is available, and the fast hermetic suite is green on the entry tree; run `/d2b-panel-round plan` against that exact clean base and feature snapshot and require 10/10 sign-off with zero recommendations. If `adr046w5` is not yet merged, implementation entry additionally requires at least 5 of its 10 work reviews returned and green integration on its converged tree. A constitution-amendment, base, or feature change before dispatch invalidates the plan receipt. This authorizes implementation only: T480's distinct work panel, seal, and merge eligibility remain blocked until `adr046w5` is sealed and merged and W6 is rebased onto the updated integration lineage (FR-057).

### Group `wi:ADR-046-provider-activation-nixos` (7 items)

- [ ] T222 [P] [US2] `ADR046-activation-001` - packages/d2b-host/src/bin/d2b-activation-helper.rs (adapt)
- [ ] T223 [P] [US2] `ADR046-activation-002` - docs/reference/schemas/v3/activation-nixos.d2bus.org.NixosGeneration.json and packages/d2b-contracts/src/activation_nixos.rs (create)
- [ ] T224 [US2] `ADR046-activation-003` - packages/d2b-provider-activation-nixos/src/controller/ (replace)
- [ ] T225 [US2] `ADR046-activation-004` - packages/d2b-provider-activation-nixos/src/runner/ (adapt)
- [ ] T226 [P] [US2] `ADR046-activation-005` - packages/d2b/src/activation.rs (replace)
- [ ] T227 [P] [US2] `ADR046-activation-006` - nixos-modules/providers/activation-nixos.nix (adapt)
- [ ] T228 [US2] `ADR046-activation-007` - packages/d2b/src/lib.rs (delete-after-cutover)

### Group `wi:ADR-046-provider-audio-pipewire` (13 items)

- [ ] T229 [P] [US2] `ADR046-audio-001` - `packages/d2b-provider-audio-pipewire/src/audio_policy.rs` (copy-unchanged)
- [ ] T230 [US2] `ADR046-audio-002` - `packages/d2b-provider-audio-pipewire/src/argv.rs` (component template renderer) (adapt)
- [ ] T231 [US2] `ADR046-audio-004` - `packages/d2b-provider-audio-pipewire/src/mediator/enforcement.rs` (adapt)
- [ ] T232 [US2] `ADR046-audio-005` - `packages/d2b-provider-audio-pipewire/src/{resource_type,admission,provider_extension}.rs` (adapt)
- [ ] T233 [US2] `ADR046-audio-006` - `packages/d2b-provider-audio-pipewire/src/controller/audio_service.rs` (adapt)
- [ ] T234 [US2] `ADR046-audio-007` - `packages/d2b-provider-audio-pipewire/src/mediator/mod.rs` (create)
- [ ] T235 [US2] `ADR046-audio-008` - `nixos-modules/components/audio/v3-resource.nix` (replace)
- [ ] T236 [US2] `ADR046-audio-009` - `packages/d2b-provider-audio-pipewire/tests/minijail_contract.rs` (provider-local) (adapt)
- [ ] T237 [US2] `ADR046-audio-010` - `packages/d2b-provider-audio-pipewire/src/telemetry.rs` (adapt)
- [ ] T238 [US2] `ADR046-audio-011` - `packages/d2b-provider-audio-pipewire/src/guest_agent/mod.rs` (adapt)
- [ ] T239 [US2] `ADR046-audio-012` - `packages/d2b-provider-audio-pipewire/src/share_adapter.rs` (adapt)
- [ ] T240 [US2] `ADR046-audio-013` - `packages/d2b-provider-audio-pipewire/src/authority.rs` (speaker mixer + mic arbiter) (adapt)
- [ ] T241 [US2] `ADR046-audio-014` - `packages/d2b-provider-audio-pipewire/src/streams.rs` (adapt)

### Group `wi:ADR-046-provider-clipboard-wayland` (12 items)

- [ ] T242 [P] [US2] `ADR046-clipboard-001` - packages/d2b-provider-clipboard-wayland/ with src (create)
- [ ] T243 [US2] `ADR046-clipboard-002` - packages/d2b-provider-clipboard-wayland/src/clipd_host/ service binary modules such as service (adapt)
- [ ] T244 [US2] `ADR046-clipboard-003` - packages/d2b-provider-clipboard-wayland/src/controller/ and clipboard-controller binary (create)
- [ ] T245 [US2] `ADR046-clipboard-004` - packages/d2b-provider-clipboard-wayland/src/picker_session/ and picker-session binary (adapt)
- [ ] T246 [US2] `ADR046-clipboard-005` - packages/d2b-provider-clipboard-wayland service descriptors and generated Rust async ttrpc bindings (create)
- [ ] T247 [P] [US2] `ADR046-clipboard-006` - nixos-modules/providers/clipboard-wayland.nix and d2b.artifacts.clipboard-wayland catalog entry (replace)
- [ ] T248 [US2] `ADR046-clipboard-007` - packages/d2b-provider-clipboard-wayland/src/controller/rbac.rs or equivalent controller reconcile module (create)
- [ ] T249 [US2] `ADR046-clipboard-008` - packages/d2b-provider-clipboard-wayland/src/service/audit.rs and packages/d2b-provider-clipboard-wayland/src/service/metrics.rs (adapt)
- [ ] T250 [US2] `ADR046-clipboard-009` - packages/d2b-provider-clipboard-wayland/tests/ (extract)
- [ ] T251 [US2] `ADR046-clipboard-010` - packages/d2b-provider-clipboard-wayland/integration/ (create)
- [ ] T252 [US2] `ADR046-clipboard-011` - packages/d2b-contract-tests/tests/policy_clipboard.rs (adapt)
- [ ] T253 [US2] `ADR046-clipboard-012` - nixos-modules/default.nix (delete-after-cutover)

### Group `wi:ADR-046-provider-credential-entra` (1 items)

- [ ] T254 [US2] `ADR046-cred-entra-001` - `packages/d2b-provider-credential-entra/src/{lib.rs,controller.rs,service.rs,controller_main.rs,agent_main.rs,audit.rs,telemetry.rs}` (adapt)

### Group `wi:ADR-046-provider-credential-managed-identity` (5 items)

- [ ] T255 [US2] `ADR046-cred-mi-001` - `packages/d2b-provider-credential-managed-identity/src/{lib.rs, controller.rs, agent.rs, service.rs, audit.rs, telemetry.rs}` (adapt)
- [ ] T256 [US2] `ADR046-cred-mi-002` - packages/d2b-provider-credential-managed-identity/src/controller.rs (adapt)
- [ ] T257 [US2] `ADR046-cred-mi-003` - nixos-modules/options-resources.nix (replace)
- [ ] T258 [US2] `ADR046-cred-mi-004` - packages/d2b-provider-credential-managed-identity/src/{audit.rs,telemetry.rs} (adapt)
- [ ] T259 [US2] `ADR046-mi-topology-001` - packages/d2b-provider-credential-managed-identity/src/{controller.rs,agent.rs} (adapt)

### Group `wi:ADR-046-provider-credential-secret-service` (6 items)

- [ ] T260 [P] [US2] `ADR046-cred-ss-001` - packages/d2b-contracts/src/v3/credential.rs (adapt)
- [ ] T261 [P] [US2] `ADR046-cred-ss-002` - packages/d2b-contracts/proto/v3/credential.proto (create)
- [ ] T262 [US2] `ADR046-cred-ss-003` - `packages/d2b-provider-credential-secret-service/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [ ] T263 [P] [US2] `ADR046-cred-ss-004` - packages/d2b-provider-credential-<impl>/src/controller.rs (create)
- [ ] T264 [P] [US2] `ADR046-cred-ss-005` - nixos-modules/options-resources.nix (create)
- [ ] T265 [P] [US2] `ADR046-cred-ss-006` - packages/d2b-provider-credential-secret-service/src/{audit.rs,telemetry.rs} (adapt)

### Group `wi:ADR-046-provider-device-gpu` (9 items)

- [ ] T266 [P] [US2] `ADR046-gpu-001` - `packages/d2b-provider-device-gpu/` with `src/` (extract)
- [ ] T267 [US2] `ADR046-gpu-002` - `packages/d2b-provider-device-gpu/src/{controller.rs,telemetry.rs}` (adapt)
- [ ] T268 [US2] `ADR046-gpu-003` - `packages/d2b-provider-device-gpu/src/probe.rs` (create)
- [ ] T269 [US2] `ADR046-gpu-004` - `packages/d2b-provider-device-gpu/src/arbitration.rs` (create)
- [ ] T270 [US2] `ADR046-gpu-005` - `packages/d2b-provider-device-gpu/src/worker_gpu.rs` (adapt)
- [ ] T271 [US2] `ADR046-gpu-006` - `packages/d2b-provider-device-gpu/src/worker_video.rs` (adapt)
- [ ] T272 [US2] `ADR046-gpu-007` - `nixos-modules/assertions.nix` (new GPU Device eval assertions) (adapt)
- [ ] T273 [US2] `ADR046-gpu-008` - `packages/d2b-provider-device-gpu/` component descriptor (create)
- [ ] T274 [US2] `ADR046-gpu-009` - `packages/d2b-provider-device-gpu/README.md` (create)

### Group `wi:ADR-046-provider-device-security-key` (35 items)

- [ ] T275 [US2] `ADR046-security-key-001` - Move to `packages/d2b-provider-device-security-key/src/session.rs` and `cid.rs` (adapt)
- [ ] T276 [US2] `ADR046-security-key-002` - Move to `packages/d2b-provider-device-security-key/src/relay.rs` (adapt)
- [ ] T277 [US2] `ADR046-security-key-003` - Adopt `main.rs` and `uhid.rs` as the v3 Process binary entry point (adapt)
- [ ] T278 [US2] `ADR046-security-key-004` - Preserve revalidation logic (adapt)
- [ ] T279 [US2] `ADR046-security-key-005` - Adapt to v3 Zone/ResourceRef identifiers (adapt)
- [ ] T280 [US2] `ADR046-security-key-006` - Move to `packages/d2b-provider-device-security-key/tests/` (adapt)
- [ ] T281 [US2] `ADR046-security-key-007` - Move to `packages/d2b-provider-device-security-key/tests/` (adapt)
- [ ] T282 [P] [US2] `ADR046-security-key-008` - New crate `packages/d2b-provider-device-security-key/` with `src/` (create)
- [ ] T283 [US2] `ADR046-security-key-009` - `packages/d2b-provider-device-security-key/src/controller.rs` (create)
- [ ] T284 [US2] `ADR046-security-key-010` - `packages/d2b-provider-device-security-key/src/relay.rs` (create)
- [ ] T285 [US2] `ADR046-security-key-011` - `packages/d2b-provider-device-security-key/src/session.rs` (create)
- [ ] T286 [US2] `ADR046-security-key-012` - `packages/d2b-provider-device-security-key/src/cid.rs` (create)
- [ ] T287 [US2] `ADR046-security-key-013` - `packages/d2b-provider-device-security-key/src/probe.rs` (create)
- [ ] T288 [US2] `ADR046-security-key-014` - `packages/d2b-provider-device-security-key/src/descriptor.rs` (create)
- [ ] T289 [US2] `ADR046-security-key-015` - `nixos-modules/minijail-profiles.nix` entries for relay and controller (create)
- [ ] T290 [US2] `ADR046-security-key-016` - Provider descriptor Process templates and owned CTAPHID `Endpoint` template for `Provider/device-security-key` (create)
- [ ] T291 [US2] `ADR046-security-key-017` - Signed Provider descriptor JSON for `Provider/device-security-key` in the provider package (create)
- [ ] T292 [US2] `ADR046-security-key-018` - v3 `SecurityKeyOpenDevice` broker op and Core LaunchTicket DeviceGrant resolution path (create)
- [ ] T293 [US2] `ADR046-security-key-019` - `nixos-modules/` resource compiler/eval assertions for physical Device (create)
- [ ] T294 [US2] `ADR046-security-key-020` - `nixos-modules/components/security-key-guest.nix` migration gate `d2b.securityKey._legacySystemdUnit` (create)
- [ ] T295 [US2] `ADR046-security-key-021` - Core `device-grant` audit and Provider controller Service/Binding ceremony lifecycle audit (create)
- [ ] T296 [US2] `ADR046-security-key-022` - Provider/controller bounded telemetry emitter and observability-otel handoff for security-key metrics (create)
- [ ] T297 [US2] `ADR046-security-key-023` - `packages/d2b-provider-device-security-key/README.md` (create)
- [ ] T298 [US2] `ADR046-security-key-024` - Authority/projection Service Endpoint and Binding private Endpoint resolution (create)
- [ ] T299 [US2] `ADR046-security-key-025` - `d2b-contracts` neutral `SecurityKeyEffectPort` trait/types (create)
- [ ] T300 [US2] `ADR046-security-key-026` - `packages/d2b-provider-device-security-key/src/{resource_type,provider_extension,admission}.rs` (create)
- [ ] T301 [US2] `ADR046-security-key-027` - Provider descriptor state declaration (create)
- [ ] T302 [US2] `ADR046-security-key-028` - `packages/d2b-provider-device-security-key/src/share_adapter.rs` (adapt)
- [ ] T303 [US2] `ADR046-security-key-029` - `packages/d2b-provider-device-security-key/src/{authority,relay,streams}.rs` (adapt)
- [ ] T304 [US2] `ADR046-security-key-030` - Removed from daemon (delete-after-cutover)
- [ ] T305 [US2] `ADR046-security-key-031` - Removed from daemon startup (delete-after-cutover)
- [ ] T306 [US2] `ADR046-security-key-032` - Removed from guest Nix module (delete-after-cutover)
- [ ] T307 [US2] `ADR046-security-key-033` - Removed from `packages/d2b-contract-tests/tests/` (delete-after-cutover)
- [ ] T308 [US2] `ADR046-security-key-034` - Removed from `d2b-core/src/processes.rs` (delete-after-cutover)
- [ ] T309 [US2] `ADR046-security-key-035` - Removed from contracts and broker (delete-after-cutover)

### Group `wi:ADR-046-provider-device-tpm` (13 items)

- [ ] T310 [P] [US2] `ADR046-device-tpm-001` - packages/d2b-provider-device-tpm/{src/,tests/,integration/README.md,README.md} (adapt)
- [ ] T311 [US2] `ADR046-device-tpm-002` - packages/d2b-provider-device-tpm/src/effect_port.rs (wrap)
- [ ] T312 [US2] `ADR046-device-tpm-003` - packages/d2b-provider-device-tpm/src/controller.rs (replace)
- [ ] T313 [US2] `ADR046-device-tpm-004` - packages/d2b-provider-device-tpm/src/resources.rs (replace)
- [ ] T314 [US2] `ADR046-device-tpm-005` - packages/d2b-provider-device-tpm/src/resources.rs (adapt)
- [ ] T315 [US2] `ADR046-device-tpm-006` - packages/d2b-provider-device-tpm/src/resources.rs (adapt)
- [ ] T316 [US2] `ADR046-device-tpm-007` - packages/d2b-provider-device-tpm/src/status.rs (create)
- [ ] T317 [US2] `ADR046-device-tpm-008` - packages/d2b-provider-device-tpm/src/{effect_port.rs,status.rs} (replace)
- [ ] T318 [US2] `ADR046-device-tpm-009` - packages/d2b-provider-device-tpm/tests/marker_fail_closed.rs (adapt)
- [ ] T319 [US2] `ADR046-device-tpm-010` - packages/d2b-provider-device-tpm/src/resources.rs (create)
- [ ] T320 [US2] `ADR046-device-tpm-011` - nixos-modules/options-resources.nix and Nix eval/golden tests for §17.1 Device JSON (replace)
- [ ] T321 [US2] `ADR046-device-tpm-012` - packages/d2b-provider-device-tpm/src/controller.rs (adapt)
- [ ] T322 [US2] `ADR046-device-tpm-013` - packages/d2bd/src/* (delete-after-cutover)

### Group `wi:ADR-046-provider-device-usbip` (9 items)

- [ ] T323 [P] [US2] `ADR046-usbip-001` - packages/d2b-contracts/src/usbip_effect_port.rs (create)
- [ ] T324 [US2] `ADR046-usbip-002` - packages/d2b-core/src/device_usbip_adapter.rs (adapt)
- [ ] T325 [US2] `ADR046-usbip-003` - packages/d2b-provider-device-usbip/ (create)
- [ ] T326 [US2] `ADR046-usbip-004` - packages/d2b-provider-device-usbip/src/{controller,reconcile,export_import}.rs (adapt)
- [ ] T327 [US2] `ADR046-usbip-005` - packages/d2b-provider-device-usbip/src/reconcile.rs (adapt)
- [ ] T328 [US2] `ADR046-usbip-006` - packages/d2b-provider-device-usbip/src/status.rs (adapt)
- [ ] T329 [US2] `ADR046-usbip-007` - packages/d2b-provider-device-usbip/{src,tests,integration/README.md} (adapt)
- [ ] T330 [US2] `ADR046-usbip-008` - nixos-modules/components/usbip.nix (adapt)
- [ ] T331 [US2] `ADR046-usbip-009` - packages/d2bd/src/ (delete-after-cutover)

### Group `wi:ADR-046-provider-display-wayland` (4 items)

- [ ] T332 [US2] `ADR046-display-001` - `packages/d2b-provider-display-wayland/src/` (adapt)
- [ ] T333 [US2] `ADR046-display-002` - Zone bundle emitter for `WaylandSession` / `WaylandPolicy` ResourceSpecs under `d2b.zones.<zone>.resources.*` (adapt)
- [ ] T334 [US2] `ADR046-display-003` - `packages/d2b-provider-display-wayland/src/audit.rs` (adapt)
- [ ] T335 [US2] `ADR046-display-004` - `packages/d2b-provider-display-wayland/integration/` (create)

### Group `wi:ADR-046-provider-network-local` (20 items)

- [ ] T336 [P] [US2] `ADR046-nl-001` - `d2b-contracts` trait plus `d2b-core` core adapter (create)
- [ ] T337 [US2] `ADR046-nl-002` - Broker wire contract and broker/core adapter operation table for `DeletePersistentTap` (adapt)
- [ ] T338 [P] [US2] `ADR046-nl-003` - `d2b-contracts` opaque byte-array newtypes (create)
- [ ] T339 [US2] `ADR046-nl-004` - Core LaunchTicket builder and dependency resolver that walks `Guest.ownerRef: Network/<name>` to resolved tap FDs. (create)
- [ ] T340 [P] [US2] `ADR046-nl-005` - Core adapter imports `d2b-host` modules (adapt)
- [ ] T341 [US2] `ADR046-nl-006` - `packages/d2b-provider-network-local/src/{controller.rs,metrics.rs}` (adapt)
- [ ] T342 [P] [US2] `ADR046-nl-007` - `packages/d2b-provider-network-local/src/process_specs.rs` agent template plus agent service implementation in the net-VM artifact. (create)
- [ ] T343 [P] [US2] `ADR046-nl-008` - `packages/d2b-provider-network-local/src/config_volume.rs`. (adapt)
- [ ] T344 [P] [US2] `ADR046-nl-009` - `packages/d2b-provider-network-local/src/process_specs.rs`. (adapt)
- [ ] T345 [P] [US2] `ADR046-nl-010` - `net-vm-base` nixos-system artifact and artifact catalog entry `d2b.artifacts.net-vm-base`. (adapt)
- [ ] T346 [P] [US2] `ADR046-nl-011` - Nix module resource emission for `Provider/network-local` (adapt)
- [ ] T347 [P] [US2] `ADR046-nl-012` - Nix flake/resource schema checks for declared Networks and provider `validate.rs` parity. (adapt)
- [ ] T348 [P] [US2] `ADR046-nl-013` - `packages/d2b-provider-network-local/tests/schema_roundtrip.rs` (adapt)
- [ ] T349 [US2] `ADR046-nl-014` - `packages/d2b-provider-network-local/tests/controller_state.rs`. (create)
- [ ] T350 [P] [US2] `ADR046-nl-015` - `packages/d2b-provider-network-local/integration/host_fabric.rs` (adapt)
- [ ] T351 [P] [US2] `ADR046-nl-016` - Process templates for agent and dnsmasq plus sandbox/eval tests. (adapt)
- [ ] T352 [P] [US2] `ADR046-nl-017` - `packages/d2b-provider-network-local/README.md`. (create)
- [ ] T353 [P] [US2] `ADR046-nl-018` - Device-usbip EffectPort/adapter owns USBIP rules (adapt)
- [ ] T354 [P] [US2] `ADR046-nl-019` - Provider descriptor (create)
- [ ] T355 [P] [US2] `ADR046-nl-020` - Network schema/Provider descriptor (adapt)

### Group `wi:ADR-046-provider-notification-desktop` (6 items)

- [ ] T356 [P] [US2] `ADR046-notify-001` - `packages/d2b-provider-notification-desktop/src/{types,redact,action_nonce}.rs` (adapt)
- [ ] T357 [US2] `ADR046-notify-002` - `packages/d2b-provider-notification-desktop/src/stream_admission.rs` (adapt)
- [ ] T358 [US2] `ADR046-notify-003` - `packages/d2b-provider-notification-desktop/src/controller.rs` (create)
- [ ] T359 [US2] `ADR046-notify-004` - `packages/d2b-provider-notification-desktop/src/host_sink.rs` (adapt)
- [ ] T360 [US2] `ADR046-notify-005` - `packages/d2b-provider-notification-desktop/src/guest_source.rs` (create)
- [ ] T361 [US2] `ADR046-notify-006` - Nix: Zone resource authoring in `nixos-modules/` (adapt)

### Group `wi:ADR-046-provider-observability-otel` (6 items)

- [ ] T362 [US2] `ADR046-otel-001` - `packages/d2b-provider-observability-otel/src/{forwarder_bin,controller,binding}.rs` (adapt)
- [ ] T363 [US2] `ADR046-otel-002` - `packages/d2b-provider-observability-otel/src/{collector_bin,emitter_socket,ingress_policy,exporter,controller,service,binding}.rs` (adapt)
- [ ] T364 [US2] `ADR046-otel-003` - `packages/d2b-provider-observability-otel/src/nix/journald.nix` (adapt)
- [ ] T365 [US2] `ADR046-otel-004` - `packages/d2b-contract-tests/tests/policy_observability.rs` (updated) (adapt)
- [ ] T366 [US2] `ADR046-otel-005` - `packages/d2b-provider-observability-otel/src/share_adapter.rs` (adapt)
- [ ] T367 [US2] `ADR046-otel-006` - `packages/d2b-provider-observability-otel/src/{authority,service,binding,projection}.rs` (adapt)

### Group `wi:ADR-046-provider-runtime-azure-container-apps` (7 items)

- [ ] T368 [US2] `ADR046-aca-001` - `packages/d2b-provider-runtime-azure-container-apps/src/controller.rs` (replace)
- [ ] T369 [US2] `ADR046-aca-002` - `packages/d2b-provider-runtime-azure-container-apps/src/deployment_service.rs` (adapt)
- [ ] T370 [US2] `ADR046-aca-003` - `packages/d2b-contracts/src/provider_effects/aca.rs` (adapt)
- [ ] T371 [US2] `ADR046-aca-004` - ACA sandbox-agent Endpoint/session controller (replace)
- [ ] T372 [US2] `ADR046-aca-005` - `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` (adapt)
- [ ] T373 [US2] `ADR046-aca-006` - `nixos-modules/` (generated Guest resource options) (replace)
- [ ] T374 [US2] `ADR046-aca-007` - `nixos-modules/` (create)

### Group `wi:ADR-046-provider-runtime-azure-virtual-machine` (9 items)

- [ ] T375 [P] [US2] `ADR046-azure-vm-001` - `src/{lib.rs,config.rs,schema.rs,error.rs,effect/mod.rs}` (adapt)
- [ ] T376 [US2] `ADR046-azure-vm-002` - `src/effect/{mod.rs,real.rs,fake.rs,rate_limit.rs}` (adapt)
- [ ] T377 [US2] `ADR046-azure-vm-003` - `src/controller/{mod.rs,lifecycle.rs,idempotency.rs}` (adapt)
- [ ] T378 [US2] `ADR046-azure-vm-004` - `src/controller/bootstrap.rs` (adapt)
- [ ] T379 [US2] `ADR046-azure-vm-005` - `src/credential.rs` (adapt)
- [ ] T380 [US2] `ADR046-azure-vm-006` - `src/controller/idempotency.rs` (adapt)
- [ ] T381 [US2] `ADR046-azure-vm-007` - `nixos-modules/` (Provider/Guest resource emitters) (adapt)
- [ ] T382 [US2] `ADR046-azure-vm-008` - `src/{telemetry.rs,audit.rs}` (adapt)
- [ ] T383 [P] [US2] `ADR046-azure-vm-009` - `tests/` (adapt)

### Group `wi:ADR-046-provider-runtime-cloud-hypervisor` (7 items)

- [ ] T384 [P] [US2] `ADR046-ch-001` - `packages/d2b-provider-runtime-cloud-hypervisor/src/controller.rs` (adapt). For the manifest-required end-to-end real-KVM/guest-control validation, this task also solely owns new `tests/host-integration/runtime-cloud-hypervisor-guest-acceptance.nix` and only that check's discovery/build recipe in `Makefile`; the exact attr is `vmChecks.x86_64-linux.runtime-cloud-hypervisor-guest-acceptance`
- [ ] T385 [US2] `ADR046-ch-002` - `packages/d2b-provider-runtime-cloud-hypervisor/src/bootstrap_graph.rs` (replace)
- [ ] T386 [US2] `ADR046-ch-003` - `packages/d2b-provider-runtime-cloud-hypervisor/src/vmm_argv.rs` (adapt)
- [ ] T387 [US2] `ADR046-ch-004` - `packages/d2b-provider-runtime-cloud-hypervisor/nix/` (Nix emitter) (adapt)
- [ ] T388 [US2] `ADR046-ch-005` - `packages/d2b-provider-runtime-cloud-hypervisor/src/health.rs` (adapt)
- [ ] T389 [US2] `ADR046-ch-006` - `packages/d2b-provider-runtime-cloud-hypervisor/src/metrics.rs` (replace)
- [ ] T390 [US2] `ADR046-ch-007` - `packages/d2b-provider-runtime-cloud-hypervisor/src/state.rs` (replace)

### Group `wi:ADR-046-provider-runtime-qemu-media` (19 items)

- [ ] T391 [P] [US2] `ADR046-qemu-media-001` - packages/d2b-provider-runtime-qemu-media/{src/lib.rs,tests/provider_layout.rs,integration/mod.rs,README.md} (create)
- [ ] T392 [US2] `ADR046-qemu-media-002` - packages/d2b-provider-runtime-qemu-media/src/types/guest.rs (adapt)
- [ ] T393 [US2] `ADR046-qemu-media-003` - packages/d2b-provider-runtime-qemu-media/src/config.rs (adapt)
- [ ] T394 [US2] `ADR046-qemu-media-004` - packages/d2b-provider-runtime-qemu-media/src/{descriptor.rs,state.rs} (create)
- [ ] T395 [US2] `ADR046-qemu-media-005` - packages/d2b-provider-runtime-qemu-media/src/controller/volume.rs (adapt)
- [ ] T396 [US2] `ADR046-qemu-media-006` - packages/d2b-provider-runtime-qemu-media/src/controller/media_watch.rs (adapt)
- [ ] T397 [US2] `ADR046-qemu-media-007` - packages/d2b-provider-runtime-qemu-media/src/controller/device_watch.rs (create)
- [ ] T398 [US2] `ADR046-qemu-media-008` - packages/d2b-provider-runtime-qemu-media/src/controller/display.rs (create)
- [ ] T399 [US2] `ADR046-qemu-media-009` - packages/d2b-provider-runtime-qemu-media/src/controller/process_builder.rs (adapt)
- [ ] T400 [US2] `ADR046-qemu-media-010` - packages/d2b-provider-runtime-qemu-media/src/qmp/ (adapt)
- [ ] T401 [US2] `ADR046-qemu-media-011` - packages/d2b-provider-runtime-qemu-media/src/controller/hotplug.rs (adapt)
- [ ] T402 [US2] `ADR046-qemu-media-012` - packages/d2b-provider-runtime-qemu-media/src/controller/network.rs (create)
- [ ] T403 [US2] `ADR046-qemu-media-013` - packages/d2b-provider-runtime-qemu-media/src/controller/reconcile.rs (create)
- [ ] T404 [US2] `ADR046-qemu-media-014` - packages/d2b-provider-runtime-qemu-media/src/controller/status.rs (create)
- [ ] T405 [US2] `ADR046-qemu-media-015` - packages/d2b-provider-runtime-qemu-media/src/audit.rs (create)
- [ ] T406 [US2] `ADR046-qemu-media-016` - packages/d2b-provider-runtime-qemu-media/src/telemetry.rs (create)
- [ ] T407 [US2] `ADR046-qemu-media-017` - nixos-modules/options-guest-qemu-media.nix (adapt)
- [ ] T408 [US2] `ADR046-qemu-media-018` - packages/d2b-provider-runtime-qemu-media/tests/conformance_guest.rs (adapt)
- [ ] T409 [US2] `ADR046-qemu-media-019` - packages/d2b-provider-runtime-qemu-media/integration/ (create)

### Group `wi:ADR-046-provider-shell-terminal` (13 items)

- [ ] T410 [P] [US2] `ADR046-sterm-001` - `packages/d2b-provider-shell-terminal/src/resources/{pool,session}.rs` (create)
- [ ] T411 [P] [US2] `ADR046-sterm-002` - `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-terminal-controller.rs` (create)
- [ ] T412 [P] [US2] `ADR046-sterm-003` - `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-session-supervisor.rs` (adapt)
- [ ] T413 [P] [US2] `ADR046-sterm-004` - `packages/d2b-provider-shell-terminal/src/process_templates.rs` (replace)
- [ ] T414 [P] [US2] `ADR046-sterm-005` - `packages/d2b-provider-shell-terminal/src/service/open_session.rs` (create)
- [ ] T415 [P] [US2] `ADR046-sterm-006` - `packages/d2b-provider-shell-terminal/src/session/{pty,ring}.rs` (adapt)
- [ ] T416 [P] [US2] `ADR046-sterm-007` - `packages/d2b-provider-shell-terminal/src/session/adopt.rs` (adapt)
- [ ] T417 [P] [US2] `ADR046-sterm-008` - `packages/d2b-provider-shell-terminal/src/host_rules.rs` (replace)
- [ ] T418 [P] [US2] `ADR046-sterm-009` - `packages/d2b-provider-shell-terminal/src/guest_rules.rs` (replace)
- [ ] T419 [P] [US2] `ADR046-sterm-010` - `packages/d2b-provider-shell-terminal/src/authz.rs` (replace)
- [ ] T420 [P] [US2] `ADR046-sterm-011` - `packages/d2b-provider-shell-terminal/src/{audit,telemetry}.rs` (create)
- [ ] T421 [P] [US2] `ADR046-sterm-012` - `packages/d2b-provider-shell-terminal/src/migration.rs` (delete-after-cutover)
- [ ] T422 [P] [US2] `ADR046-sterm-013` - `packages/d2b-provider-shell-terminal/src/service/{controller,supervisor}.rs` (adapt)

### Group `wi:ADR-046-provider-system-core` (1 items)

- [ ] T423 [US2] `ADR046-system-core-001` - `packages/d2b-provider-system-core/src/manifest.rs` (adapt)

### Group `wi:ADR-046-provider-system-minijail` (6 items)

- [ ] T424 [US2] `ADR046-minijail-001` - `packages/d2b-provider-system-minijail/src/sandbox_compiler.rs` (adapt)
- [ ] T425 [US2] `ADR046-minijail-002` - Provider-side opaque request builder in `packages/d2b-provider-system-minijail/src/launch.rs` (adapt)
- [ ] T426 [US2] `ADR046-minijail-003` - Broker-side: `d2b-priv-broker` retains `SpawnRunner` and user-namespace pre-establishment (adapt)
- [ ] T427 [US2] `ADR046-minijail-004` - Broker-side parent wait/reap and typed terminal relay in `packages/d2b-priv-broker/src/` (adapt)
- [ ] T428 [US2] `ADR046-minijail-005` - `packages/d2b-provider-system-minijail/src/` - controller binary entry point (adapt)
- [ ] T429 [US2] `ADR046-minijail-006` - `nixos-modules/` - v3 Nix `Process`/`EphemeralProcess` resource authoring (adapt)

### Group `wi:ADR-046-provider-system-systemd` (3 items)

- [ ] T430 [US2] `ADR046-systemd-001` - `packages/d2b-provider-system-systemd/src/controller.rs` (async reconcile loop) (adapt)
- [ ] T431 [US2] `ADR046-systemd-002` - `nixos-modules/` (Provider ResourceSpec emission for `system-systemd`) (adapt)
- [ ] T432 [US2] `ADR046-systemd-003` - `packages/d2b-provider-system-systemd/tests/conformance.rs` (adapt)

### Group `wi:process-provider-integration:w6` (1 item)

- [ ] T039 [US1] `ADR046-process-002` - `packages/d2b-provider-system-systemd/`, `packages/d2b-provider-system-minijail/` (adapt). The authoritative graph defers this item from W4 to W6; its existing hermetic surfaces do not satisfy the production composition and Layer 2 evidence named by the manifest.

### Group `wi:ADR-046-provider-transport-azure-relay` (7 items)

- [ ] T433 [P] [US2] `ADR046-transport-relay-001` - `packages/d2b-provider-transport-azure-relay/src/relay_transport.rs` (adapt)
- [ ] T434 [US2] `ADR046-transport-relay-002` - `packages/d2b-provider-transport-azure-relay/src/credential_client.rs` (create)
- [ ] T435 [US2] `ADR046-transport-relay-003` - `packages/d2b-provider-transport-azure-relay/src/reconnect.rs` (create)
- [ ] T436 [US2] `ADR046-transport-relay-004` - `packages/d2b-provider-transport-azure-relay/src/transport_settings.rs` (create)
- [ ] T437 [US2] `ADR046-transport-relay-005` - `packages/d2b-provider-transport-azure-relay/src/backpressure.rs` (adapt)
- [ ] T438 [US2] `ADR046-transport-relay-006` - `packages/d2b-provider-transport-azure-relay/src/{metrics.rs, audit.rs}` (create)
- [ ] T439 [P] [US2] `ADR046-transport-relay-007` - `packages/d2b-provider-transport-azure-relay/src/tests/integration/README` (create)

### Group `wi:ADR-046-provider-transport-unix` (11 items)

- [ ] T440 [US2] `ADR046-transport-unix-001` - `packages/d2b-provider-transport-unix/src/credit.rs` (adapt)
- [ ] T441 [US2] `ADR046-transport-unix-002` - `packages/d2b-provider-transport-unix/src/{seqpacket,identity,socket}.rs` (adapt)
- [ ] T442 [US2] `ADR046-transport-unix-003` - `packages/d2b-provider-transport-unix/src/{stream,socket}.rs` (adapt)
- [ ] T443 [US2] `ADR046-transport-unix-004` - `packages/d2b-provider-transport-unix/src/credit.rs` (adapt)
- [ ] T444 [US2] `ADR046-transport-unix-005` - `packages/d2b-provider-transport-unix/src/descriptor.rs` (adapt)
- [ ] T445 [US2] `ADR046-transport-unix-006` - `packages/d2b-provider-transport-unix/src/admission.rs` (adapt)
- [ ] T446 [US2] `ADR046-transport-unix-007` - `packages/d2b-provider-transport-unix/src/{portal,service}.rs` (adapt)
- [ ] T447 [US2] `ADR046-transport-unix-008` - `packages/d2b-provider-transport-unix/` crate Cargo.toml binary target `d2b-transport-unix-service` (adapt)
- [ ] T448 [US2] `ADR046-transport-unix-009` - `docs/reference/schemas/v3/providers/transport-unix.transport-binding.json` (create)
- [ ] T449 [US2] `ADR046-transport-unix-010` - `packages/d2b-provider-transport-unix/src/{audit,metrics}.rs` (create)
- [ ] T450 [US2] `ADR046-transport-unix-011` - `packages/d2b-provider-transport-unix/integration/` and `integration/README.md` (adapt)

### Group `wi:ADR-046-provider-transport-vsock` (7 items)

- [ ] T451 [US2] `ADR046-vsock-001` - `packages/d2b-provider-transport-vsock/src/effect_port.rs` (create)
- [ ] T452 [US2] `ADR046-vsock-002` - `packages/d2b-provider-transport-vsock/src/framing.rs` and `src/bridge.rs` (adapt)
- [ ] T453 [US2] `ADR046-vsock-003` - `packages/d2b-provider-transport-vsock/src/service.rs` (adapt)
- [ ] T454 [US2] `ADR046-vsock-004` - `d2b-core-controller` child Zone runtime `LiveVsockEffectPort` (adapt)
- [ ] T455 [P] [US2] `ADR046-vsock-005` - ProviderDeployment Volume creation/deletion path plus `packages/d2b-provider-transport-vsock/tests/state_volume.rs`. (create)
- [ ] T456 [US2] `ADR046-vsock-006` - `packages/d2b-provider-transport-vsock/integration/host_guest.rs` and `integration/no_fd_transfer.rs`. (create)
- [ ] T457 [P] [US2] `ADR046-vsock-007` - Remove legacy paths from `d2b-host` and `d2bd` (delete-after-cutover)

### Group `wi:ADR-046-provider-volume-local` (13 items)

- [ ] T458 [US2] `ADR046-vl-001` - `d2b-contracts/src/v3/volume_layout.rs` (adapt)
- [ ] T459 [US2] `ADR046-vl-002` - Full `packages/d2b-provider-volume-local/` scaffold per §Crate layout: `src/` (adapt)
- [ ] T460 [US2] `ADR046-vl-003` - `src/controller.rs` (adapt)
- [ ] T461 [US2] `ADR046-vl-004` - `src/store_view.rs` (adapt)
- [ ] T462 [US2] `ADR046-vl-005` - `src/swtpm_volume.rs` (adapt)
- [ ] T463 [US2] `ADR046-vl-006` - `src/source.rs` (block-image and tmpfs branches) (create)
- [ ] T464 [US2] `ADR046-vl-007` - `src/{migration,snapshot,sealing}.rs` (adapt)
- [ ] T465 [US2] `ADR046-vl-008` - `src/relocation.rs` (create)
- [ ] T466 [US2] `ADR046-vl-009` - `src/audit.rs` (adapt)
- [ ] T467 [US2] `ADR046-vl-010` - `nixos-modules/zone-resources.nix` (per §ADR046-pstate-010) (adapt)
- [ ] T468 [US2] `ADR046-vl-011` - `packages/xtask/src/provider_crate_policy.rs` (adapt)
- [ ] T469 [US2] `ADR046-vl-012` - `packages/d2b-host/src/volume_effect_adapter.rs` (or the equivalent host-runtime crate designated by the Zone broker owner) (adapt)
- [ ] T470 [US2] `ADR046-vl-013` - Zone core ProviderDeployment controller-start path (outside `d2b-provider-volume-local`) (create)

### Group `wi:ADR-046-provider-volume-virtiofs` (7 items)

- [ ] T471 [US2] `ADR046-vvfs-001` - `packages/d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs` (adapt)
- [ ] T472 [US2] `ADR046-vvfs-002` - `packages/d2b-provider-volume-virtiofs/src/user_ns.rs` (conformance kit) (extract)
- [ ] T473 [US2] `ADR046-vvfs-003` - `packages/d2b-provider-volume-virtiofs/src/controller.rs` (adapt)
- [ ] T474 [US2] `ADR046-vvfs-004` - `packages/d2b-provider-volume-virtiofs/src/readiness.rs` (adapt)
- [ ] T475 [US2] `ADR046-vvfs-005` - `packages/d2b-provider-volume-virtiofs/src/controller.rs` (pre-launch prerequisite check) (adapt)
- [ ] T476 [US2] `ADR046-vvfs-006` - `nixos-modules/resources-volume.nix` (store-view and user Volume attachment emission) (adapt)
- [ ] T477 [US2] `ADR046-vvfs-export-001` - `packages/d2b-provider-volume-virtiofs/src/export.rs` (create)

### Group `wi:core-controller-coordination:w6` (1 items)

- [ ] T478 [US2] `ADR046-core-002` - `packages/d2b-core-controller/tests/system_core_coordination.rs` (adapt)

- [ ] T479 [US2] W6 CONVERGE + FREEZE + GUEST ACCEPTANCE - depends explicitly on T039 and on every W6 work-item row T222-T478, and its completion additionally depends on exact-F6 `w6-cloud-hypervisor-guest-acceptance` evidence. Do not use the numeric interval alone: derive the exact W6 work-item ID set from the authoritative manifest, require 258 unique rows, compare it with `{T039} union {T222-T478}`, and fail on any missing, extra, duplicate, unchecked, or unreachable member. Mechanically traverse the task dependency graph from every manifest-derived W6 task and require each to reach T479 before freeze. The sole Guest acceptance family is `Provider/runtime-cloud-hypervisor`: T384 (`ADR046-ch-001`) owns its controller, `tests/host-integration/runtime-cloud-hypervisor-guest-acceptance.nix`, that check's sole `Makefile` recipe, and authoritative end-to-end real-KVM/guest-control validation, while T384-T390 own the exact family files listed in their manifest rows under `packages/d2b-provider-runtime-cloud-hypervisor/` and the T387 `nixos-modules/` Guest emitter extension. The other three Guest-capable runtime families are out of this acceptance scope and no matrix is required. After `adr046w5` is sealed and merged, rebase W6 onto updated `v3`; merge every slice branch into the wave integration branch, run integration tests and CI on the converged tree, resolve every content-changing result, then reconcile and fold all changelog fragments. Before freezing, confirm reference docs landed with behavior (FR-019), no change contradicts the decision register (FR-047), every required removal proof passed (FR-023), and the deferred-findings and friction registers are current (FR-051, FR-052, FR-053). Open or update one PR against `v3`, identify its clean HEAD and tree as proposed F6, and run T384's authoritative integration obligation only through the heavy-gated `make test-host-integration` lane against that exact candidate. Require nonempty enumeration and successful no-skip builds of both `vmChecks.x86_64-linux.runtime-cloud-hypervisor-guest-acceptance` and `vmChecks.x86_64-linux.daemon-restart-vm-survival`; real KVM boot; the production controller's Provider-owned Cloud Hypervisor process effect; an authenticated guest-control session; the declared Guest's ready state; and FR-075's public lifecycle start/status/restart/same-runner-adoption/reachability/stop result. Emit exactly one passing `EvidenceRecord` with `validation = "w6-cloud-hypervisor-guest-acceptance"` bound to F6's candidate, commit, tree, and snapshot and containing both the Guest acceptance and continuity results. A fake VMM, direct controller call, another runtime family, declaration or bundle emission alone, status-only projection, actionable refusal, skip, empty discovery, a missing exact attr, or evidence from any other candidate is ineligible. Freeze that same clean HEAD and tree as F6 only with the passing record; T479 cannot complete without it. T479 MUST NOT issue a binding panel request, panel-attest, or seal. Any content change, slice merge, generated-output change, changelog fold, rebase, or acceptance-record identity change after F6 is frozen invalidates F6 and requires T479 and the acceptance lane to rerun.
- [ ] T480 [US2] W6 SINGLE BINDING WORK GATE + MERGE - depends on T479 including its exact-F6 `w6-cloud-hypervisor-guest-acceptance` record. Require HEAD and tree to equal clean F6, revalidate FR-036's external amendment ancestry, T221's unanimous plan-panel receipt and reviewed feature snapshot, the reviewed entry base as an ancestor of every W6 implementation head, and exactly one passing acceptance record bound to F6. Reinvoke the same closed acceptance predicate before pre-panel dispatch, panel request, panel-attest, seal, merge-target registration, merge eligibility, and merge; missing, duplicate, wrong-family, fake-boundary, skipped, empty, stale, or wrong-candidate evidence refuses each boundary. T480's work panel is not a substitute. Against F6, first dispatch the read-only reviewer Task lane and rubber-duck Task lane in parallel, each bound to `gpt-5.6-luna` / `max` / `long_context`; a content defect from either lane abandons F6 and returns to T479 before any binding panel request. Route the defect through scoped fixes, convergence, validation, and Guest acceptance, then iterate delta/full-context `/d2b-panel-round plan` phase reviews until the replacement provisional candidate has 10/10 sign-off with zero recommendations. Only that final candidate may receive W6's exactly one binding `/d2b-panel-round work` request; import its validation evidence, panel-request (refused unless every prior-wave work item is Merged and W6 is rebased after the predecessor merge), panel-attest (10/10 unanimous), seal (every prior-wave and wave item Merged), register merge-target, pass merge-eligibility, and then merge the already-open PR. A nonunanimous binding result permanently fails the W6 close: retain its candidate, request, findings, and records, issue no second binding request for any candidate, and stop with an integrator scope escalation; findings are not waived. From binding panel request through disposition, the final candidate and its tree are immutable. The merge MUST preserve the successful candidate's tree byte-for-byte. After merge, rebase the next wave onto updated `v3`, then clean up in order: delete each worktree `packages/target`, remove worktrees, delete local branches, delete remote branches, run `nix-collect-garbage`, and audit `git worktree list` plus `git branch -a` for residue.

**Checkpoint**: W6 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up.
Full US1 completion is placed here, specifically after T479/T480 accept
`Provider/runtime-cloud-hypervisor` on exact F6. The candidate-bound host-integration evidence
must show the declared Guest reaching the real Provider-owned Cloud Hypervisor process effect,
an authenticated guest-control session, and ready state alongside the previously accepted
`Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`. Missing,
skipped, status-only, fake-boundary, other-family, or
actionable-refusal evidence leaves US1 incomplete. Successor entry criteria satisfied.

---

## Wave W7: Feasibility closure, reset and cutover, security, streamline, delivery contract

**Requirements**: see spec-coverage.md traceability tables | **Story**: US3 | **Work items**: 73 | **Parallel groups**: 5

- [ ] T481 [US3] W7 PLAN PANEL + ENTRY - before any W7 implementation lane is dispatched, confirm Gate 0 passed, destinations are uncontended, the stack is proposed against the exact named parent commit, the heavy-gate semaphore is available, and the fast hermetic suite is green on the entry tree; then run `/d2b-panel-round plan` against that exact clean base and feature snapshot and require 10/10 sign-off with zero recommendations. If W6 is not yet merged, implementation entry additionally requires at least 5 of its 10 work reviews returned and green integration on its converged tree. A base or feature change before dispatch invalidates the plan receipt. This authorizes implementation only: T555's distinct work panel, seal, and merge eligibility remain blocked until W6 is sealed and merged and W7 is rebased onto the updated integration lineage (FR-057).

### Group `wi:ADR-046-feasibility-and-spikes` (10 items)

- [ ] T482 [US3] `ADR046-feasibility-002` - `proofs/process-fastlaunch-spike/` (adapt)
- [ ] T483 [P] [US3] `ADR046-feasibility-003` - `proofs/effectport-async-spike/` (adapt)
- [ ] T484 [P] [US3] `ADR046-feasibility-004` - `proofs/provider-packaging-spike/` (adapt)
- [ ] T485 [P] [US3] `ADR046-feasibility-005` - `proofs/bus-routing-noise-spike/` (adapt)
- [ ] T486 [P] [US3] `ADR046-feasibility-006` - `proofs/provider-state-export-spike/` (adapt)
- [ ] T487 [P] [US3] `ADR046-feasibility-007` - `proofs/process-provider-conformance-spike/` (adapt)
- [ ] T488 [P] [US3] `ADR046-feasibility-008` - `proofs/nix-authoring-spike/` (adapt)
- [ ] T489 [P] [US3] `ADR046-feasibility-009` - `proofs/cli-discovery-spike/` (adapt)
- [ ] T490 [US3] `ADR046-feasibility-010` - `proofs/e2e-composition-spike/` (adapt)
- [ ] T491 [US3] `ADR046-feasibility-011` - `proofs/test-runtime-budget-spike/` (adapt)

### Group `wi:ADR-046-reset-and-cutover` (11 items)

- [ ] T492 [P] [US3] `ADR046-reset-001` - `packages/d2b-cutover/src/{inventory,snapshot,checkpoint}.rs` (adapt)
- [ ] T493 [US3] `ADR046-reset-002` - `packages/d2b-cutover/src/{bundle_validate,trust_preflight}.rs` (adapt)
- [ ] T494 [US3] `ADR046-reset-003` - `packages/d2b-cutover/src/{consent,drain,disposition}.rs` (adapt)
- [ ] T495 [US3] `ADR046-reset-004` - `packages/d2b-cutover/src/adopt.rs` (adapt)
- [ ] T496 [US3] `ADR046-reset-005` - `packages/d2b-cutover/src/{store_bootstrap,provider_sequence}.rs` (create)
- [ ] T497 [US3] `ADR046-reset-006` - `packages/d2b-cutover/src/{zonelink_cutover,guest_activation}.rs` (adapt)
- [ ] T498 [US3] `ADR046-reset-007` - `packages/d2b-cutover/src/{verify,doctor,degraded}.rs` (adapt)
- [ ] T499 [US3] `ADR046-reset-008` - `packages/d2b-cutover/src/finalize.rs` (create)
- [ ] T500 [US3] `ADR046-reset-009` - `packages/d2b-cutover/src/{journal,rollback,hold}.rs` (adapt)
- [ ] T501 [US3] `ADR046-reset-010` - `packages/d2b-cutover/src/reset_scope.rs` (adapt)
- [ ] T502 [US3] `ADR046-reset-011` - `tests/integration/live/cutover-real-host.sh` (create)

### Group `wi:ADR-046-security-and-threat-model` (19 items)

- [ ] T503 [US3] `ADR046-security-001` - `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (adapt)
- [ ] T504 [US3] `ADR046-security-002` - `packages/d2b-session/tests/noise_conformance.rs` (adapt)
- [ ] T505 [US3] `ADR046-security-003` - `packages/d2b-resource-store/tests/rbac_property.rs` (adapt)
- [ ] T506 [US3] `ADR046-security-004` - `packages/d2b-bus/fuzz/fuzz_targets/zonelink_frame.rs` (adapt)
- [ ] T507 [P] [US3] `ADR046-security-005` - `packages/xtask/src/effectport_boundary_check.rs` (adapt)
- [ ] T508 [US3] `ADR046-security-006` - `packages/d2b-provider-system-minijail/tests/launchticket_toctou.rs` (adapt)
- [ ] T509 [US3] `ADR046-security-007` - `packages/d2b-contract-tests/tests/quarantine_not_kill_matrix.rs` (adapt)
- [ ] T510 [US3] `ADR046-security-008` - `packages/d2b-provider-system-core/tests/no_isolation_propagation.rs` (adapt)
- [ ] T511 [US3] `ADR046-security-009` - `packages/d2b-provider-volume-local/tests/marker_tamper_fault_injection.rs` (adapt)
- [ ] T512 [US3] `ADR046-security-010` - `packages/d2b-contract-tests/tests/zero_secret_invariant.rs` (adapt)
- [ ] T513 [US3] `ADR046-security-011` - `packages/d2b-provider-{clipboard-wayland,shell-terminal,device-security-key,notification-desktop}/tests/stream_redaction.rs` (adapt)
- [ ] T514 [US3] `ADR046-security-012` - `packages/d2b-audit/tests/privileged_fail_closed.rs` (adapt)
- [ ] T515 [US3] `ADR046-security-013` - `packages/d2b-bus/tests/dos_ceiling_fault_injection.rs` (adapt)
- [ ] T516 [US3] `ADR046-security-014` - `packages/d2b/src/commands/{doctor,support_bundle}.rs` (adapt)
- [ ] T517 [US3] `ADR046-security-015` - `packages/d2b-core-controller/src/reset.rs` (adapt)
- [ ] T518 [P] [US3] `ADR046-security-016` - `tests/unit/gates/security-matrix-coverage.sh` (adapt)
- [ ] T519 [US3] `ADR046-security-017` - `tests/integration/containers/malicious-child-zone.rs` (adapt)
- [ ] T520 [P] [US3] `ADR046-security-018` - `docs/reference/security-manual-validation-checklist.md` (adapt)
- [ ] T521 [US3] `ADR046-security-019` - `packages/d2b-contract-tests/tests/minijail_process_ownership.rs` (adapt)

### Group `wi:ADR-046-streamline` (24 items)

- [ ] T522 [P] [US3] `ADR046-streamline-001` - `docs/specs/ADR-046-spec-set.json` (create)
- [ ] T523 [US3] `ADR046-streamline-002` - `docs/specs/schemas/*.schema.json` (create)
- [ ] T524 [US3] `ADR046-streamline-003` - `packages/xtask/src/bin/spec_schema_check.rs` (create)
- [ ] T525 [US3] `ADR046-streamline-004` - `docs/specs/providers/TEMPLATE.md` (create)
- [ ] T526 [US3] `ADR046-streamline-005` - `packages/d2b-contract-tests/tests/policy_spec_vocabulary.rs` (create)
- [ ] T527 [US3] `ADR046-streamline-006` - `packages/d2b-resource-store-redb/tests/provider_state_graph.rs` (or the eventual crate implementing Zone resource storage) (create)
- [ ] T528 [US3] `ADR046-streamline-007` - `packages/d2b-contract-tests/tests/policy_effectport_boundary.rs` (adapt)
- [ ] T529 [US3] `ADR046-streamline-008` - `packages/d2b-contract-tests/tests/policy_work_items.rs` (create)
- [ ] T530 [US3] `ADR046-streamline-009` - `docs/specs/ADR-046-provider-catalog.md` (create)
- [ ] T531 [US3] `ADR046-streamline-010` - `tests/tools/reconcile-stale-base.sh` (reporting only) plus a documented `git town sync`/`git town` restack procedure this report feeds into (adapt)
- [ ] T532 [P] [US3] `ADR046-streamline-011` - `packages/xtask/src/bin/handoff_manifest.rs` (schema/validator only) (create)
- [ ] T533 [US3] `ADR046-streamline-012` - `tests/tools/import-task-db-consistency.sh` (create)
- [ ] T534 [US3] `ADR046-streamline-013` - `tests/tools/anti-serialization-report.sh` (adapt)
- [ ] T535 [P] [US3] `ADR046-streamline-014` - `tests/tools/run-layer.sh` extension (this repository already has `tests/tools/run-layer.sh` and `layer1-jobs.py` bounded-parallelism precedent) plus fake `EffectPort`/`ResourceClient` stub crates under `packages/d2b-provider-toolkit-fakes/` (adapt)
- [ ] T536 [US3] `ADR046-streamline-015` - Shared `packages/xtask` regeneration-conflict-detection helper consumed by every `gen-*`/`spec-registry` subcommand (adapt)
- [ ] T537 [US3] `ADR046-streamline-016` - `packages/d2b-contract-tests/tests/policy_no_leaked_decision_prefix.rs` (create)
- [ ] T538 [P] [US3] `ADR046-streamline-017` - `docs/specs/ADR-046-streamline-evidence-commands.md` (adapt)
- [ ] T539 [US3] `ADR046-streamline-018` - `tests/tools/worktree-disk-report.sh` (adapt)
- [ ] T540 [US3] `ADR046-streamline-019` - `packages/xtask/src/bin/terminology_check.rs` (`cargo run -p xtask -- terminology-check`) (create)
- [ ] T541 [US3] `ADR046-streamline-020` - `packages/d2b-contract-tests/tests/policy_test_placement.rs` (create)
- [ ] T542 [US3] `ADR046-streamline-021` - `packages/d2b-contract-tests/tests/policy_test_determinism.rs` (create)
- [ ] T543 [US3] `ADR046-streamline-022` - `packages/xtask/src/test_runtime_ledger.rs` (shared with `ADR046-delivery-007`) (adapt)
- [ ] T544 [US3] `ADR046-streamline-023` - `packages/xtask/src/bin/legacy_test_retirement.rs` (`cargo run -p xtask -- legacy-test-retirement`) (adapt)
- [ ] T545 [US3] `ADR046-streamline-024` - `packages/xtask/src/bin/implementation_graph.rs` (`cargo run -p xtask -- implementation-graph`) (create)

### Group `wi:ADR-046-validation-and-delivery` (9 items)

- [ ] T546 [P] [US3] `ADR046-delivery-001` - `packages/xtask/src/heavy_gate.rs` (adapt)
- [ ] T547 [P] [US3] `ADR046-delivery-002` - `packages/xtask/src/delivery/snapshot.rs` (adapt)
- [ ] T548 [US3] `ADR046-delivery-003` - `packages/xtask/src/delivery/validate_import.rs` (adapt). This task also owns the one hermetic `d2b-recovery-point-attestation` version 1 validator used unchanged by T580, T555, and T556. Decode each JSON timestamp directly into a bounded integer `RecoveryUnixSeconds` newtype with range 0 through 253402300799; sample verifier time once into the same type; use checked bounded addition for each 86,400-second deadline; and require `previewedAtUnix <= capturedAtUnix <= verifiedAtUnix <= attestedAtUnix <= verifierNowUnix < expiresAtUnix`. The table-driven suite starts from one valid canonical record and independently varies every required top-level and qualification field, every delivery binding, and every timestamp. It must include wrong `operatorSubjectSha256` and `restoreInstructionsSha256`; missing, duplicate, extra, malformed, and wrong-type cases; negative, fractional, and out-of-range cases for each of the six timestamp fields; future cases for each of the four event timestamps; checked-add overflow from capture and verification; retention/expiration mismatch; wrong candidate/commit/tree/preview/host/locator; and changed canonical bytes. The Cargo test-list command must succeed, discover at least one matching non-ignored test, discover zero ignored matching tests, and the tests must execute without skip. Empty discovery is failure. No close-stage task may copy or weaken this validator.
- [ ] T549 [US3] `ADR046-delivery-004` - `packages/xtask/src/gen_spec_set.rs` (adapt)
- [ ] T550 [US3] `ADR046-delivery-005` - `packages/xtask/src/delivery/panel.rs` (adapt)
- [ ] T551 [US3] `ADR046-delivery-006` - `packages/xtask/src/delivery/{seal,eligibility,history_proof}.rs` (adapt)
  T550 and T551 jointly own the closed failed-candidate reason
  `candidate-content-or-history-mismatch`. After a binding request exists, any commit, tree,
  history, merge-target, evidence-identity, or request-bound byte mismatch must use the same
  durable transition as nonunanimity and expiry: retain the request and every panel/seal
  record and file-and-directory-durably publish the terminal failed closure with that exact
  reason. The wave reservation remains permanent. Seal, eligibility, and merge must refuse;
  they may never return to convergence or admit another candidate after the binding request.

  Inject crashes before and after failed-record file sync, no-replace publication, directory
  sync, and terminal-disposition publication. On every restart, the mismatch candidate is
  closed once, its request/history remains immutable and readable, the wave reservation is
  retained, and no same-candidate or alternate-candidate retry is possible. The matrix
  independently covers content bytes, commit/tree identity, history-only rebase,
  merge-target identity, and evidence refresh at panel-attest, seal, merge-target,
  merge-eligibility, and merge.
- [ ] T552 [P] [US3] `ADR046-delivery-007` - `packages/xtask/src/test_runtime_ledger.rs` (adapt)
- [ ] T553 [US3] `ADR046-delivery-008` - `docs/specs/ADR-046-implementation-graph.json` (adapt)
- [ ] T554 [US3] `ADR046-delivery-009` - `packages/xtask/src/gen_spec_set.rs` (adapt)

- [ ] T580 [US3] **Converge and freeze W7, then prove the qualified recovery-point attestation gate** (FR-043, SC-025). Depends on every task row in the five W7 implementation groups above, whose boundary task IDs are T482 and T554, not only T494, T500, and T502; the integrator MUST mechanically require all 73 group rows checked. Before attestation, merge every W7 slice, reconcile generated manifests and reference/changelog fragments, rebase after sealed W6, run integration tests and CI, resolve every content-changing result, open or update the one W7 PR against `v3` and record its identity, then freeze one clean candidate id, full commit OID, and full tree OID as provisional F7c. A prebinding phase finding or evidence expiry may return here for another provisional identity; no return or successor is permitted after W7's binding request. No slice branch remains to merge after F7c.
  A qualifying point is an operator-owned external `full-host-snapshot` or `full-host-backup` that covers boot/system configuration, the active NixOS generation, every exact preview-inventory artifact, and preserved identity state; targets the same daily-driver host; remains read-only through expiration; has available restore instructions; and passed post-capture external `snapshot-readback` or `backup-verify`. A d2b-only export, checkout, unverified copy, or partial path backup fails.
  Require one canonical `d2b-recovery-point-attestation` version 1 record with exactly the FR-043 fields and values. Its candidate id, commit OID, tree OID, preview digest, and domain-separated daily-driver `/etc/machine-id` digest must match F7c and the live host. All five closed qualification booleans must be true; verification and result must be `passed`; no raw host id, uid, username, recovery locator, restore instructions, or payload may enter delivery evidence.
  Freshness is mechanical and uses only T548's bounded timestamp newtypes and checked
  arithmetic. Decode all six timestamp fields as JSON integers in 0 through 253402300799,
  sample verifier now once into the same type, and require
  `previewedAtUnix <= capturedAtUnix <= verifiedAtUnix <= attestedAtUnix <= verifierNowUnix < expiresAtUnix`.
  Compute capture plus 86,400 and verification plus 86,400 with checked bounded addition;
  expiration must equal the minimum of those two results and `retentionUntilUnix`. Import and
  every later boundary check occur strictly before expiration. A negative, fractional,
  future event time, out-of-range value, checked-add overflow, clock reversal, expiry,
  host/preview/candidate/commit/tree change, or changed canonical record bytes invalidates
  the evidence.
  Run T502's `tests/integration/live/cutover-real-host.sh` FR-043 matrix against exact F7c, but
  use T548's hermetic validator as the oracle rather than reimplementing its predicates in
  shell. The hermetic matrix starts from one valid record and independently omits, duplicates,
  or mutates every required top-level field, every qualification member, and every delivery
  binding, explicitly including wrong `operatorSubjectSha256` and
  `restoreInstructionsSha256`. It covers absent, extra, duplicate, failed, malformed,
  wrong-type, partial-coverage, wrong-host, wrong-operator, wrong-restore-instructions,
  wrong-candidate, wrong-commit, wrong-tree, wrong-preview, stale, expired, future-event,
  negative, fractional, out-of-range, checked-add overflow, clock-order, external-locator,
  and post-freeze-change refusals and proves no post-boundary step executes in each negative.
  Validator test discovery must succeed, be nonempty, contain zero ignored matching tests,
  and execute without skip. Import exactly one F7c-bound `EvidenceRecord` with
  `validation = "recovery-point-attestation"` and `result = "passed"`; `candidate_id`,
  `content_id`, and `snapshot_sha256` bind F7c, while `output.sha256` and `output.bytes`
  identify the exact canonical external record, `command` names only the verifier command,
  and an opaque `locator` resolves that record.
  **Done when** all 73 group rows are checked; all slice heads are ancestors of F7c; generated/reference/changelog reconciliation, integration, and CI pass on clean F7c; the full refusal/recording matrix exits zero; exactly one current record satisfies every FR-043 identity, qualification, chronology, freshness, expiration, digest, and external-resolution predicate; and T580 is checked. Any mismatch leaves T580 incomplete and blocks T555. This task verifies and imports operator evidence; it does not implement, create, retain, or restore the external host snapshot or backup.
  The W7 PR identity recorded before F7c froze is the sole merge vehicle. PR creation,
  update, or retargeting after freeze is forbidden; T555 may only register the frozen merge
  target and evaluate eligibility, and T556 may only merge this already-open PR.
  A return from T555 before the binding request, due to a phase finding or
  `recovery-attestation-expired`, starts a new provisional T580 iteration. Rerun convergence
  and the complete validation set, open or update the PR before the new freeze, allocate a
  distinct provisional candidate identity, and import a newly created canonical attestation.
  After the binding request, nonunanimity, `candidate-content-or-history-mismatch`, or
  `recovery-attestation-expired` terminally fails W7 and cannot return here or transfer any
  approval to another request.

- [ ] T555 [US3] W7 SINGLE BINDING WORK GATE - depends on T580. Before dispatching either native pre-panel lane, require HEAD and tree exactly the current W7 provisional candidate, a clean index/worktree, T580 checked, exactly one passing `recovery-point-attestation`, T481's unanimous plan-panel receipt with its reviewed feature snapshot, and ancestry from the reviewed W7 entry base to every W7 implementation head. T555's work panel is not a substitute. Invoke T548's same hermetic validator - never a stage-local predicate copy - before phase review, panel request, panel-attest, seal, merge-target registration, and merge eligibility. It must validate every FR-043 field and delivery binding, the candidate/commit/tree/preview/live-host/operator/restore-instruction digests, all qualification fields, locator resolution, bounded integer timestamp decoding, checked 86,400-second expiration arithmetic, and `previewed <= captured <= verified <= attested <= verifier-now < expires`. Missing, extra, failed, malformed, duplicate, wrong-type, negative, fractional, future-event, out-of-range, overflow, stale, expired, wrong-host, wrong-operator, wrong-restore-instructions, wrong-preview, wrong-candidate, wrong-commit, wrong-tree, unresolvable, post-freeze, empty validator discovery, ignored, or skipped state refuses the stage. Run the native Copilot pre-panel procedure against that candidate: dispatch a read-only reviewer Task lane and rubber-duck Task lane in parallel, each bound to `gpt-5.6-luna` / `max` / `long_context`. A content/history defect or pre-request attestation expiry abandons the provisional candidate and returns to T580 for scoped correction, fresh evidence, validation, and a delta/full-context `/d2b-panel-round plan` phase review. Iterate that nonbinding phase surface to 10/10 sign-off with zero recommendations before selecting the final candidate. Only then issue W7's exactly one binding `/d2b-panel-round work` request; import final-candidate validation evidence, panel-request (refused unless every prior-wave work item is Merged and W7 is rebased after the predecessor merge), panel-attest (10/10 unanimous), seal (every prior-wave and wave item Merged), register merge-target, and pass merge-eligibility. Nonunanimity, attestation expiry, or any post-request content/history/binding mismatch durably fails the W7 close and retains its request, findings, and records. Issue no second binding request for any candidate and stop for integrator scope escalation; findings are not waived. From binding panel request through durable disposition, the final candidate and tree are immutable. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), every removal proof for a path retired in this wave passed (FR-023), and register/friction updates are already in the candidate (FR-053). From round 9, LOW/MEDIUM may be deferred to deferred-findings.md; CRITICAL/HIGH never (FR-051, FR-052).
- [ ] T556 [US3] W7 MERGE-ONLY - depends on the successful T555 binding result. Refuse unless HEAD and tree still equal that exact approved W7 candidate, every slice head is already its ancestor, T580 remains checked, and T548's same validator still accepts the sole `recovery-point-attestation` at T556 entry and immediately before merge. Also require the T580-opened PR still targets `v3` at that candidate and panel/seal/merge-target/merge-eligibility all remain valid. T556 MUST NOT open, update, or retarget a PR; merge a slice; fold a changelog fragment; regenerate content; run a content-changing command; rerun integration/CI as a convergence step; refresh expired evidence in place; or issue another panel request. Merge only the already-open T580 PR and only if the resulting merge preserves the approved tree byte-for-byte. A content, commit/tree, history, merge-target, evidence-identity, or attestation-expiry failure after the binding request retains the request/panel/seal records, durably fails the W7 close, permits no successor request, and stops for integrator scope escalation. T550/T551 crash tests must prove terminal failure survives every publication crash point before T556 may complete. After merge, rebase the next wave onto updated `v3`, then clean up in order: delete each worktree `packages/target`, remove worktrees, delete local branches, delete remote branches, run `nix-collect-garbage`, and audit `git worktree list` plus `git branch -a` for residue.

**Checkpoint**: W7 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W8: Friction closure (terminal wave)

**Story**: US4 | **Work items**: recorded at W7 close | **Parallel groups**: determined by T557 at W7 close

W8 has **no spec members and no work items yet, by design**. Its contents are the delivery
friction accumulated across W0 through W7 - in the categories signoff, build, test, merge,
codegen, and disk - triaged at W7 close. Its destinations are `packages/xtask/`,
`tests/tools/`, `packages/d2b-contract-tests/tests/`, and `Makefile`.

It runs the same wave template unchanged, including exactly one binding ten-role panel.

- [ ] T557 [US4] W8 TRIAGE - collect and classify friction from every prior wave into the six categories; record the resulting work items in the manifest
- [ ] T558 [US4] W8 PLAN PANEL + ENTRY - depends on T557. After T557 fixes the triaged work-item set, file ownership, and validation map, and before any W8 implementation lane is dispatched, confirm Gate 0 passed, every fixed destination carries no open contention flag, the stack is proposed against the exact named parent commit, the heavy-gate semaphore is available, and the fast hermetic suite is green on the entry tree; then run `/d2b-panel-round plan` against that exact clean W8 implementation base and feature snapshot and require 10/10 sign-off with zero recommendations. If W7 is not yet merged, implementation entry additionally requires at least 5 of its 10 work reviews returned and green integration on its converged tree. A task-map, ownership, feature, or base change before dispatch invalidates the receipt and requires another plan review. This authorizes implementation only: T565's distinct work panel, seal, and merge eligibility remain blocked until W7 is sealed and merged and W8 is rebased onto the updated integration lineage (FR-057).
- [ ] T559 [US4] W8 IMPLEMENT + CONVERGE - depends explicitly on T557 and the passing T558 plan gate. Execute the triaged items (count known only after T557), merge every W8 slice into the wave integration branch, rebase after the sealed W7 merge, run integration tests and CI on the converged tree, and resolve every content-changing result. No slice branch may remain to merge after T559, and T559 MUST NOT issue a binding work-panel request, panel-attest, or seal.

W8 and Phase R share one final candidate F8. All repository content needed for release,
including the changelog fold, version header, release summary, and retirement list, lands
before T560 freezes F8. Candidate-bound validation follows the freeze but precedes the one
binding panel. A candidate-bound failure that needs content changes abandons F8 and returns
through the owning pre-freeze task, T566, and T560 before validation is rerun. Once the
binding panel request is issued, that candidate is immutable. A unanimous candidate leaves
only merge work; a nonunanimous binding result terminally fails W8, permits no second
request, and cannot enter T561.

The executable pre-freeze DAG is `T557 + T558 -> T559 -> T571 ->
{T562,T568,T570,T572} -> T566 -> T560`. T571 publishes the retirement list before the four
candidate-sensitive conditions run. Any content or history change after one of
T562/T568/T570/T571/T572 completes invalidates all five completions and their evidence,
requires T559 convergence to be re-established, and requires all five to rerun. T566's own
final release-content commit therefore reruns all five checks read-only against its resulting
HEAD and then reruns integration and CI against that exact final HEAD/tree before T560 may
freeze. Any content change from T566 or either post-T566 run restarts convergence at T559,
then T571, all four candidate-sensitive conditions, T566, and both post-T566 runs. T560
mechanically rejects differing commit/tree bindings, pre-T566 integration/CI results, missing
rerun records, or declaration-order-only satisfaction.

### Pre-freeze release conditions

- [ ] T562 [US4] Condition 1 - depends on T571. Before F8 is frozen, confirm the five closing specs are Accepted and import their work items' validation evidence
- [ ] T568 [US4] Depends on T571. Before F8 is frozen, confirm the companion inventory published at W5 (T577) is still accurate for the release candidate; a companion added or changed since W5 must be caught and the inventory update committed here. Re-derive the set under FR-064's two limbs rather than re-reading the published table: enumerate the validation host's flake inputs, keep every currently published row, and add any repository a reference doc, example, template, or how-to names as consuming a d2b surface; then confirm each candidate consumes at least one surface on the closed public list. Each row carries repository, pinned commit (not a tag or version string), maintainer of record, discovery source, and consumed surfaces. A removal requires a recorded negative determination at a named revision and date; a candidate whose consumption cannot be determined stays in the set and blocks. A candidate found reading a private `root:d2bd` `0640` bundle artifact is reported as a defect and not admitted
- [ ] T570 [US4] Depends on T571. Before F8 is frozen, confirm capability parity for every path whose migration disposition promised a successor (FR-041); any correction lands before T560
- [ ] T571 [US4] Depends on T559. Before F8 is frozen, publish the explicit retirement list with justifications, and name each retirement in the release notes (FR-042). Per FR-063 this list is also the **only** lawful path to shipping with a companion surface that failed its T569 exercise: each such entry carries a justification, a named owner, the condition that would restore the surface, and a release-note line, is unavailable where FR-041 promised a successor, and must be decided **before** the tag - a failed exercise relabelled afterwards is not a retirement. The published inventory row must not read as verified while its surface is retired. If T569 exposes a lawful retirement not already recorded, abandon F8 and return through T571, T566, and T560 before rerunning every candidate-bound condition; never edit the panel-bound F8.
- [ ] T572 [US4] Depends on T571. Before F8 is frozen, confirm zero foundation surfaces remain deliberately unwired from production (SC-021); any correction lands before T560
- [ ] T566 [US4] Condition 5 - depends on T559, T562, T568, T570, T571, and T572. Fold every W8 changelog fragment, make `CHANGELOG.md` carry the new version header and version-level summary, strip every internal wave, phase, and finding marker, run the focused changelog validation, and commit that final release content before F8 is frozen. Then rerun T562, T568, T570, T571, and T572 read-only and require all five records to bind this exact final pre-freeze HEAD and tree. Run integration and CI after those release bytes are committed and require both results to bind the same exact HEAD/tree. Any content-producing result restarts T559 and the complete downstream pre-freeze sequence; a result against the earlier T559 tree is not evidence.
- [ ] T560 [US4] W8 FREEZE - depends on T559, T562, T566, T568, T570, T571, and T572. Require every W8 slice head to be an ancestor of the converged branch, all changelog fragments folded by T566, every pre-freeze release artifact committed, all five release-condition reruns bound after T566, integration and CI green from the mandatory post-T566 runs against this exact final HEAD/tree, and a clean index/worktree. Reject any result bound only to T559 or any earlier release-content tree. Open or update one PR against `v3`, then freeze that exact clean HEAD and tree as F8. T560 MUST NOT issue a binding panel request, panel-attest, or seal. Any content change, slice merge, generated-output change, changelog fold, or rebase after F8 is frozen invalidates F8 and all F8-bound evidence and restarts T559 plus the complete post-T566 validation path before T560 may freeze a successor.

### Exact-candidate release conditions and close

- [ ] T563 [US4] Condition 2 - depends on T560. Every DELETE and REPLACE row's removal proof passes on exact F8, not merely when it was first established
- [ ] T564 [US4] Condition 3 - depends on T560. The complete validation matrix passes against exact F8, including the manual hardware, live-host, and cloud tiers at least once with recorded external evidence, plus the reset and cutover scenarios
- [ ] T567 [US4] Condition 6 - depends on T560. Every prior wave's cleanup is done; no dangling implementation worktrees or branches remain
- [ ] T569 [US4] Depends on T560. Verify each companion by exercising it against exact F8 on a live host - `d2b-toolkit`, `d2b-wlterm`, `d2b-wlcontrol`, `d2b-clip-picker`; `weezterm` consumes no d2b contract (FR-040, SC-024). The set exercised is the one T568 re-derives under FR-064, not this task's illustrative list. All seven FR-065 conditions must hold: live host and not a VM, container, or CI runner; the exact candidate snapshot named by commit; the companion at a pinned commit; every surface in the row exercised rather than sampled; every surface Conformant or Retired under FR-063; zero Blocked including zero unclassifiable; evidence in FR-063's shape. Source inspection, a version or tag match, a green docs check, a successful build, a green CI run in the companion's own repository, an exercise against a non-candidate build, an exercise off the live host, a partial exercise, and the fact that the contracts were published at W5 are each explicitly not evidence. A capability-conditional refusal is Conformant only if it names the false capability key or refusal state and at least one concrete operator action - a silently greyed control is Blocked. **If F8 moves for any reason, every verification recorded against the previous snapshot is void and must be re-run.** A failure here is the detection event FR-062 names, and its response is to hold the release, abandon F8 for a pre-panel correction, or amend FR-045, never to relax FR-040
- [ ] T565 [US4] Condition 4 + W8 SINGLE BINDING WORK GATE - depends on T560, T563, T564, T567, and T569. Require HEAD and tree to equal clean provisional F8, every release condition and evidence record to name F8, T558's unanimous entry plan-panel receipt to match the current feature snapshot, and the reviewed W8 entry base to be an ancestor of every W8 implementation head. T565's work panel is not a substitute. Against F8, first run the native read-only reviewer and rubber-duck pre-panel lanes; a content defect abandons provisional F8 and returns through the owning pre-freeze task, T566, and T560 before any binding panel request. Route findings through scoped fixes and the complete post-T566 integration/CI and release-condition sequence, then iterate delta/full-context `/d2b-panel-round plan` phase reviews to 10/10 sign-off with zero recommendations before selecting final F8. Only then issue W8's exactly one binding ten-role `/d2b-panel-round work` request, require unanimous sign-off with zero recommendations against F8, panel-attest, seal, register merge-target, and pass merge-eligibility. A nonunanimous binding result permanently fails the W8 close: retain F8, its request, findings, and records, issue no second binding request for any candidate, and stop for integrator scope escalation; findings are not waived. From binding panel request through disposition of F8, F8 and its tree are immutable.
- [ ] T561 [US4] W8 MERGE-ONLY - depends on T565. Refuse unless HEAD and tree still equal exact F8 and panel, seal, and merge eligibility remain valid. T561 MUST NOT merge a slice, fold a changelog fragment, edit or regenerate content, rebase, rerun integration/CI as convergence, or issue another panel request or panel round. Merge the already-open PR against `v3` only if the merge preserves F8's tree byte-for-byte. Any pre-merge content or history change blocks W8 and requires integrator escalation rather than another binding panel. After merge, clean up external worktrees, branches, targets, and the Nix store, and audit for residue.

**Checkpoint**: the exact F8 tree that ships is merged to `v3`.

---

## Phase R: Publish d2b 3.0

**Story**: US4. Publication consumes the merged **W8** candidate, not W7 - gating earlier would
release a candidate a later wave still modifies.

- [ ] T573 [US4] Depends on T561. Confirm the merged `v3` tree is byte-identical to sealed F8, then tag and publish d2b 3.0 without changing release content

**Checkpoint**: d2b 3.0 released.

---

## Pipelined wave execution

Panel review commonly runs **one to two times the coding duration**. Strictly serializing
review after implementation would idle the implementation capacity for more than half of every
cycle. Waves are therefore **pipelined**: the next wave starts coding while the current wave is
still in review, but nothing about the review gate is weakened.

### The pipeline

```text
W(N)   code ──> converge ──> integration tests ──> native Task review + rubber-duck ──> panel (10 lanes) ──> seal 10/10 ──> merge to v3
                                    │                                                      │
                                    │ 5 of 10 panels back                                  │
                                    │ + integration green                                  │
                                    ▼                                                      ▼
W(N+1)                            code ──> converge ──> integration tests ──> native Task review + rubber-duck ──> rebase on v3 ──> panel ──> seal ──> merge
```

### The four conditions (all required)

A wave may begin implementation early only when:

1. At least **5 of the 10** predecessor panel lanes have returned, **and**
2. the predecessor's **integration tests pass** on its converged tree, **and**
3. the successor issues **no panel request, no seal, and no merge** until the predecessor is
   sealed at 10/10 with zero recommendations and merged, **and**
4. the successor **rebases onto the updated `v3`** after that merge and **before** its own
   panel runs.

Condition 4 is what makes this safe: the successor's panel always binds to a snapshot that
already contains every finding the predecessor's panel produced. A panel never reviews a tree
built on unreviewed contracts.

### What is pipelined and what is not

| Activity | Pipelined? |
| --- | --- |
| Implementation coding | **Yes** - starts at 5 of 10 |
| Slice convergence onto the wave branch | **Yes** |
| Integration testing | **Yes**, subject to the 2-slot heavy-gate ceiling |
| Pre-panel native Task review and rubber-duck lanes | **Yes** - read-only, no heavy-gate slot |
| Panel request | **No** - strictly after predecessor seal + merge |
| Seal | **No** - strictly ordered |
| Merge to `v3` | **No** - strictly ordered |

### Accepted cost: rework

A predecessor finding that changes a contract may invalidate in-flight successor work. That
rework is **absorbed by the wave that started early** and is the explicit price of the
pipeline. It MUST NOT be used as an argument to weaken, shorten, or partially accept the
predecessor's panel (FR-050). If rework becomes chronic, the correct response is to start
later, not to review less.

### Governance status

This model required a constitution amendment. Principle VI was redefined in **constitution
2.0.0** to permit pipelined dispatch under exactly these four conditions. T585-T588 landed the
matching ADR-046 delivery-spec and tooling changes: only implementation start is pipelined;
panel request, seal, and merge remain strictly ordered.

---

### Pre-panel native review lanes

After a wave's slices converge and integration tests pass, but **before** any panel lane is
dispatched, the wave runs two read-only Copilot Task lanes **in parallel**:

```text
slices converge ──> integration tests ──┬──> reviewer Task ────────┐
                                        └──> rubber-duck Task ─────┴──> iterative /d2b-panel-round plan
                                                                          │ 10/10, zero findings
                                                                          └──> one /d2b-panel-round work ──> seal
```

| Lane | What it checks | Scope |
| --- | --- | --- |
| reviewer Task | The implementation against `spec.md`, `plan.md`, `tasks.md`, and the constitution; constitution conflicts are automatically CRITICAL | The feature directory plus the converged wave tree |
| rubber-duck Task | Code quality, tests, errors, types, comments, and simplification risks | The wave's diff only |

Issue two separate native Copilot Task invocations together in one coordination cycle: one
reviewer lane and one rubber-duck lane. Each invocation creates exactly one read-only lane and
must carry the explicit binding `model: gpt-5.6-luna`, `reasoning_effort: max`, `context_tier: long_context`.
Content findings iterate through the nonbinding `/d2b-panel-round plan` phase surface and
scoped fix rounds. Only after phase convergence does the final candidate receive the wave's
one `/d2b-panel-round work` binding request, whose ten read-only seats bind
`github-copilot`, `gpt-5.6-sol`, `xhigh`, and `default` in its committed table.
No dotted verification or review command is part of this repository's process.

### Scoping the native lanes

The native lanes receive an explicit file list or retrieval instruction in their prompts.
Target the wave's own diff:

```bash
# The wave's changes only: integration branch vs its actual base
git diff --name-only $(git merge-base v3 adr046-w<N>-integrate)..adr046-w<N>-integrate

# When stacked on an unmerged predecessor, the base is that branch, not v3
git diff --name-only adr046-w<N-1>-integrate..adr046-w<N>-integrate
```

### Why this runs before the panel, not after

Both lanes are cheap and read-only. The panel is ten reviewer lanes that commonly
cost one to two times the coding duration. Sending a wave to panel with defects these lanes
would have caught spends the most expensive review capacity on the cheapest findings, and a
finding that arrives during panel forces a content change, which invalidates the snapshot and
every validation and panel record bound to it.

### Disposition of findings

- **Actionable finding from either lane, at any severity** - blocking. Fix it and return
  through convergence before dispatching the panel. Every constitution conflict is CRITICAL.
- **Nonblocking observation from either lane** - record it only in that lane's summary. It is
  not a finding, recommendation, or deferred item and does not enter `deferred-findings.md`.
  The round-nine LOW/MEDIUM deferral rule applies only to `/d2b-panel-round` rounds, never to
  these native pre-panel gates.
- Anything either gate raises that is actually a process problem rather than a code problem
  belongs in the friction log.

---

## Panel convergence and delivery memory

Panel rounds are bounded. Under Constitution 2.2.0, from **round nine onward**, a reviewer may
classify a LOW or MEDIUM finding as **deferred** instead of blocking; CRITICAL and HIGH never
become deferrable (FR-051).

Deferral is a re-filing, not a dismissal. The finding moves **out of `recommendations`** into
[deferred-findings.md](./deferred-findings.md). That is what keeps `signoff` true if and only
if `recommendations` is empty - an invariant enforced in code at `panel.rs:277` in both
directions, so a deferred finding left in `recommendations` is rejected by `panel-attest`.
**No tooling change is required for this rule**, which is why it is shaped as a deferral
rather than as a relaxation of the sign-off check.

Two registers are maintained continuously, not written at the end:

| Register | Purpose | Feeds |
| --- | --- | --- |
| [deferred-findings.md](./deferred-findings.md) | Every deferred LOW/MEDIUM finding with its disposition | Next wave's planning; W8 triage |
| [friction-log.md](./friction-log.md) | What slowed delivery, in W8's six categories | W8 triage directly |

Neither may contain panel transcripts, command output, or attestation payloads - metadata only.

Per-wave obligations, folded into each wave's GATE task:

- Record any deferral at the moment it is made, not at wave close
- Review both registers at wave close; schedule or withdraw stale deferrals
- Log friction in the wave where it was felt, categorized on entry
- If the same friction category recurs across three waves, promote it to a task

---

## Branch, worktree, and convergence model

### Branch topology

Two tiers. Slice branches carry parallel subagent work; one integration branch per wave is the
convergence point and the only thing that becomes a PR against `v3`.

```text
v3
 └── adr046-w2-integrate          <- wave integration branch, PR target = v3
      ├── adr046-w2-primitives    <- slice branch, own worktree, own subagent
      └── adr046-w2-routing       <- slice branch, own worktree, own subagent

      └── adr046-w3-integrate     <- next wave STACKED on the unmerged w2 branch
           └── adr046-w3-provider
```

- **One worktree and branch per wave** for integration: `adr046-w<N>-integrate`.
- **One worktree and branch per slice**: `adr046-w<N>-<slice>`, cut from that wave's
  integration branch, never from `v3` directly.
- The next wave's integration branch is **stacked on the previous wave's integration branch
  while it is still unmerged**, and is re-pointed at `v3` once that merge lands.
- **All merges target `v3`.** Never `main`, never a local octopus merge, never a direct push.

### Convergence

Every parallel slice in a wave converges on that wave's integration branch **before** any of
the following runs. None of them are per-slice activities:

1. **Integration testing** - `make test-integration` and `make test-host-integration` against
   the converged tree, through the heavy-gate semaphore (2 concurrent lanes maximum)
2. **Panel review** - 10 read-only subagent lanes against the converged snapshot
3. **PR and CI** - one PR per wave integration branch, required checks green

Running any of these against an individual slice is wasted work: the snapshot the panel binds
to and the tree CI validates are the converged tree, not a slice.

### Wave lifecycle

```bash
# 1. Create the wave integration worktree, stacked on the prior wave if still unmerged
git worktree add -b adr046-w3-integrate ../d2b-w3 adr046-w2-integrate   # or v3 if w2 merged

# 2. Fan out one slice worktree per parallel group, cut from the integration branch
git worktree add -b adr046-w3-<slice> ../d2b-w3-<slice> adr046-w3-integrate

# 3. Subagents work in parallel, each confined to its own worktree and destinations

# 4. Converge: merge each slice into the wave integration branch
git -C ../d2b-w3 merge --no-ff adr046-w3-<slice>

# 5. Integration tests, panel, PR/CI against the converged branch

# 6. After the parent wave merges to v3, rebase this wave onto the updated v3
git -C ../d2b-w3 rebase v3
```

**Rebase invalidation**: a rebase changes history. Panel records may be reused **only** if the
byte-identical history proof passes (identical integrated content, identical generated
artifacts, identical dependency diff and repository set). Required CI **always** reruns on the
new history regardless. Treat a rebase that touches content as a re-snapshot.

### Cleanup (mandatory, not optional)

After a wave's PR merges, in this order:

```bash
# 1. Delete each slice worktree's real target dir FIRST or the removal reclaims nothing
rm -rf ../d2b-w3-<slice>/packages/target
git worktree remove ../d2b-w3-<slice>

# 2. Local branches
git branch -d adr046-w3-<slice>
git branch -d adr046-w3-integrate

# 3. Remote branches
git push origin --delete adr046-w3-<slice> adr046-w3-integrate

# 4. Store reclamation
nix-collect-garbage

# 5. Audit for orphans left by abandoned or superseded work
git worktree list
git branch -a | grep adr046-
```

A wave is not closed until `git worktree list` and `git branch -a` show no residue for it. The
release gate re-checks this: condition 6 requires the release to cut from a tree with no
dangling implementation worktrees or branches.

---

## Parallel subagent execution model

This plan is executed by a coding agent that dispatches **subagents**. Parallelism is the
default, not an optimization: the delivery contract makes launching every ready, file-disjoint
slice in the same coordination cycle a **positive obligation**, and a launch count below the
ready count without a recorded blocker fails wave entry criteria.

### Dispatch rule

When several slices are ready and file-disjoint, dispatch them **in a single message with
multiple subagent calls**. Sequential dispatch of independent slices is a process failure to
correct, not a scheduling preference.

### Per-wave implementation fan-out

One write-capable subagent per parallel group, each in its **own git worktree** so no two
agents share a working tree or a `packages/target/`:

| Wave | Groups | Concurrent implementation subagents |
| --- | --- | --- |
| W2 | 2 | **2** - zero file-overlap edges; both start together |
| W3 | 1 | **1** - strictly serial by design |
| W4 | 6 | **6** |
| W5 | 12 manifest groups + completion graph | **12** for the original manifest groups; after T589, T590/T591/T594 start together and the T591 branch continues as T591 -> T592 -> T593 -> T605, then daemon composition waits for T590/T592/T594/T605, followed by **5** file-disjoint acceptance/docs slices including T604 |
| W6 | 29 | **up to 28** - 27 Provider-dossier groups plus process-provider integration and core-controller coordination; T039 is never elided |
| W7 | 5 | **5** |

Worktree setup per slice (cut from the wave integration branch, never from `v3`):

```bash
git worktree add -b adr046-w<N>-<slice> ../d2b-w<N>-<slice> adr046-w<N>-integrate
```

Before removing a worktree, delete its real `packages/target/` or the removal reclaims
nothing. Compiled-output dedup across worktrees comes from `sccache`, not a shared target dir.

### Panel fan-out

Panels run as **10 read-only subagent lanes on `gpt-5.6-sol` at `xhigh`**, one
per roster role (`software`, `test`, `nixos`, `networking`, `security`, `rust`,
`product`, `docs`, `observability`, `kernel`), dispatched together in one
message.

- Lanes are **read-only by contract**. They inspect the diff, the plan, and the integrator's
  supplied validation evidence. They MUST NOT run tests, builds, evals, or long validations
  unless the integrator explicitly asks a specific lane to.
- Because they are read-only they take **no heavy-gate slot**, so all 10 run concurrently
  without contending for the semaphore.
- Each lane's verdict maps one-to-one onto a `panel-attest` record. `signoff` is true **iff**
  `recommendations` is empty.
- Every lane must run on the pinned model. A lane that silently falls back to another model
  produces a record `panel-attest` rejects, which blocks the seal.

### Hard concurrency ceilings

| Resource | Ceiling | Consequence of exceeding |
| --- | --- | --- |
| Heavy lanes (Layer 2, hardware, live, perf) | **2 per uid**, enforced by the OFD-locked semaphore | Blocks up to 30 minutes, then fails closed. Never add a second lock or a retry loop |
| Read-only panel lanes | No semaphore limit | None |
| Layer-1 subagent builds | Bounded by disk, not by lock | Each worktree carries its own multi-GiB `packages/target/` |
| Free disk under the repo root | **10 GiB minimum**, fail-closed preflight | Wave fails at the preflight guard |

The practical consequence: **implementation fan-out can be wide, heavy validation cannot.**
Dispatch 27 Provider subagents freely, but serialize their `make test-integration` and
`make test-host-integration` runs through the gate.

### What each subagent needs in its prompt

1. For each work-item id, the complete 15-field object retrieved verbatim from the canonical
   manifest; never dispatch from the task row's destination label
2. Its exact `destination` field from that object, with an instruction to write nowhere else
3. Its worktree path and branch
4. The reminder that unordered contended files are integrator-owned. A shared file may instead
   have explicitly ordered serial slice owners only when this plan names each owner and the
   dependency edge. `CHANGELOG.md` is never edited by a slice - each writes one
   `changelog.d/<branch>.md` fragment instead
5. The qualified commit-tag form: `( adr046w<n> )`, or
   `( adr046w<n>fu<m> <S><n> )` for a finding fix. Current Wave 5 work uses
   `( adr046w5 )`; legacy `ADR046-W<n>` evidence identifiers are not rewritten

### Integrator-only work (never delegated to a slice subagent)

Contended files, the spec-set and work-item manifests (last commit of each wave), the
changelog fold, the wave snapshot/panel/seal/merge sequence, and worktree cleanup.

---

## Dependencies and execution strategy

### Between waves - strictly sequential

```text
Phase 0 -> W2 -> W3 -> W4 -> W5 -> W6 -> W7 -> W8 -> Release

Pipelined: W(N+1) coding starts at 5 of 10 W(N) panels + green integration.
W(N+1) panel/seal/merge remain strictly after W(N) seal + merge + rebase.
```

Panel, seal, and merge are strictly ordered. Only implementation start is pipelined (FR-048,
FR-049, FR-050; constitution 2.0.0 Principle VI).

### Within a wave - maximize parallelism

Parallel groups are file-disjoint by construction. Launch every ready group in the same
coordination cycle; a launch count below the ready count without a recorded blocker **fails
wave entry criteria**.

| Wave | Groups | Parallelism note |
| --- | --- | --- |
| W2 | 2 | Zero file-overlap edges; both groups start together. The 3 `primitives` items have no intra-wave dependency at all |
| W3 | 1 | Strictly serial by design; every Provider dossier depends on it |
| W4 | 6 | Five parallel member-spec groups plus `core-config-hub:w4`; all six start together |
| W5 (`adr046w5`) | 12 manifest groups + completion graph | T603 is the two-pass receipt/editor-mediated resume gate and sole in-feature direct prerequisite of T589; FR-070's accepted and installed source-generation compatibility floor is an additional external dispatch prerequisite. T589 -> {T590,T591,T594}, T591 -> T592 -> T593 route -> T605, T595 composes after T590/T592/T594/T605, T596-T599 plus T604 fan out, T220 converges through iterative nonbinding plan-phase review and freezes F, and T600-T601 share that immutable candidate before T219's external-disposition-only conditional close. T219 performs no binding action |
| W6 | 29 | 27 Provider-dossier groups in five families plus process-provider integration and core-controller coordination; T039 is an explicit freeze prerequisite |
| W7 | 5 | Five file-disjoint closing groups; all five start together |

### The 14 file-overlap ordering constraints

These are the only edges that constrain shared files. Honor them as strict ordering:

- W5: `ADR046-device-006` -> `ADR046-nix-014` -> `ADR046-cli-011` -> `ADR046-nix-019` -> `ADR046-nix-031`
- W6: `ADR046-gpu-007` -> `ADR046-transport-unix-009` -> `ADR046-qemu-media-017` -> `ADR046-usbip-008`
- `ADR046-core-001` precedes `ADR046-device-007`, `-exec-013`, `-exec-015`, `-network-008`, `-telem-011`, `-zone-control-016`, `-zone-control-021`

### Unordered contended files - integrator only

The integrator-only rule applies when no explicit serial edge assigns every writer. A
contended file with named, non-overlapping-in-time owners is permitted: the plan must identify
all writers, state their order, and block the later branch until the earlier owner merges.
`transaction.rs` (`T591 -> T592`) is the representative slice-to-slice ownership transfer;
prep-to-slice ownership transfers are named in T589's file map and dependency chain.
`packages/Cargo.lock` is not transferred: T592 is its sole owner and T593 is a read-only
consumer of the frozen dependency graph. None is parallel ownership.

`packages/d2b-contracts/src/v3/volume.rs`; the `packages/Cargo.toml` member list; the
`flake.nix` output list; `nixos-modules/index.nix` and `default.nix`;
`packages/d2b-contract-tests/tests/workspace_policy.rs`; and the spec-set and work-item
manifests (last commit of each wave).

`CHANGELOG.md` is different: **no slice edits it**. Each writes one `changelog.d/<branch>.md`
fragment; the integrator folds them at wave close.

### Heavy lanes

Every Layer-2, hardware, live, and perf command runs through the two-slot semaphore. Never
invoke an internal `heavy-lane-*` target directly.

---

## Implementation strategy

### Critical path

W3 is a single serial spec that gates all 27 dossiers, so it is the narrowest point in the
program. W5 carries the highest risk: the corrected store engine exists, but the authenticated
publication, policy, controller-effect, audit, and restart owners converge only at T595.
T603 requires pre-validator A/P0 gates, validator-and-fragment V/B, and rerun B/P gates before its
external reconciliation and editor receipts; T589-T602 plus T604 ensure the production risk
is retired by operator and production-boundary evidence rather than a readiness substitute.
T605 supplies the pre-consumer contract, T595/T599 reconcile emitter and consumers, and T220
owns generated-manifest drift plus iterative nonbinding plan-phase convergence and
immutable-candidate freeze before T219's external-disposition-only gate. The retained Wave 5
request is already consumed; T219 issues no request.
C1 is fully assigned but not yet implemented. W6 carries the highest volume (258 items).

### Incremental value

Each user story is independently demonstrable:

- **After `adr046w5`** - a partial US1 production-plane checkpoint exists only if T602 and
  T219 pass. T604 proves an operator can declare a Zone through Nix activation and watch the
  Wave 5 acceptance set become real through an
  authoritative ComponentSession, ZoneBus, production store, controller, audit, and restart
  path, then remove one declaration and observe safe cleanup. A ready-looking skeleton or
  direct service fixture has no incremental value and does not satisfy this checkpoint.
  Network remains Wave 4 implementation; this acceptance result does not reassign it. Guest
  remains unaccepted, so this checkpoint does not complete US1.
- **After W6** - full US1 completes only after T479/T480 accept exact-F6
  `Provider/runtime-cloud-hypervisor` production-boundary evidence for the declared Guest's
  real Cloud Hypervisor process effect, authenticated guest-control session, and ready state;
  missing, skipped, status-only, fake-boundary, other-family, or refusal evidence leaves US1 incomplete.
  US2 is complete. Capabilities arrive declaratively through Providers.
- **After W7** - US3 is complete. An existing host can move onto 3.0.
- **After W8 plus Release** - US4 is complete.

Note that no intermediate release ships (FR-045). These are internal checkpoints, not
deliverables.

### Task count

605 tasks: 18 pre-wave/process hygiene tasks (4 panel-model migration, 4 pipelined-wave
migration), 531 initial-scope work items, 18 wave entry/gate/merge tasks for W2-W7, 5 for the terminal wave,
4 added at W5/W7 by the earlier analysis remediation, 16 added by the approved W5
production-completion amendment, 1 T603 amended-plan resume reconciliation task, and 12 for
the release.
The 531 primary work-item tasks preserve the exact items that were `Planned` at program
opening - one primary task each, no more and no fewer. At committed HEAD
`868469bf9c293cd48fff483717f14cb88c246821`, 54 of those items are now manifest `Merged` and
checked, while 477 remain manifest `Planned` and unchecked. Together with the 14 W0/W1 items
that were already `Merged` before primary-task generation, the current authoritative census
is 68 `Merged` and 477 `Planned`. T575 and T583 are process tasks that cite manifest ids; they
are not additional primary work-item tasks and their checkbox state does not override the
manifest. Task ids added after the initial generation continue from T574 rather than
renumbering the original 573.
