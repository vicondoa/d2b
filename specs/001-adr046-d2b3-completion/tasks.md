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

Wave panel requests, PR merges, post-merge seals, and eligibility checks are strictly
sequential. Successor implementation may
start after at least five predecessor selected-roster reviews return and predecessor
integration is green, but the successor cannot request a panel, seal, or merge until the
predecessor merges and seals and the successor rebases. Current selection uses the
thirteen-seat role domain and may only widen over fix deltas; fixed-ten records remain
readable as legacy data only. Within a wave, parallel groups are file-disjoint by
construction and MUST be launched in the same coordination cycle - a ready slice left
unlaunched without a recorded blocker is a process failure, not a scheduling preference.
The sole predecessor exception is ADR-046 Wave 6: this feature's exact validator/tooling
contract instantiates the generic Constitution 3.1.0 disposition with the merged Wave 5
boundary and retained no-seal state. It does not weaken any Wave 6 gate.

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
`/d2b-panel-round plan` must use one lifecycle selection artifact and return one record for every selected seat against one exact implementation base
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
delivery-record confirmation or an accepted external correction. Wave 5 is also exceptional: its retained pre-amendment `panel-request.json` consumed the
once-per-wave binding request with zero attestations and no seal. The exact validator/tooling
contract applies the generic Constitution 3.1.0 disposition to that state through merged
commit `177235ed37188b3be87525e7f016fb43401574c5` as closed history only. T219 records the disposition. No nonbinding phase round, current panel,
replacement candidate, second request, attestation, seal, or recovery action may alter it.

**Generic Constitution 3.1.0 disposition plus exact ADR-046 contract (FR-036, accepted)**:
the constitution supplies no program-specific detail. This feature and the exact delivery
validator/tooling contract limit the instantiation to ADR-046 history through merged Wave 5.
For prospective continuation,
T221 must fetch exact `origin/v3`, use it as the Wave 6 base, match the accepted
first-parent amendment integration commit after the Wave 5 merge, match the exact retained
candidate root and evidence digests, and pass the focused delivery-guard tests. Only then may
the ordinary exact-base selected-roster Wave 6 plan panel run. This disposition adds no
implementation checkbox and weakens no prospective validation, panel, PR, seal, or merge
gate.

**W2-W6 host-continuity close gate (FR-075)**: prospective execution is limited to
T220/T600 for W5 and T479 for W6. Those tasks MUST run the existing heavy-gated
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
edit. T604 is the sole W6
source owner of `tests/host-integration/daemon-restart-vm-survival.nix` and that check's
discovery/build recipe in `Makefile`; its public target must fail on empty discovery, any
`SKIP`, a missing build, or a non-x86 result presented as passing. The positive case proves
`Ready` before daemon restart, reachability, fresh-pidfd adoption of the original runner with
the same PID and start identity, continued reachability, and `Stopped` after public stop.
Its negative cases inject numeric PID reuse, pidfd/start-identity mismatch, and multiple
plausible runners and require quarantine with no adoption, signal, or cleanup against an
unproven process.
T029, T036, and T071 MUST reopen that result only while verifying the external historical
close records; they do not rerun it or use it to authorize another panel, seal, or merge.
The retained W5 result is historical and matched only by T221's predecessor guard. T480
revalidates W6's prospective result before panel request, merge, post-merge seal, and merge
eligibility. Historical F2/F3/F4 records must each contain exactly one
candidate-bound `local-host`
`EvidenceRecord.validation = "pre-adr046-host-continuity"` result; W5 folds the result into
`production-session-watch`; W6 folds it into the
`w6-cloud-hypervisor-guest-acceptance` record. Missing, duplicate, empty, skipped,
wrong-candidate, stale, status-only, private-hook, missing Ready/Stopped, non-fresh-pidfd,
incomplete unit enumeration, or nonexecuted historical W2-W4 evidence requires external
correction and leaves adjudication unchecked; it does not schedule a replacement close.
Any retained Wave 5 inventory mismatch blocks T221 and W6's prospective close.
Passing evidence names the exact enumerated and built attr, records command success, and
contains no `SKIP` result. This adds no task ID and no W5 evidence identifier.

For prospective W6-W8, the entry tasks refuse the first implementation dispatch until the
plan receipt validates. For already-delivered W2-W4 and already-dispatched W5, no
contemporaneous plan-panel receipt is cited by these feature artifacts; historical plan-review
compliance is unproven. T008, T030, T037, and T072 may be checked only by exact retained
evidence from the applicable first-dispatch base. W2-W4 do not run remedial plan panels:
their remaining tasks require exact external delivery-record confirmation or an accepted
external correction and remain historical verification/adjudication only.

Wave 5's recorded pre-edit and post-edit selected-roster plan lifecycle instructions around
T603 are historical planning evidence. They leave T072 unchecked and cannot rewrite the
missed boundary or dispose the retained request. T219 requires no recovery artifact.

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

Before T480, T555, or T565 runs `make-records`, the integrator MUST create round-local
`observed.json` with one entry for every selected seat. Every entry has exactly `provider`,
`model`, `reasoning_effort`, `context_tier`, `communication`, `agent_type`,
`agent_definition_sha256`, `run_id`, and `receipt_locator`. The first six values use the
completion-bound dispatch policy except for its fixed provider, the definition digest uses
the completion-bound staged agent bytes, and the final two values come from that seat's
actual Task result envelope. Reviewer self-report is ineligible. This is same-user process
metadata validated against the packet for correlation and uniqueness, not authentication,
an authentication proof, or proof that a particular definition executed.

T480, T556, and T561 MUST require effective `v3` rules that atomically refuse a merge when
the expected base becomes stale by configuring a nonempty required-check set with strict
up-to-date enforcement. They verify the exact base OID before snapshot, after required
checks, and immediately before merge, while GitHub enforcement closes the last race. A merge
queue does not replace this requirement. It is sufficient only when a required
`merge_group` check compares the actual merge-group integration tree with the snapshot-bound
expected `integration_tree_oid` and refuses a mismatch. A head-only match and a post-merge
tree comparison are insufficient. On any base change, the operator updates the integration
branch and restarts validation, selected-roster verification, snapshot creation, binding,
and required checks in the existing Track A order. The old snapshot, records, attestation,
and CI evidence are ineligible. If the wave's sole request was already consumed, no
replacement binding may be established unless a later accepted contract expressly permits
it. The exact ADR-046 contract permits no Wave 5 replacement binding. These R12 and R55 requirements
add no task ID, change no checkbox, preserve Track A order, and remain mandatory for
prospective Wave 6.

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

- [X] T001 [US1] Resolve CHK013 - state Gate 0's standing re-evaluation obligation as a requirement, not only an assumption
- [X] T002 [US1] Resolve CHK027 - record the ordinary entry-evidence versus exit-evidence distinction. The exact ADR-046 validator/tooling contract applies the generic Constitution 3.1.0 disposition to historical W0-W5 deviations and leaves the ordinary prospective distinction binding from T221 onward
- [X] T003 [US1] Resolve CHK028 - bound the FR-034 historical record so it waives no work-item completion obligation, including the nine `ADR046-delivery-*` items that remain Planned
- [X] T004 [US1] Resolve CHK039 - state the contended-file prep discipline; W2 has a single `nixos-modules/assertions.nix` writer and this has immediate effect
- [X] T005 [US1] Record every Gate 3 checklist item as a deliberate deferral naming its owning wave, so a scheduled obligation is never mistaken for a coverage gap
- [X] T006 [US1] Answer CHK047 - confirm whether cloud accounts and access exist for the Azure-backed Provider validation required at W6 and by the release gate
- [X] T007 [US1] Prototype the RSS corrections (range-seek replay, streaming decode, shared immutable ChangeBatch fan-out) in `proofs/redb-resource-store-spike/` so W5 confirms rather than discovers (mitigates RK-1)
- [X] T574 [US1] **Author and record the W0/W1 delivered-without-seal history** (FR-034). It names the missing artifacts (the ten panel receipts and the seal for each wave) and the evidence actually available (all 14 assigned work items recorded as Merged through reviewed pull requests). Completion remains historical evidence only; Constitution 3.1.0 supplies the generic disposition and the exact ADR-046 contract owns its one-time bounds through merged Wave 5
- [X] T575 [US1] **Raise the recorded W2 destination drift to the integrator as a specification amendment** (FR-046). `ADR-046-validation-and-delivery` §3.2 lists `packages/d2b-process/` and `packages/d2b-provider-supervisor/` under W2, but the graph assigns their owning item `ADR046-process-001` to W4. Follow the graph; do not correct the prose inside a wave
- [X] T576 [US1] **Inventory which migration-map DELETE and REPLACE rows still lack a removal proof** and assign each missing proof to the wave that removes its path (FR-023). The current [`removal-proof-inventory.md`](./removal-proof-inventory.md) 48-row census records 5 proofed DELETE rows and 33 outstanding DELETE/REPLACE rows overall

### Prior panel model migration (COMPLETED)

T581-T584 record the earlier migration to `gemini-3.1-pro-preview`. That
binding is now the exact legacy compatibility pair; current gate instructions
below use `gpt-5.6-sol` at `xhigh`.

- [X] T581 [US1] Amend `ADR-046-validation-and-delivery` §12.3 to bind the panel to `gemini-3.1-pro-preview`, updating the pinned provider/model/reasoning-effort triple and the legacy 14-field record example. Current `PanelRecord` is the 15-field form with required `panel_format_version`; the 14-field form is historical-reader compatibility only. This is a member-spec amendment: it re-opens that spec's validation and panel evidence and re-triggers Gate 0 (FR-046)
- [X] T582 [US1] Update the pinned constants in `packages/xtask/src/delivery/model.rs` (`PANEL_PROVIDER_POLICY`, `PANEL_MODEL_POLICY`, `PANEL_REASONING_EFFORT_POLICY`) and the unit test at the bottom of that file that asserts their exact values
- [X] T583 [US1] Update the `ADR046-delivery-005` work item text, which explicitly says "adapt to bind the fixed `gpt-5.6-sol` model at reasoning effort `xhigh`", then regenerate the spec-set and work-item manifests and confirm `make test-drift` is clean
- [X] T584 [US1] Add the ten read-only Copilot panel agents and bind them through `.github/skills/d2b-panel-round/SKILL.md`, then correct the AGENTS.md panel-tooling wording so panel lanes do not silently fall back to a model whose records `panel-attest` will reject. The panel table explicitly binds `github-copilot` / `gemini-3.1-pro-preview` / `high` / `default`; the retired integration is not a supported path

### Pipelined-wave migration

T585-T588 record the original fixed-ten form of the still-operative pipeline. Their checked
task text remains historical evidence; current dispatch instead counts at least five reviews
from the candidate-bound selected roster in the thirteen-seat role domain and permits only
widening over fix deltas. Panel request, seal, and merge remain ordered after predecessor
seal and merge plus successor rebase.

- [X] T585 [US1] HISTORICAL - Amend `ADR-046-validation-and-delivery` §4 to permit pipelined implementation start under the four conditions (5 of 10 reviews returned, integration green, no successor panel/seal/merge before predecessor seal and merge, mandatory post-merge rebase before the successor panel). Preserve the strict panel/seal/merge ordering verbatim. Member-spec amendment: re-opens that spec's evidence and re-triggers Gate 0 (FR-046)
- [X] T586 [US1] HISTORICAL - Relax the `wave snapshot` entry check so an unsealed predecessor blocks the successor's **exit boundary** rather than its implementation start; the predecessor-merged assertion moves to the exit boundary: `panel-request`, `seal`, and `merge-eligibility`. Add tests covering: start permitted at 5 of 10, panel request refused while the predecessor is unsealed, and seal refused when the successor has not rebased since the predecessor merge
- [X] T587 [US1] Record the accepted rework cost (FR-050) in the delivery contract so a future integrator cannot cite pipeline rework as grounds to shorten a panel
- [X] T588 [US1] Configure or document review scoping for the `v3` lineage. `detect-changed-files.sh` resolves the default branch to `main` via `origin/HEAD`, but ADR-046 integrates on `v3`, which never merges to `main`. Every wave review MUST pass an explicit diff scope (wave integration branch against its real base) or it will treat the whole v3 divergence as the wave changes

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
the sole opt-in and requires no Zone/Host gate. Existing code also lacks the production
adapter from `NetworkEffectPort` to the broker/net-VM path. Sole Network opt-in is therefore
a recorded nonconformance, not an alternative close path. T070 and T071 remain blocked until
an accepted external correction preserves F4's historical bytes, marks its sole-opt-in result
non-authorizing, and lands a versioned double-opt-in migration that removes every
current-facing sole Network-opt-in path before T220 freezes F. The generated work-item
amendment must retain T336-T355 as authoritative W6 implementation under T221 and assign all
four Network/Host cases to that W6 group. A feature-local matrix, single-opt-in assertion,
declaration-only fixture, fake adapter, or evidence from the old env surface cannot resolve
the conflict. T220 remains historical planned evidence. T221 requires the accepted migration
and regenerated double-opt-in contract on the exact fetched Wave 6 base, and T480 revalidates
the production cases at close. T604 remains W6 acceptance-only after merged T336-T355 and
reuses that landed implementation; no feature-local status correction can unblock any
boundary.

### Group `wi:core-config-hub:w4` (1 items)

- [x] T069 [US1] `ADR046-network-008` - `packages/d2b-core-controller/src/configuration.rs`: bundle application (create)

- [ ] T070 [US1] W4 HISTORICAL CONVERGENCE VERIFICATION - depends on every current W4 work-item row, beginning with T038 and ending with T069; T039 remains the manifest-authoritative W6 process-provider integration item and is not a W4 prerequisite. Verify exact external records for actual F4, preserve them byte-for-byte, and classify sole Network opt-in as a nonconforming, non-authorizing historical result. T070 cannot complete from a correction that merely ratifies sole opt-in. It requires an accepted versioned Network amendment/migration that removes the stale sole-opt-in contract path, defaults both inputs false, and regenerates the work-item manifest with T336-T355 retained as authoritative W6 implementation under T221 and all four Network/Host combinations assigned there. A missing migration, stale sole-opt-in surface, declaration-only substitute, fake adapter, feature-local-only correction, or reassignment of T336-T355 before T220 leaves T070 open. T070 owns no implementation, fix, evidence replacement, candidate freeze, phase-panel, rebase, or PR action.
- [ ] T071 [US1] W4 HISTORICAL DELIVERY CLOSE ADJUDICATION - depends on T070. Verify exact external records binding actual F4 to W4's sole binding `/d2b-panel-round work` request, attestations, seal, merge target, merge eligibility, merged PR, and resulting `v3` commit, but treat that close as non-authorizing because F4 retained sole Network opt-in. T071 cannot complete until the accepted external correction preserves those bytes, records the nonconformance, and binds the accepted double-opt-in migration plus the settled T336-T355 W6 implementation and four-case ownership. It must not dispatch reviewers, run a phase or binding panel, create replacement evidence, attest, seal, register a target, merge, rebase Wave 5, clean delivery state, or claim a new W4 close. A sole-opt-in disposition, stale current contract, local receipt predicate, pre-T220 reassignment of T336-T355, or inconsistent historical interpretation leaves T071 open.

**Checkpoint**: W4's reported seal and merge are externally confirmed or authoritatively corrected. This batch performed and claims no new W4 panel, seal, merge, rebase, or cleanup.

---

## Wave `adr046w5` (manifest label W5): Production store engine and watch, resource catalog, telemetry, CLI, Nix configuration

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 146 | **Parallel groups**: 12

**US1 scope boundary**: this wave is a partial production-plane checkpoint, not US1
completion. This wave pins the later W6 operator acceptance set as exactly
`Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm` in Zone
`acceptance`, with the exact Provider installs, configs, effects, readiness, and Device
cleanup frozen in `spec.md`.
Support resources cannot substitute. W4 history remains byte-preserved but its sole-opt-in
Network result is nonconforming. An accepted external Network contract/work-item amendment
must remove stale sole-opt-in contract paths before T220 and retain the production
implementation plus four-case matrix in authoritative W6 rows T336-T355 under T221. Wave 5
does not claim T604's positive operator activation result. T604 remains W6 acceptance-only
and consumes the implementation after T336-T355 merge. Guest runtime-effect acceptance
remains fail-closed until Wave 6
`Provider/runtime-cloud-hypervisor` completes T384 and T479/T480 accept its exact-F6 evidence.

- [ ] T072 [US1] `adr046w5` HISTORICAL ENTRY ATTESTATION - no exact contemporaneous Wave 5
  plan-panel receipt is cited, so this predicate remains unproven. The exact ADR-046
  validator/tooling contract applies the generic Constitution 3.1.0 disposition to it only
  as part of the closed history through merged Wave 5 commit
  `177235ed37188b3be87525e7f016fb43401574c5`. It does not check T072, authorize remediation,
  or permit a new Wave 5 request, attestation, seal, merge, or close. Preserve this checkbox
  unchecked as historical evidence.

**Approved production-completion amendment**: the 12 manifest groups remain authoritative for
their 146 work items. T589-T602 and T605 add the missing Wave 5 composition, coordinated
contract correction, and evidence, and T603 adds the exclusive-editor reconciliation. T604
is the W6 operator acceptance task under T221 after T336-T355 merge. None renumbers, replaces, or
completes a manifest item.

**Current state for this task graph:** the T603 and T589-T602 instructions are retained as
historical planning evidence and do not claim their unchecked work completed. Constitution
3.1.0 accepts the actual merged Wave 5 boundary without reconstructing these task results.
T219 is complete only as a historical disposition. T221 is the next executable gate.

Dependency order is:

```text
T603 editor reconciliation -> T589 -> {T590,T591,T594}
T591 -> T592 -> T593 -> T605
{T590,T592,T594,T605} -> T595
T595 -> {T596,T597,T598,T599}
{T596,T597,T598,T599} -> T220 -> freeze F
F -> {T600,T601} -> T602

historical merged Wave 5 boundary -> T219
T219 -> T221 prospective entry
```

The historical plan assigned twelve source-writing fragments to T589-T599 and T605. T603
owns no fragment; T600-T602 were evidence-only; T219 writes nothing. T604's fragment belongs
to prospective W6.

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

**Accepted telemetry correction required before dispatch:** T182-T205 remain blocked until a
versioned amendment to accepted `ADR-046-telemetry-audit-and-support` and regenerated
work-item manifests remove raw Zone, resource, operation, correlation, and trace identities
from audit and telemetry. Audit records use distinct typed domain-separated fixed digests.
Logs and spans use a typed digest only where correlation is required; metrics and OTEL
resource attributes carry no raw or digested identity. T205 owns the table-driven
redaction/cardinality/no-relabel guard for the complete producer set, and T220 rejects stale
generated rows. The existing audit and telemetry crates own derivation; no secrets service or
new runtime boundary is added.

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

- [X] T577 [US1] **Publish the desktop-companion inventory** as a versioned reference document naming each companion, its exact consumed surface, and its verification status (FR-039, contracts/companion-contracts.md CO-1). Published at W5, not at release, so companions have time to adapt. **Done**: `docs/reference/companion-contracts.md` revision 2 is current; revision 1 landed at `b72b205f`. All four inventory rows read "Pending live-host verification", and `weezterm` is excluded by a recorded negative surface-consumption determination, so publication claims no compatibility
- [X] T578 [US1] **Publish the replacement contracts the companions consume**, early enough for them to adapt given that no preview release may be published (contracts/companion-contracts.md CO-2, FR-045). **Done**: `docs/reference/zone-cli-contract.md` revision 1, landed at `b72b205f`. CO-5 remains the W5 exit condition: every "surface consumed" cell in the inventory must resolve to a committed contract at a public ref
- [X] T579 [US1] **Resolve the FR-039 / FR-045 tension before these contracts publish** (CHK025). FR-039 blocks release on external repositories while FR-045 forbids the preview build they would adapt against. This is the last moment the choice is cheap: resolve it here or amend FR-045. **Done, out of order**: T577 and T578 published first, so the resolution was encoded in shipped prose before any requirement said it. Closed by **FR-061** (contract/artifact boundary, publish-adapt-verify sequencing, per-stage refusals, amendment-only relaxation of FR-045) and **FR-062** (the adaptation assumption recorded as unvalidated with a mitigation, a detection point, and an escalation path). FR-045 is preserved, not amended. See `checklists/coverage.md`, "The W5 date-bound gate"

### Approved production resource-plane completion

Every unchecked task in this Wave 5 completion subsection is retained as historical planning
evidence only. Do not dispatch it as recovery, do not change its checkbox to reconstruct the
merged history, and do not make it a Wave 6 predecessor.

- [ ] T603 [US1] **`adr046w5` HISTORICAL FEATURE-EDITOR RECONCILIATION PLAN.** The planned
  all-or-nothing editor accounting sequence remains unproven and unchecked. Constitution
  3.1.0 does not claim its editor receipt, checkbox-only commit C, or post-edit lifecycle
  existed. It has no executable path after the merged Wave 5 boundary and does not gate
  T221.

- [ ] T589 [US1] **`adr046w5` INTEGRATOR PREP - freeze shared production contracts before parallel work.** Depends on T603, accepted external Version 2 delivery authority, the accepted telemetry authority correction, and the installed source-generation compatibility disposition. T589 refuses source changes until the generated `VD2-SC002-*` traceability bijection, regenerated ADR-046 manifests, Gate 0, source-floor import, and versioned telemetry work-item rows all validate on its exact base. It owns the shared Resource API/store/controller/bus and delivery-validator files already listed by the Wave 5 file-ownership map, plus only implementation, schema, fixture, API-snapshot, and changelog rows assigned to T589 by `VD2-SC002-TRACEABILITY`; it does not restate or redefine the external protocol. It freezes registrar admission, policy bootstrap, audit journal/status, required-Zone `(Zone, operation_id)` `InspectOperation`, UUIDv7 issuance/expiry, the exact-seven Wave 5 evidence profile, and generated SC-002 validation hooks before slice branches. It explicitly permits the same ID in different Zones and creates no host-global operation index. `operator-nix-activation-cleanup` remains T604-owned W6 evidence and is excluded from this profile. **Done when** all external prerequisites validate, every assigned generated row has one implementation and one enforcing test owner, the shared prep commit and API/schema gates pass, and T590, T591, T594, and the serialized T592/T593/T605 chain can branch from that exact commit.
  T589 consumes accepted Version 2 only through the generated `VD2-SC002-*` rows. It owns the shared registrar admission, policy-bootstrap, operation-status, authoritative-audit hook, exact-seven evidence validator, and the implementation/schema/fixture/API rows assigned to T589 by generated traceability. It verifies the installed source-floor import but does not produce, install, or redefine it. No feature-local encoding, registry count, state table, or recovery matrix is implementation authority.
- [ ] T590 [P] [US1] **Install and recover the single-owner Zone resource policy without a bootstrap cycle.** Depends on T589. Owned files: `packages/d2b-resource-api/src/authz.rs`, `packages/d2b-core-controller/src/rbac.rs`, and new focused tests under `packages/d2b-resource-api/tests/production_policy.rs`. `ZoneResourceRuntime` owns each `PolicyBootstrapRead` and requests installation, but `d2b-resource-api` alone parses and compiles policy into the immutable `PolicySet` interpreted by `NativeAuthorizer`. For initial install and restart, consume the one-shot capability to read only this Zone's policy-input envelopes at the exact durable nonzero `policy_revision`; it has no public subject, general read/mutation operation, clone, copy, default, public construction, conversion, trait-based mint, reconstruction, or reuse path. A failed installation attempt consumes the capability. After installation, perform every normal policy read/update through an authenticated Resource API session. Authorize T589's `InspectOperation` only for the registrar-derived subject and explicitly selected Zone. A wrong subject or replay-binding mismatch within that Zone, or an ID absent from that Zone, returns the same non-observing result as unknown and never exposes another Zone's operation; if the same ID independently exists in the selected Zone, that Zone's record is returned. On revision advance, compile the exact committed revision before atomic replacement, invalidate cached allows, and report ready only when installed revision and Zone UID equal live durable metadata. Refuse revision zero, stale/missing/cross-Zone/invalid policy, a caller claim, reusable bootstrap access, and any fallback to a constant or partial set. **Done when** focused tests cover first install, authenticated revision advance, restart recovery of the advanced revision, failed-attempt consumption, capability non-reuse, same-subject/Zone operation inspection, wrong-subject indistinguishability, and same-ID independent records in two Zones; external compile-fail fixtures prove construction, field access, `Default`, `Clone`/`Copy`, `From`/`TryFrom`, conversion, and capability reconstruction are impossible; T589's trait-solver, roots.json, golden, and API-surface seals remain current; `make test-rust` runs the Rust and compile-fail/doctest companions and `make test-rust-api-surface` passes; and every failure leaves only the affected Zone unpublished, degraded, and denied.
- [ ] T591 [P] [US1] **Restore the D106 store boundary and make it exhaustive.** Depends on T589. Owned files: `packages/d2b-resource-store-redb/src/transaction.rs`, `packages/d2b-resource-store/tests/d106_policy.rs`, and `packages/d2b-contract-tests/tests/policy_resource_mutation_seal.rs`. Preserve T589's frozen policy-neutral transactional audit hook. Remove redb deserialization or ownership of `RoleSpec`, `RoleBindingSpec`, `PolicySet`, and all other RBAC DTOs. Move policy-shape interpretation to the Resource API policy owner while retaining policy-neutral canonical-envelope, installed-schema, structural, atomicity, revision, and seal checks in the store. Expand the guard from three hand-picked source files to the full store/redb crate source and dependency graph. The scan MUST enumerate a nonempty source set independently for each store crate and a nonempty resolved dependency set; an empty, missing, or filtered-away input is a failure. Add a hermetic poison fixture that injects both a forbidden RBAC DTO use and a forbidden Resource API dependency and proves the existing test-policy/fixture-contract path rejects them. **Done when** the policy test proves neither store crate depends on the Resource API or contains/imports/deserializes an RBAC policy DTO, the poison negative fails for the intended D106 reasons through existing `make test-policy` and fixture-contract gates, the native evaluator remains the only allow issuer, and authorized Role/RoleBinding mutations still pass through the sealed generic envelope path.
- [ ] T592 [US1] **Complete durable store identity recovery, authoritative audit, and target-broker handoff adoption.** Depends on T591 and is the serialized writer of `packages/d2b-resource-store-redb/src/transaction.rs`. Owned source scopes are `packages/d2b-resource-store-redb/src/{lib.rs,actor.rs,audit.rs,migration.rs,backup.rs,tests.rs,transaction.rs}`, `packages/d2b-audit/src/{lib.rs,sink.rs,record_types.rs,segment.rs,export.rs}`, `packages/d2b-contracts/src/{lib.rs,broker_wire.rs}`, `packages/d2b-core/src/{privileges.rs,privileges_w3.rs}`, `packages/d2b-priv-broker/src/{bootstrap.rs,lib.rs,main.rs,protocol.rs,runtime.rs,live_handlers.rs,sys.rs,audit.rs,ops/mod.rs,ops/audit_op.rs}`, `packages/d2bd/src/{lib.rs,daemon_version.rs}`, `nixos-modules/{options-zones.nix,resources.nix,resources-bundle.nix,privileges-json.nix}`, `tests/unit/nix/cases/zone-audit.nix`, its generated schema/catalogue/API snapshots and focused policy/compatibility tests, both Cargo lockfiles, and the accepted resource-store/audit normative specs assigned by the generated manifest. T592 owns the physical store migration, immutable mutation journal and separate export state, fd-anchored broker export/prune path, typed durable `(Zone, operation_id)` `InspectOperation` backend, UUIDv7 issuance/expiry and per-Zone durable retention clock, protocol-5 target adoption, accepted-socket peer-pidfd broker op and the sole approved FFI quarantine, exact source-to-target coordinator transfer, target catalogues/snapshots, and generated outputs. It creates no host-global operation-ID index. It consumes the installed source generation read-only and never creates a source compatibility actor, unit, override, or target-only substitute.

  SC-002 and source-floor detail comes only from T592-assigned generated rows for `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY`. Those rows solely define schemas, encodings, receipts, capability transitions, fixtures, poison cases, counts, and transition matrices. T592 must refuse a missing, stale, wrong-owner, non-ancestor, or failing row before source changes; feature-local prose is not an expectation source.

  **Done when** store migration/rollback and crash recovery preserve advanced identities; every committed privileged mutation has one immutable transactional journal row and durable export state; required-Zone same-ID inspection is replay-bound; cross-subject/request variants are non-observing; concurrent same-ID operations in two Zones each apply exactly once; response loss and restart return the selected Zone's original result; malformed/future/expired/clock-discontinuous IDs and post-prune reuse refuse before mutation; raw identifier, trace, path, and peer-identity canaries remain absent; `zone-audit.nix` pins option placement, every default and bound, and missing/unknown/out-of-range failures; protocol/catalogue/schema/snapshot/privilege parity and both lockfiles move atomically; accepted-socket fd transfer is close-on-exec and leak-free on every error; target/apply/GC-root and live apply-peer substitutions refuse before mutation; source-to-target coordinator ownership transfers exactly once through the three existing units; every T592 generated Version 2 row has its assigned implementation and enforcing test; and `make test-rust`, `make test-policy`, enabled `make test-fixture-contracts`, `make test-drift`, and the owned Nix tests pass without skip.
- [ ] T593 [US1] **Publish the authenticated Resource API and watch route.** Depends on T592. Owned files: `packages/d2b-bus/src/{router.rs,registry.rs,authorization.rs,operations.rs,streams.rs,session_seam_tests.rs,transport/unix.rs}`, `packages/d2b-resource-api/src/{adapter.rs,watch.rs}`, `packages/d2b-contracts/src/v3/services.rs`, `packages/d2b-session-unix/src/{lib.rs,adapter.rs,descriptor.rs,pidfd.rs,socket.rs,error.rs,subject.rs,zone_admission.rs}`, `packages/d2b-session-unix/tests/{subject_mapping.rs,unix_session.rs}` plus new compile-fail fixtures in that directory, `packages/d2b-bus/tests/{production_resource_route.rs,public_mint_surface.rs}`, and accepted normative specification `docs/specs/ADR-046-componentsession-and-bus.md`. T593 may not create or edit a project-authored FFI crate, `packages/d2b-priv-broker/src/sys.rs`, a Cargo manifest, or a lockfile; T592 has already frozen the only broker wire, FFI, and dependency boundary. Replace the unregistered production seam with a route whose registration consumes the authenticated ComponentSession admission. At Unix accept, transfer the accepted socket to T592's typed `OpenPeerPidfdFromAcceptedSocket` broker operation with `SCM_RIGHTS` and consume its returned `OwnedFd` pidfd; `pidfd_open(SO_PEERCRED.pid)` is forbidden. T593 must use T592's two receive helpers unchanged: both set `MSG_CMSG_CLOEXEC`, reject truncated control data, require exactly one expected fd, and close all excess or error-path fds. No request or session type carries a raw descriptor integer, credential tuple, or numeric PID. The session adapter, descriptor, bus Unix transport, and session seam must all consume the same accepted-socket evidence object; none may reacquire credentials, accept a caller-supplied verifier, or construct evidence from a credential tuple or numeric PID. Treat `SO_PEERPIDFD` support as part of the kernel floor and fail closed with an actionable unsupported-kernel error when the broker returns that typed refusal. Require `FD_CLOEXEC`, verify the `SO_PEERCRED` tuple, expected process generation/start identity, expected cgroup, and liveness against that exact fd, and consume all evidence into one private registrar issuer. Reject a dead fd, credential/generation/cgroup mismatch, ambiguous evidence, or any numeric-PID-only path. Remove the public `ZoneBootstrapIdentity::verify` path, its `Clone` implementation and identity/evidence accessors, the `VerifiedUnixPeer::credentials` accessor, caller-supplied verifier and credential constructors, and every direct or transitive re-export that permits external issuance; neither type may expose construction fields, `Clone`, `Copy`, `Default`, conversions, raw credentials, pidfd, generation, or cgroup evidence. `ZoneRegistrar` exclusively derives and propagates the subject from its private mapping; requests and stream frames carry no subject claim. Register exact-Zone ResourceService and controller routes; add T589's required-Zone `InspectOperation` to the closed service/method catalogue, authorization map, and router without a selector-free or host-global route; admit watch replay/live delivery through ZoneBus; and expose one registration/readiness observation from actual owned handles. Bump accepted `ADR-046-componentsession-and-bus` from Version 1 to Version 2 and normatively pin accepted-socket transfer to the typed broker operation, private registrar issuance, consumed evidence, and the sealed public surface; T593 updates source-level mint, compile-fail, adapter, transport, and session-seam seals, T605 serialized after T593 regenerates the shared API snapshots, and T220 coordinates generated manifests, references, tests, and changelog treatment. **Done when** same-Zone authenticated Get/List/Watch/InspectOperation reaches the real service; cross-Zone, self-named, unregistered, reused-admission, direct-WatchService, missing/extra/truncated/malformed ancillary fd, post-receive decode failure, numeric-PID-only, post-credential PID reuse, dead-pidfd, credential/generation/cgroup mismatch, unsupported `SO_PEERPIDFD`, and ambiguity paths are denied with stable descriptor counts and no exec leak; existing adapter, descriptor, Unix transport, subject-mapping, Unix-session, and session-seam tests use accepted-socket evidence and reject all caller-supplied verifier/credential paths; external compile-fail/API-surface checks prove no public verifier, constructor, clone, credential/evidence accessor, conversion, re-export, or alternate issuer survives; source policy proves `d2b-session-unix` retains workspace `unsafe_code = "forbid"` with no local syscall/raw-fd fallback or project-authored FFI dependency; and neither `UnregisteredBusAdapter` nor a fixed endpoint can satisfy production publication.
- [ ] T594 [P] [US1] **Bind controller fan-in, effects, and cleanup to the durable replay/adoption ledger.** Depends on T589. Owned files: `packages/d2b-core-controller/src/{runtime.rs,resource_store.rs,provider_effects.rs,cleanup.rs,watches.rs,controllers.rs}`, `packages/d2b-controller-toolkit/src/{context.rs,runner.rs}`, and their existing focused unit tests. Register the production endpoint, consume admitted watch frames into the bounded fan-in, and record every post-commit effect intent before `EffectPort`. Bind each ledger entry to Zone, controller generation, resource UID, committed revision, operation id, and effect ordinal; reuse that key for idempotent dispatch/adoption. On restart relist and adopt/replay pending entries before cleanup. Complete cleanup only by compare-and-set on the same UID and exact nonzero expected revision. **Done when** unit crash-window tests prove no effect before commit or ledger durability, replay/adoption after every later crash point, no lost cleanup intent, and denial of stale, zero, wrong-UID, wrong-generation, or ambiguous completion.
- [ ] T605 [US1] **Correct and pin the system-core Zone handler contract.** Depends on T593. Sole writable ownership: `packages/d2b-contracts/src/v3/zone.rs`; compiler-regenerated public and private snapshots under `tests/golden/api-surface/`, regenerated only with `make api-surface-pin`; the existing lowest-layer guard `packages/d2b-contract-tests/tests/policy_contracts.rs`; governing normative specifications `docs/specs/providers/ADR-046-provider-system-core.md` and `docs/specs/ADR-046-resources-zone-control.md`; and paired reference page `docs/reference/resource-plane-runtime.md` (adapt). Add `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`; the one exact serialized spelling is `system-core-host` and `system-core-user`, matching the committed kebab-case rule. Bump both governing specification `Version` values and state explicitly that internal/telemetry `handler` labels remain `system_core_host` and `system_core_user` while those underscore values are forbidden in serialized `Zone.status.handlers[]`. The Zone status-handler contract MUST accept exactly one record with each serialized name, phase, and `lastReconciledAt`, reject duplicate or missing records, underscore/wrong-name substitution, and preserve `ZoneHandlerName::ProviderLifecycle` as a distinct allowed value that cannot substitute for either. Treat `packages/xtask/src/zone_schema.rs`, `docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json`, downstream T595/T599 consumers, and integrator-owned generated spec manifests as read-only inputs: because `ZoneSpec` is unchanged, generator execution MUST leave the committed desired-state schema byte-identical. **Done when** focused `d2b-contracts` tests prove both exact wire round-trips, underscore rejection, exactly-one-each list acceptance, duplicate/missing/wrong-name rejection, and `ProviderLifecycle` preservation/non-substitution; both normative specs and version metadata, the targeted guard, paired reference page, and public/private API snapshots pin the same pre-consumer distinction plus T593's removal of public peer/bootstrap issuance and evidence access after `make api-surface-pin`; the Zone desired schema is byte-identical before and after its existing generator; and the targeted contract plus `make test-rust-api-surface` pass. T605 does not wait for or attest to T595/T599 output and does not run the full `make test-drift`; T595 owns the emitter, T599 owns later consumer reconciliation, and T220 owns generated-manifest reconciliation plus the final drift gate.
- [ ] T595 [US1] **Compose the production Zone runtime and host-generation path.** Depends on T590, T592, T594, and T605. Sole owned files are `packages/d2bd/src/{resource_runtime.rs,lib.rs}`, `packages/d2bd/Cargo.toml`, `packages/d2b/src/{lib.rs,dispatch.rs,host_generation.rs}`, `packages/d2b/Cargo.toml`, `nixos-modules/{bundle-zones.nix,host-daemon.nix,host-broker.nix,options-site.nix}`, `flake.nix`, `examples/{minimal,graphics-workstation,multi-env,with-entra-id,with-observability}/{configuration.nix,flake.nix}`, `templates/default/{configuration.nix,flake.nix}`, `tests/unit/nix/cases/host-generation-rebuild-ref.nix`, `tests/host-integration/host-generation-handoff.nix`, the accepted Nix configuration spec, and focused tests in those owners. T595 writes `packages/d2b/src/dispatch.rs` before dependent T599 takes its later serialized ownership; they never write it concurrently. Compose T590-T594 into one daemon-owned per-Zone runtime: ingest each installed resource bundle automatically, install policy, register authenticated ResourceService/watch/controller routes, recover effects and audit, expose required-Zone durable operation inspection, and publish readiness only from live owned handles including T605's exact system-core handler names. Startup and shutdown visit every Zone and isolate failures.

  The deployment entrypoint remains unprivileged, validates one bounded flake/configuration target, builds and stages immutable bytes, obtains public-socket lifecycle authorization, and submits one opaque intent through `d2b host-generation` subcommands. The target closure and installed generation both expose only `bin/d2b`; no `d2b-host-generation-deploy` executable, alias, wrapper, package output, or completion entry exists. Privileged mutation is performed only by the installed source broker before transfer and T592's target broker after transfer, under the broker-owned durable coordinator and existing broker service. The caller cannot select an intent, privileged executable, command, path, or authority token. Target/apply/GC-root and live peer identity are revalidated before each mutation; all ambiguity, substitution, restart, and response-loss paths fail closed or resume the same intent. No new unit or daemon-owned rollback path is permitted.

  T595 consumes only its generated `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY` rows. They solely own source-floor fields, digests, fixtures, poison registries, transition ids, and counts. Missing or failing generated ownership blocks implementation; no feature-local copy is test authority.

  **Done when** startup and deployment switches automatically ingest add/change/remove bundles with no duplicate logical effect; readiness, required-Zone operation inspection, audit, restart adoption, and per-Zone failure isolation use the production route; packaging/help/completion tests prove `d2b` is the sole public binary and all host-generation operations are its subcommands; every flake, example, and template fixture owned above supplies an explicit valid `hostGenerationRebuildRef`, while the focused Nix case proves no default, exact 2048-byte acceptance, 2049-byte refusal, grammar bounds, and missing-reference evaluation; the parameterized migration and rollback VM test exercises the existing broker service and exact ownership transfer with no skip; source/target/apply/GC-root/peer substitutions and unauthorized caller classes mutate nothing; raw Nix stderr and private identities never escape; bundle/schema/reference/changelog outputs are reconciled by T220; every T595 generated Version 2 row has one implementation and enforcing test owner; and the owned Rust, Nix, fixture-contract, drift, and host-integration gates pass.
- [ ] T596 [P] [US1] **Add authenticated publication, watch, readiness, and Zone-isolation acceptance coverage.** Depends on T595. Sole owned file: new `packages/d2bd/tests/resource_plane_authenticated.rs`. Enter through the production daemon Unix session boundary, registrar, ZoneBus route, ResourceService, store, and controller endpoint. Consume T605's contract evidence and cover authoritative same-Zone Get/List/Watch, cross-Zone denial and audit, caller-supplied subject rejection, consumed-admission reuse, partial-readiness non-publication, exact `Provider/system-core` registration ownership, and an actual `Zone.status.handlers[]` list containing exactly one `system-core-host` and one `system-core-user` record with `phase` and `lastReconciledAt`, backed by active, initialized, current handlers. Prove ComponentSession admission is bound to the accepted peer's live pidfd and expected generation/cgroup evidence; after daemon restart require a newly opened pidfd for the rediscovered peer. Reject numeric-PID-only admission, stale evidence after numeric PID reuse, start-time/generation/cgroup mismatch, dead peer/`ESRCH`, and multiple plausible peers. Reject duplicate, missing, underscore/wrong-name required records and `provider-lifecycle` substitution. Run the three-Zone open/close matrix with failures in the first and middle positions; remove the Provider registration and each required list record in turn and prove only that Zone degrades. No Wave 6 dossier is required. The test must assert every Zone was visited and later healthy Zones remain operable. Direct service calls, `ProductionWatchHarness`, fake endpoints, status-only Provider substitutes, and readiness mutation helpers are forbidden in this file. **Done when** all cases pass against production owners, fresh-pidfd and every PID-reuse/mismatch/`ESRCH`/ambiguity negative pass, the emitted list shape matches T605, and removing or corrupting any required readiness owner makes the affected Zone return its specific actionable refusal.
- [ ] T597 [P] [US1] **Add restart effect-replay and cleanup-revision acceptance coverage.** Depends on T595. Sole owned files: new `packages/d2bd/tests/resource_plane_restart.rs` and new `packages/d2b-core-controller/tests/effect_replay.rs`. Crash after generation commit, after ledger durability, after effect dispatch, after adoption, and before completion; reopen through the broker-owned store path and assert each outstanding effect is replayed or adopted exactly once. Exercise pending cleanup across restart and reject zero, stale, wrong-UID, wrong-controller-generation, and ambiguous completion without changing durable state. **Done when** the matrix observes zero lost intents, zero duplicate logical effects, and adopt-before-cleanup ordering in every case.
- [ ] T598 [P] [US1] **Add authoritative audit, pending-result, replay-binding, retention, and redaction acceptance coverage.** Depends on T595. Sole owned file: new `packages/d2bd/tests/resource_plane_audit.rs`. Mutate through the authenticated production Resource API, including a multi-mutation batch; crash at every mutation/journal commit, segment append, file sync, directory sync, export-completion, rotation, journal-prune, segment-prune, operation-expiry, and operation-prune boundary; reopen; and compare immutable authoritative journal rows with exported logical records by fixed operation digest plus mutation ordinal. Include sink unavailable, disabled callback, incomplete export, hash-chain mismatch, duplicate replay, record oversize, invalid/default/boundary audit configuration, post-export journal retention, early-journal-prune refusal, and prune/sync-failure typed-health negatives. Prove the journal row commits transactionally with the privileged mutation before any effect is success-shaped; segment export and its completion cursor are separate and cannot rewrite or delete an unexported row; an exported row becomes deletion-eligible only after durable completion plus `audit.retentionDays`. After committed export-pending state, require `CommittedPendingAudit` through T589's `PendingAuditStatus` protobuf field, including `DeleteResponse` and batch ordinals, with the exact canonical `ResourceStatus` composite and no ordinary success or rollback claim. Inspect the same operation through T589's typed ResourceService method and T592 durable backend before and after restart only with a required Zone and exact replay-binding match to the original registrar-derived subject, canonical semantic request, target, verb, expected revision, and idempotency data in that Zone. Prove cross-subject and altered-request/target/verb/revision/idempotency/restart mismatches deny without observation or reapplication; concurrently submit the same opaque ID in two Zones and prove both independent operations commit once. Exercise commit-then-response-loss, restart, UUIDv7 malformed/future/expired/overflow cases, retention-clock rollback, prune at `expiresAt`, and reuse after prune; every old-ID mutation or inspection refuses before mutation. Retry a different ID and prove normal revision/conflict behavior. Inject distinct raw operation, correlation, subject, Zone, resource, and trace canaries; require only typed domain-separated fixed digests in journal rows, audit segments, and exports, no digest-class relabel, no identity in metrics or OTEL resource attributes, and no raw canary in errors, logs, metrics, spans, or redacted `Debug`. **Done when** every committed privileged mutation has an immutable authoritative row at commit, ordinary success waits for segment file and directory durability plus completion durability, multi-mutation restart yields exactly one export per ordinal, same-Zone same-ID apply count is one, cross-Zone same-ID apply count is one per Zone, all replay-binding and expiry cases deny as specified, the exact composite round-trips through every mutation response and required-Zone `InspectOperation`, all raw canaries remain absent, fixed-digest constructor and record-size limits hold, configured segment/journal retention and the fixed 30-day operation retention prune correctly without ID reuse, every prune/sync failure degrades health, status observability is stable across restart until expiry, and every audit/export failure leaves the affected Zone unpublished with an actionable typed refusal.
  The redaction matrix must enter through every migrated producer named by T592 and through
  T592's broker drain request. Its raw-canary error assertions cover backend and internal
  error contexts; the bounded direct operator-response exception is owned by T599 and is not
  an audit, telemetry, log, span, metric, or `Debug` exception. It covers valid-present,
  absent, and malformed trace context:
  present yields only the typed trace digest, absent stays absent, and malformed refuses
  before mutation, with no fabrication or cross-class relabel. Distinct root-path and opaque
  storage-handle canaries must also remain absent from fixed `Debug` output for every
  sensitive DTO, error, `SegmentWriter`, sink, exporter, directory owner, and broker owner.
- [ ] T599 [P] [US1] **Reconcile Wave 5 CLI and reference promises with emitted behavior (FR-019, FR-074).** Depends on T595. Sole owned files: `packages/d2b/src/{dispatch.rs,resource.rs,context.rs}`, `packages/d2b-contracts/src/cli_output.rs`, accepted normative specification `docs/specs/ADR-046-cli-and-operations.md`, `docs/reference/{zone-cli-contract.md,desktop-wrapper.md,companion-contracts.md,cli-contract.md,components-audio.md,components-usbip.md,components-usb-security-key.md,resource-client.md}`, `packages/d2b-contract-tests/tests/{policy_cli_consumers.rs,policy_docs.rs}`, focused CLI DTO/schema tests in the owning crates, and task-local `changelog.d/cli-operation-recovery.md` for T220 to fold. Its `dispatch.rs` ownership is the explicit later serial handoff from T595; T599 preserves and reconciles T595's frozen `d2b host-generation` namespace. Implement the recovery contract in `contracts/operator-cli.md` only through T589's typed store/ResourceService request and response, T590 authorization, T593 method catalogue/router, and T595 daemon/client path; an in-memory map or CLI-only synthesized result is forbidden. Every mutating generic and typed resource verb accepts `--operation-id <OPAQUE_ID>`; the ID is exactly 16 UUIDv7-layout bytes rendered as lowercase 32-hex without separators; an initial call emits it; an exact same-Zone retry reuses the original operation/idempotency binding; and `d2b --zone <ZONE> op inspect --operation-id <OPAQUE_ID> [--watch]` remains the accepted required-Zone status command rather than creating a competing command or host-global lookup. The same opaque ID is permitted independently in different Zones. Own the versioned operation-recovery DTO in `cli_output.rs` and its generated `JsonSchema` checks. Bump accepted `ADR-046-cli-and-operations` from Version 1 to Version 2 and coordinate a deliberate breaking amendment: assign pending exit 75 and replay-mismatch exit 76 for resource mutations/inspection, retain the existing meanings for unrelated exec commands, require `zoneRef` and `schemaVersion: 2` in every recovery success/error JSON envelope, add the exact closed remediation-action enum, and update the stable error-class and exit tables. Migration guidance must tell Version 1 consumers to require `schemaVersion`, upgrade parsing before using recovery, treat a missing or `1` version as the old 0/1/2 contract, and never reinterpret or silently migrate an arbitrary Version 1 operation ID; the v3 clean cutover has no persisted Version 1 recovery-state import. Human and JSON remediation may contain only a closed action such as `inspect-operation`, `retry-identical-operation`, `start-new-operation`, `wait-for-audit-export`, or `verify-operation-context`; it must never embed Zone or operation IDs in executable text, argv arrays, shell fragments, or free-form remediation. Raw Zone and operation ID appear only in their bounded `zoneRef` and `operationId` status fields. Pin mutation and inspection exits plus exact human/JSON pending/final/not-found/expired/refusal shapes, mandatory envelope fields, DTO schema, UUIDv7 issuance/expiry bounds, required Zone, cross-Zone same-ID independence, action enum, and absence of executable remediation vectors. Compare exact `d2b --help`, subcommand help, JSON output, capability keys, typed refusals, public wire fields, binary outputs, and completions. `d2b` remains the sole public binary: inspect, repair, restoration, authorization, and apply are all `d2b host-generation` subcommands, and no standalone executable, wrapper, alias, or migration fallback is emitted. Resource status documentation must expose committed-pending-audit through T589's additive protobuf status field and the exact `ResourceStatus.phase`, `outcome.code`, `update.state`, and `update.operation_id` composite; never claim success or rollback. Reconcile every downstream status consumer owned by this task with T605's paired contract and T595's emitted `Zone.status.handlers[]`: system-core readiness is attributed to `Provider/system-core` plus exactly one `system-core-host` and one `system-core-user`; underscore labels and `provider-lifecycle` cannot substitute. Candidate absence of a command or field is a defect, not permission to delete its promise, unless the same change follows the explicit parity or FR-042 retirement path with replacement, migration guidance, owner, release treatment, and contract tests. Do not add a fallback or claim companion verification. **Done when** every documented desktop-wrapper, companion, audio, USB, security-key, host-generation, and resource operation is present beneath emitted `d2b` behavior or has an approved parity/retirement record; operation inspection reaches the durable backend; pending-audit recovery matches the Version 2 amendment; exact tests cover Version 1 migration refusal, required `zoneRef`/`schemaVersion`, required Zone, UUIDv7 IDs and expiry, cross-Zone reuse, exits, all remediation actions, sole-binary packaging/help/completions, and no Zone/ID-bearing argv or executable remediation; T595's emitter and all T599-owned consumers match T605's exact names and non-substitution rule; and focused docs/DTO/schema/contract checks are clean. T220 reconciles the accepted-spec version into generated manifests, verifies paired references/tests/schema and release treatment, folds the fragment, and runs the full drift gate.
  Direct Version 2 operator CLI/JSON status and recovery responses are the sole raw-identity
  output exception: a bounded `zoneRef` and, where the exact envelope specifies it, bounded
  `operationId` may echo only the values supplied, generated, or received by that operator as
  recovery coordinates. T599's tests must prove those fields remain confined to that direct
  response and never become telemetry labels, spans, exported audit identities, or unrelated
  error context. An envelope such as `operation-not-found` that omits `operationId` must not
  add it as unrelated context.
  Preserve the accepted `op inspect` controls as
  `[--watch] [--deadline <DURATION> | --no-deadline]`: test each flag, their mutual-exclusion
  refusal, default-deadline behavior, and signal cancellation with no deadline. Human recovery
  narrows the preceding shared-remediation clause: JSON alone carries a closed action. Human
  mode instead
  renders the exact safe static `d2b op inspect` guidance from `contracts/operator-cli.md`
  without flags, identifiers, argv, or shell text; machine output retains only the closed
  remediation-action enum and never gains a free-form guidance field.
  T599 additionally owns the public versioned runbook
  `docs/how-to/host-generation-recovery-v1.md` and the generated closed mapping
  `docs/reference/host-generation-recovery-actions-v1.json`. Every public recovery action in
  `contracts/operator-cli.md` must resolve to either an exact CLI invocation or the
  identically named runbook anchor with a named operator role; bare procedure names are
  ineligible. T599's link and contract tests enumerate the emitted action set and fail on a
  missing, extra, duplicate, unowned, or broken mapping. The release gate requires both
  artifacts to be committed and referenced by the CLI contract.
  For resource mutations, T599 generates an omitted `--operation-id` client-side before
  transport creation. The commit-then-response-loss test requires exact human and JSON
  output containing that bounded ID and `zoneRef`, action `inspect-operation`, and recovery
  through `d2b op inspect` with zero second mutation; generating a replacement ID after an
  ambiguous response is forbidden.
- [ ] T220 [US1] **`adr046w5` CONVERGE + SELECTED-ROSTER PHASE LIFECYCLE + FREEZE.** Depends on T596-T599 and on the accepted external Network contract/work-item amendment. Merge every Wave 5 slice, reconcile owned generated manifests and the twelve source-writing changelog fragments, regenerate the nix-unit inventory pins for T592's audit case, and verify that accepted Version 2 plus `ADR-046-validation-and-delivery-traceability.{json,md}` and the amended telemetry authority were complete before T589. Require an exact generated bijection for all Wave 5 `VD2-SC002-*` identifiers and the complete T599 runbook/action mapping; copied SC-002 prose and declaration-only fixtures fail. Before freezing F, require the accepted versioned migration to define `effectiveEastWest = Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`, default both inputs false, remove every current-facing sole Network-opt-in path, and regenerate the work-item manifest with T336-T355 retained as authoritative W6 implementation under T221 and all four Network/Host cases assigned there. T220 refuses with remediation to amend the Network contract and work-item graph if any current contract, schema, fixture, checklist, or generated manifest still permits sole Network opt-in or moves T336-T355 before T221. T220 does not require or claim the W6 implementation or four-case results. T604 remains W6 acceptance-only and is excluded from the exact-seven Wave 5 profile. Run that exact-seven profile and all named enforcing gates. Create exactly one current phase lifecycle and one stable discovery ledger. For every provisional candidate and every fix, rerun deterministic selection against the current candidate; the selected roster may only widen. Dispatch the first selected roster through the lifecycle's sole comprehensive discovery, then dispatch every selected roster only through scoped verification with the stable ledger, responses, self-verification, fix delta, and full candidate. Never rerun comprehensive discovery and never create a successor lifecycle. Freeze final F only when every selected seat is unanimous with no recommendations. T220 creates no binding delivery request, attestation, or seal; the retained request remains immutable and receives no disposition or recovery action. Any content change or rebase invalidates F and reruns deterministic selection, scoped verification, and T600-T602 without replacing the lifecycle or discovery ledger. **Done when** generated manifests, telemetry rows, nix-unit pins, and traceability are clean, the accepted double-opt-in migration and settled W6 ownership are ancestors of F, twelve fragments are folded, the one selected-roster lifecycle and stable discovery ledger remain intact, every selected lifecycle seat approves exact F through scoped verification, T600-T602 can bind F and its tree, and no unresolved SC-002, stale sole-opt-in, or runbook prerequisite remains. This unchecked row remains historical planning evidence and is not reconstructed by the exact ADR-046 disposition.
  T220 consumes generated Version 2 rows rather than restating them. It proves the T599 runbook/action mapping is total, the selected-roster lifecycle approves exact F, and the exact twelve-fragment set is folded.
- [ ] T600 [US1] **Capture exact-candidate production-boundary evidence.** Depends on T220 and owns no repository files. Emit exactly `production-session-watch`, `effect-replay-cleanup`, `audit-drain-replay`, and `system-core-handler-contract`, once each, bound to F and its tree. Exercise the authenticated session/pidfd route, restart effect and cleanup replay, immutable audit and required-Zone `InspectOperation`, concurrent same-ID operations in two Zones, response loss, restart, UUIDv7 expiry/post-prune refusal, complete raw-identity redaction plus typed digest/no-relabel rules, exact system-core readiness, and no-skip FR-075 host integration. Import SC-002 evidence only through the T600 rows of the accepted generated `VD2-SC002-*` traceability table. **Done when** all four records pass their assigned enforcing gates and no copied feature protocol, fake boundary, skipped host check, host-global operation index, raw identity, or unrelated identifier is used.
- [ ] T601 [US1] **Capture exact-candidate RSS, owner fan-in, removal, and reference evidence.** Depends on T220 and runs read-only in parallel with T600 subject to the heavy-gate limit. Owns no repository files; import delivery evidence records only. T601 exclusively owns these three closed `EvidenceRecord.validation` identifiers: `resource-plane-rss-owner-fanin`, `wave5-removal-proofs`, and `cli-reference-conformance`. Measure the full daemon-owned publication path at 10,000 resources and 100 authenticated watches with no baseline subtraction; prove one store owner, one policy owner, one ResourceService route, one controller endpoint/fan-in, and one authoritative audit journal/export owner per Zone. The `Provider/system-core` registration and handler records belong only to T600's `system-core-handler-contract`. Re-run every manifest-label W5 removal proof at F instead of citing `removal-proof-w5.md`'s historical `a7f4a6a4` snapshot. Compare emitted CLI/help/JSON/wire behavior with all T599 pages, including the accepted Version 2 amendment and migration guidance, sole-public-`d2b` packaging and host-generation subcommands, exact UUIDv7 16-byte/lowercase-32-hex IDs, required Zone, cross-Zone same-ID independence, expiry, same-Zone retry and typed durable status command, exits, mandatory `zoneRef`/`schemaVersion: 2`, DTO/schema, human/JSON forms, closed remediation actions, Version 1 non-migration, and absence of any Zone/ID-bearing argv or executable remediation. Do not re-emit T600's handler-contract kind. **Done when** T601 emits exactly its three assigned identifiers once each for F; RSS is <=24,576 KiB, owner counts are exactly one, all current removal-proof predicates are true, Version 2 docs/DTO/schema/migration/release treatment and sole-binary packaging match emitted behavior, and every record names F and F's tree.
  `cli-reference-conformance` must exercise accepted `op inspect --deadline`,
  `op inspect --no-deadline`, their mutual-exclusion refusal, and cancellation; compare the
  exact identifier-free human `d2b op inspect` guidance separately from the unchanged closed
  JSON action enum. It must also prove client-side ID creation before transport and the
  commit-then-response-loss human/JSON output, then inspect that same ID with zero second
  mutation.
- [ ] T602 [US1] **`adr046w5` PRODUCTION-PLANE CHECKPOINT CONVERGENCE - historical planned evidence.** Depends on T600 and T601 and owns no repository files. Revalidate one T072 disposition, the editor receipt plus dedicated checkbox-only T603 commit, C as an ancestor of F, every T589-T599/T605/T220 completion, exact selected-roster approval of F, and the exact seven T600/T601 lane/identifier pairs. Invoke the same checked-in validators used by import, reopen, panel, seal, and eligibility. For SC-002, require every applicable generated Wave 5 `VD2-SC002-*` row and reject any missing, duplicate, stale, wrong-owner, wrong-candidate, or non-enforcing result; no feature-local field list or historical count is authority. **Done when** the exact seven records bind F and its tree, generated traceability and drift checks pass, and the worktree is clean at F. This unchecked row neither gates T219 nor authorizes a Wave 5 close.
- [x] T219 [US1] `adr046w5` HISTORICAL DISPOSITION COMPLETE - the exact delivery
  validator/tooling contract applies the generic Constitution 3.1.0 disposition only to
  ADR-046 history through merged Wave 5 commit
  `177235ed37188b3be87525e7f016fb43401574c5`. The retained state remains candidate
  `d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`, snapshot identity
  `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`, head
  `19b77dad63060bcadd41f1ef800978d2c53cc030`, retained `panel-request.json` SHA-256
  `15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7`, the exact retained
  evidence inventory and digests, zero attestations, and no seal. This checked state records
  historical disposition only; it does not claim a panel pass, seal, implementation result,
  or reconstructed close. No Wave 5 recovery, replacement candidate, second request,
  retroactive attestation, reconstructed seal, or new close action is authorized. T219 owns
  no repository file and creates no delivery artifact.

**Checkpoint `adr046w5`**: the retained Wave 5 state is closed as immutable history through
the merged boundary. It has no seal and receives no recovery or reconstructed close.
W6 starts from freshly fetched `origin/v3`; this checkpoint claims no mutation of the
retained candidate or delivery history.

---

## Wave W6: Remaining Provider dossiers in five file-disjoint families

**Manifest inventory**: 27 Provider dossiers and 258 work items. T336-T355 remain
authoritative W6 work and may start only after T221's W6 plan lifecycle passes. The launch
set remains all 27 dossiers and all 258 work items, in 29 groups including
process-provider integration and core-controller coordination. T604 is the W6 operator
acceptance task that consumes the merged T336-T355 result; it does not move those rows.

- [ ] T221 [US2] W6 HISTORICAL-PREDECESSOR GUARD + PLAN PANEL + ENTRY - before any Wave 6
  implementation lane is dispatched, fetch `origin/v3` and require the exact resolved
  `refs/remotes/origin/v3` commit as the clean entry base. Create the Wave 6 entry snapshot
  through the production delivery command so the historical-predecessor guard must pass.
  The guard must prove that merged Wave 5 commit
  `177235ed37188b3be87525e7f016fb43401574c5` and the unique integration commit on the base's
  first-parent lineage after it whose tree carries the exact accepted generic Constitution
  3.1.0 bytes are ancestors of the Wave 6 base and head. It must match the retained candidate,
  embedded snapshot identity, snapshot-file digest, request digest, evidence-tree digest
  `7deb84943d36962493422407ac74342fd598b2fea4970ea1a162942e25cfd33d`, exact candidate-root
  entry set, sole `evidence/local-host` directory, every evidence filename and SHA-256 listed
  in `data-model.md`, zero attestations, and no seal. Missing, extra, changed, partial,
  non-first-parent, non-ancestor, unfetched, or substituted state refuses.

  Run the focused production-guard validation with no ignored or skipped result:

  ```bash
  cargo test --manifest-path packages/Cargo.toml -p xtask \
    delivery::work_item_state::tests
  ```

  Then confirm Gate 0 passed, destinations are uncontended, the stack is proposed against
  that exact base, the heavy-gate semaphore is available, and the fast hermetic suite is
  green. Run `/d2b-panel-round plan` against the exact base, entry snapshot, and current
  feature snapshot. Require one record for every selected seat, N/N sign-off, and zero
  recommendations. The selection uses the current thirteen-seat role domain and may only
  widen over fix deltas. Any base, constitution, retained-state, evidence, feature, or guard
  change invalidates the entry snapshot and plan result. This authorizes implementation only.
  The same production guard is rechecked at panel request, seal, and merge eligibility.
  T480's distinct validation, exact-candidate panel, protected PR, post-merge seal, and
  merge-eligibility gates remain mandatory. No Wave 5 seal is required or created.

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

**Current generated ownership remains W6:** T336-T355 are the authoritative production
implementation rows under T221. T221 requires the accepted versioned
`ADR-046-resources-network` contract/work-item amendment on the exact fetched base. It must
require
`Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`, default both false,
remove every current-facing sole Network-opt-in path, and regenerate these rows with the
production adapter, site-gate transport, schema migration, and all four real
emitter/controller/broker/net-VM cases still assigned to W6. The amendment must not move or
replace T336-T355 with pre-T220 implementation owners. T604 remains W6 acceptance-only, owns
no Network implementation, and consumes the merged T336-T355 result. T479 revalidates the
implementation and evidence, and T480 rechecks them at close.

- [ ] T336 [P] [US2] `ADR046-nl-001` - preserve the landed `d2b-provider-network-local::controller::NetworkEffectPort`; amend the stale generated destination; implement it in `packages/d2bd/src/network_effect_adapter.rs`; wire it after T595 through `packages/d2bd/{Cargo.toml,src/lib.rs,src/resource_runtime.rs}` to the typed broker client, with no direct host mutation (adapt/create)
- [ ] T337 [US2] `ADR046-nl-002` - Broker wire contract and broker/core adapter operation table for `DeletePersistentTap` (adapt)
- [ ] T338 [P] [US2] `ADR046-nl-003` - `d2b-contracts` opaque byte-array newtypes (create)
- [ ] T339 [US2] `ADR046-nl-004` - Core LaunchTicket builder and dependency resolver that walks `Guest.ownerRef: Network/<name>` to resolved tap FDs. (create)
- [ ] T340 [P] [US2] `ADR046-nl-005` - `d2bd` Network adapter maps opaque Provider intents only to typed broker operations; no `d2b-host` mutation API is imported or called (adapt)
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

- [ ] T604 [US1] **Prove exact-F6 operator activation through real effect and cleanup.** Depends on T221, merged Wave 5 T595, the accepted pre-T220 double-opt-in contract migration, and merged W6 T336-T355 production implementation plus four-case matrix. It remains W6 acceptance work and owns no Network implementation row. Own only `packages/d2b-contract-tests/tests/resource_operator_activation.rs`, `packages/d2bd/tests/resource_operator_activation.rs`, `tests/host-integration/resource-operator-activation.nix`, `tests/host-integration/daemon-restart-vm-survival.nix`, the two host-generation case-id fixtures, their Makefile discovery/build recipe, and `changelog.d/operator-resource-activation.md`. Start from the independently installed source-generation compatibility floor and enter through Nix emission, automatic daemon ingestion, the production controller, and real broker effects. Prove the exact Volume, Network, and TPM Device resources in `spec.md`, including real Network bridges/firewall/net-VM/DHCP readiness through the landed double-opt-in implementation, idempotent redeploy, Device-only removal, TPM state preservation, and unchanged unrelated resources. Emit and validate SC-002 evidence only through the T604 rows of `VD2-SC002-RECEIPT`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY`; no receipt field, digest, size, or registry count in this task is normative. Run fixture-contract, Rust production-boundary, and no-skip `resource-operator-activation` host-integration legs on the same proposed F6. Guest runtime-effect and FR-075 continuity acceptance remain T384/T479/T480. **Done when** every named production effect and cleanup predicate passes on proposed F6, every T604 generated traceability row has exact evidence, the operator-activation VM check is enumerated and built without skip, and no fake adapter, status-only result, sole Network opt-in, implementation ownership, or Guest-success claim is used.
  T604 consumes only the T604-owned rows in the generated Version 2 traceability table. Its Network positive is blocked until authoritative W6 T336-T355 have merged and their double-opt-in production implementation plus all four cases pass; its SC-002 positive is blocked until `VD2-SC002-RECEIPT`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY` resolve to exact enforcing evidence. T479 imports `operator-nix-activation-cleanup` for exact F6. No feature-local protocol copy or historical registry count is an acceptance source.

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

- [ ] T479 [US2] W6 CONVERGE + FREEZE + OPERATOR/GUEST ACCEPTANCE - depends on T039, T604, T221, and every W6 work item in the authoritative manifest regenerated by the accepted Network amendment. Derive that exact set from the manifest, reject missing/extra/duplicate/unchecked/unreachable rows, and require every member to reach T479 in the dependency graph; no stale numeric interval or pre-amendment census is authority. Require every Wave 6 head to descend from T221's fetched exact `origin/v3` base and the accepted integration commit carrying the generic Constitution 3.1.0 disposition. Reinvoke the production historical-predecessor guard before freezing F6; it must still match the immutable retained Wave 5 state and evidence inventory. Confirm the accepted double-opt-in migration remains an ancestor, merge every W6 slice including T336-T355 plus T604, require the four Network/Host production cases, run integration/CI, fold W6 fragments, and freeze clean proposed F6. On that exact candidate, run no-skip `make test-host-integration` for `resource-operator-activation`, `runtime-cloud-hypervisor-guest-acceptance`, and `daemon-restart-vm-survival`. Require real Volume/Network/Device effects and cleanup, real Cloud Hypervisor/KVM and authenticated guest-control readiness, and FR-075 continuity. Import exactly one F6-bound `operator-nix-activation-cleanup` record from T604 and one `w6-cloud-hypervisor-guest-acceptance` record from T384; fake, status-only, refusal, skipped, empty, stale, wrong-family, or wrong-candidate evidence is ineligible. T479 issues no binding request, attestation, or seal. Any content, merge, generated-output, fold, rebase, base, retained-state, or evidence-identity change invalidates F6 and reruns both acceptance lanes.
- [ ] T480 [US2] W6 SINGLE BINDING WORK GATE + MERGE - depends on T479 including its exact-F6 `operator-nix-activation-cleanup` and `w6-cloud-hypervisor-guest-acceptance` records. Require HEAD and tree to equal clean F6; revalidate T221's exact-base unanimous plan-panel receipt, reviewed feature snapshot, accepted first-parent generic Constitution 3.1.0 integration ancestry, and exact retained Wave 5 inventory; and require the reviewed entry base to be an ancestor of every W6 implementation head. Reinvoke the production historical-predecessor guard and both closed acceptance predicates before panel request, merge, post-merge seal, and merge eligibility; missing, extra, changed, wrong-family, fake-boundary, skipped, empty, stale, or wrong-candidate evidence refuses each boundary. T480's work panel is not a substitute. Against F6, first dispatch the read-only reviewer Task lane and rubber-duck Task lane in parallel, each bound to `gpt-5.6-luna` / `max` / `long_context`; a content defect from either lane abandons F6 and returns to T479 before any binding panel request. Route the defect through scoped fixes, convergence, validation, and both acceptance lanes, then iterate delta/full-context `/d2b-panel-round plan` phase reviews until the replacement provisional candidate has N/N sign-off for the selected lifecycle roster with zero recommendations. Only that final candidate may receive W6's exactly one binding `/d2b-panel-round work` request: create the final snapshot and candidate-bound selection, issue the sole panel request, run `make-records`, and panel-attest N/N for the selected roster. Then require the already-open PR to preserve that exact head, wait for required CI, import the final evidence, capture the green merge-target input, and merge through the protected PR flow. The merge MUST preserve the successful candidate's tree byte-for-byte. Only after the merge may T480 seal the wave, register the captured merge target, and pass merge eligibility. A nonunanimous binding result permanently fails the W6 close: retain its candidate, request, findings, and records, issue no second binding request for any candidate, and stop with an integrator scope escalation; findings are not waived. From binding panel request through disposition, the final candidate and its tree are immutable. After the post-merge close passes, rebase the next wave onto updated `v3`, then clean up in order: delete each worktree `packages/target`, remove worktrees, delete local branches, delete remote branches, run `nix-collect-garbage`, and audit `git worktree list` plus `git branch -a` for residue. The one-time predecessor disposition never substitutes for these Wave 6 gates.

**Checkpoint**: W6 converged, panelled, merged to `v3`, sealed, rebased, and cleaned up.
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

- [ ] T481 [US3] W7 PLAN PANEL + ENTRY - before any W7 implementation lane is dispatched, confirm Gate 0 passed, destinations are uncontended, the stack is proposed against the exact named parent commit, the heavy-gate semaphore is available, and the fast hermetic suite is green on the entry tree; then run `/d2b-panel-round plan` against that exact clean base and feature snapshot and require N/N sign-off for the selected lifecycle roster with zero recommendations. If W6 is not yet merged, implementation entry additionally requires at least five of its selected-roster work reviews returned and green integration on its converged tree. The selection uses the current thirteen-seat role domain and may only widen over fix deltas. A base or feature change before dispatch invalidates the plan receipt. This authorizes implementation only: T555's distinct work panel, seal, and merge eligibility remain blocked until W6 is sealed and merged and W7 is rebased onto the updated integration lineage (FR-057).

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
  merge-target identity, and evidence refresh at panel-attest, merge, post-merge seal,
  merge-target, and merge-eligibility.
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

- [ ] T555 [US3] W7 SINGLE BINDING WORK GATE - depends on T580. Before dispatching either native pre-panel lane, require HEAD and tree exactly the current W7 provisional candidate, a clean index/worktree, T580 checked, exactly one passing `recovery-point-attestation`, T481's unanimous plan-panel receipt with its reviewed feature snapshot, and ancestry from the reviewed W7 entry base to every W7 implementation head. T555's work panel is not a substitute. Invoke T548's same hermetic validator - never a stage-local predicate copy - before phase review, panel request, panel-attest, merge, seal, merge-target registration, and merge eligibility. It must validate every FR-043 field and delivery binding, the candidate/commit/tree/preview/live-host/operator/restore-instruction digests, all qualification fields, locator resolution, bounded integer timestamp decoding, checked 86,400-second expiration arithmetic, and `previewed <= captured <= verified <= attested <= verifier-now < expires`. Missing, extra, failed, malformed, duplicate, wrong-type, negative, fractional, future-event, out-of-range, overflow, stale, expired, wrong-host, wrong-operator, wrong-restore-instructions, wrong-preview, wrong-candidate, wrong-commit, wrong-tree, unresolvable, post-freeze, empty validator discovery, ignored, or skipped state refuses the stage. Run the native Copilot pre-panel procedure against that candidate: dispatch a read-only reviewer Task lane and rubber-duck Task lane in parallel, each bound to `gpt-5.6-luna` / `max` / `long_context`. A content/history defect or pre-request attestation expiry abandons the provisional candidate and returns to T580 for scoped correction, fresh evidence, validation, and a delta/full-context `/d2b-panel-round plan` phase review. Iterate that nonbinding phase surface to N/N sign-off for the selected lifecycle roster with zero recommendations before selecting the final candidate. Only then create the final snapshot and candidate-bound selection, issue W7's exactly one binding `/d2b-panel-round work` request, run `make-records`, and panel-attest N/N for the selected roster; the panel request is refused unless every prior-wave work item is Merged and W7 is rebased after the predecessor merge. T555 creates no seal, merge target, or eligibility result. Nonunanimity, attestation expiry, or any post-request content/history/binding mismatch durably fails the W7 close and retains its request, findings, and records. Issue no second binding request for any candidate and stop for integrator scope escalation; findings are not waived. From binding panel request through durable disposition, the final candidate and tree are immutable. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), every removal proof for a path retired in this wave passed (FR-023), and the lifecycle ledger plus friction log are current (FR-053). Constitution 3.0 supplies no round-count deferral: pre-existing late MINOR and NIT observations remain nonblocking history, while admitted BLOCKER and MAJOR findings remain blocking (FR-051, FR-052).
- [ ] T556 [US3] W7 MERGE + POST-MERGE CLOSE - depends on the successful T555 binding result. Refuse unless HEAD and tree still equal that exact approved W7 candidate, every slice head is already its ancestor, T580 remains checked, and T548's same validator still accepts the sole `recovery-point-attestation` at T556 entry and immediately before merge. Also require the T580-opened PR still targets `v3` at that candidate, the request and attestation remain valid, and required CI is green. T556 MUST NOT open, update, or retarget a PR; merge a slice; fold a changelog fragment; regenerate content; run a content-changing command; rerun integration/CI as a convergence step; refresh expired evidence in place; or issue another panel request. Import the final candidate's CI and local/host evidence, capture the exact green merge-target input, then merge only the already-open T580 PR and only if the resulting merge preserves the approved tree byte-for-byte. After that merge completes, invoke T548's validator at each remaining boundary, seal the wave, register the captured merge target, and pass merge eligibility. A content, commit/tree, history, merge-target, evidence-identity, or attestation-expiry failure after the binding request retains the request, panel artifacts, and any post-merge records already created, durably fails the W7 close, permits no successor request, and stops for integrator scope escalation. T550/T551 crash tests must prove terminal failure survives every publication crash point before T556 may complete. After the post-merge close passes, rebase the next wave onto updated `v3`, then clean up in order: delete each worktree `packages/target`, remove worktrees, delete local branches, delete remote branches, run `nix-collect-garbage`, and audit `git worktree list` plus `git branch -a` for residue.

**Checkpoint**: W7 converged, panelled, merged to `v3`, sealed, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W8: Friction closure (terminal wave)

**Story**: US4 | **Work items**: recorded after W7 merge, seal, and cleanup | **Parallel groups**: determined by T557 after W7 cleanup

W8 has **no spec members and no work items yet, by design**. Its contents are the delivery
friction accumulated across W0 through W7 - in the categories signoff, build, test, merge,
codegen, and disk - triaged only after W7 is merged, sealed, and cleaned up. Its destinations are `packages/xtask/`,
`tests/tools/`, `packages/d2b-contract-tests/tests/`, and `Makefile`.

It runs the same wave template unchanged, including exactly one binding selected-roster panel.

- [ ] T557 [US4] W8 TRIAGE - depends on T556 including its completed W7 merge, seal, and ordered cleanup. Only after the worktree, branch, target, and Nix-store residue audit passes, collect and classify actual friction from every prior wave into the six categories and record the resulting work items in the manifest
- [ ] T558 [US4] W8 PLAN PANEL + ENTRY - depends on T557. Revalidate that W7 is merged and sealed, T556's cleanup and residue audit are complete, and the W8 stack starts from the resulting updated `v3` HEAD. After T557 fixes the triaged work-item set, file ownership, and validation map, and before any W8 implementation lane is dispatched, confirm Gate 0 passed, every fixed destination carries no open contention flag, the stack is proposed against that exact named parent commit, the heavy-gate semaphore is available, and the fast hermetic suite is green on the entry tree; then run `/d2b-panel-round plan` against that exact clean W8 implementation base and feature snapshot and require N/N sign-off for the selected lifecycle roster with zero recommendations. W8 has no pipelined triage or entry exception. The selection uses the current thirteen-seat role domain and may only widen over fix deltas. A task-map, ownership, feature, or base change before dispatch invalidates the receipt and requires another plan review. This authorizes implementation only; T565's distinct work panel, PR merge, post-merge seal, and merge eligibility remain separate (FR-057).
- [ ] T559 [US4] W8 IMPLEMENT + CONVERGE - depends explicitly on T557 and the passing T558 plan gate. Execute the triaged items (count known only after T557), merge every W8 slice into the wave integration branch rooted at the updated post-W7 `v3` HEAD, run integration tests and CI on the converged tree, and resolve every content-changing result. No slice branch may remain to merge after T559, and T559 MUST NOT issue a binding work-panel request, panel-attest, or seal.

W8 and Phase R share one final candidate F8. All repository content needed for release,
including the changelog fold, version header, release summary, release-binary and flake
versions, explicit prebuilt-or-source-fallback state, manual-only publication workflow, and
retirement list, lands before T560 freezes F8. Candidate-bound validation follows the freeze but precedes the one
binding panel. A candidate-bound failure that needs content changes abandons F8 and returns
through the owning pre-freeze task, T566, and T560 before validation is rerun. Once the
binding panel request is issued, that candidate is immutable. A unanimous candidate leaves
only merge work; a nonunanimous binding result terminally fails W8, permits no second
request, and cannot enter T561.

The executable pre-freeze DAG is `T556 -> T557 -> T558 -> T559 -> T571 ->
{T562,T568,T570,T572} -> T566 -> T560`. T556 includes W7 merge, seal, cleanup, and residue
audit. T571 publishes the retirement list before the four
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
- [ ] T566 [US4] Condition 5 + RELEASE STATE - depends on T559, T562, T568, T570, T571, and T572. This is the sole serial writer of `.github/workflows/release-host-binaries.yml`, `nix/prebuilt.json`, `flake.nix`, `CHANGELOG.md`, the Cargo manifests for the six release binaries (`d2b`, `d2bd`, `d2b-wayland-proxy`, `d2b-unsafe-local-helper`, `d2b-host` for `d2b-activation-helper`, and `d2b-priv-broker`), and both affected lockfiles. Fold every W8 changelog fragment, make `CHANGELOG.md` carry the 3.0.0 version header and version-level summary, set every release-binary and flake package version to that exact version, strip every internal wave, phase, and finding marker, and run focused changelog/version validation. For the first 3.0.0 tag, choose the existing explicit source fallback: commit `nix/prebuilt.json` with `version: null`, `system: "x86_64-linux"`, and an empty `binaries` object, which existing `nix/prebuilt.nix` maps to source builds. A complete manifest whose version and hashes match F8 is also valid, but the 1.4.1 manifest is not. Remove the `v3` push publication trigger and post-tag manifest repair PR; publication is manual-only and every build/tag/release job depends on an identity/prepublication job that accepts the sealed F8 tree, the merged `v3` HEAD commit supplied at dispatch, and the version, then verifies that the merged HEAD tree equals the sealed tree. Commit that final release state before F8 is frozen. Then rerun T562, T568, T570, T571, and T572 read-only and require all five records to bind this exact final pre-freeze HEAD and tree. Run integration and CI after those release bytes are committed and require both results to bind the same exact HEAD/tree. Any content-producing result restarts T559 and the complete downstream pre-freeze sequence; a result against the earlier T559 tree is not evidence.
  At publication dispatch, that identity job accepts the current merged `v3` HEAD commit as
  the publication identity and the sealed F8 tree as the content identity, then requires the
  merged HEAD tree to equal the sealed tree. The sealed F8 feature-tip commit remains seal
  evidence only; the workflow MUST NOT require its commit OID to equal the merged HEAD commit
  OID, and it tags only the verified merged `v3` HEAD.
- [ ] T560 [US4] W8 FREEZE - depends on T559, T562, T566, T568, T570, T571, and T572. Require every W8 slice head to be an ancestor of the converged branch, all changelog fragments folded by T566, every pre-freeze release artifact committed, release-binary/flake/changelog versions equal 3.0.0, and `nix/prebuilt.json` be either a complete matching manifest or the explicit `version: null`/`system: "x86_64-linux"`/empty-binaries source fallback. Require the publication workflow to have no push trigger, no post-tag manifest repair path, and no build/tag/release job outside the prepublication identity dependency. All five release-condition reruns bind after T566, integration and CI are green from mandatory post-T566 runs against this exact final HEAD/tree, and the index/worktree is clean. Reject any result bound only to T559 or any earlier release-content tree. Open or update one PR against `v3`, then freeze that exact clean HEAD and tree as F8. T560 MUST NOT issue a binding panel request, panel-attest, seal, tag, publication, or release workflow dispatch. Any content change, slice merge, generated-output change, changelog fold, or rebase after F8 is frozen invalidates F8 and all F8-bound evidence and restarts T559 plus the complete post-T566 validation path before T560 may freeze a successor.

### Exact-candidate release conditions and close

- [ ] T563 [US4] Condition 2 - depends on T560. Every DELETE and REPLACE row's removal proof passes on exact F8, not merely when it was first established
- [ ] T564 [US4] Condition 3 - depends on T560. The complete validation matrix passes against exact F8, including the manual hardware, live-host, and cloud tiers at least once with recorded external evidence, plus the reset and cutover scenarios
- [ ] T567 [US4] Condition 6 - depends on T560. Every prior wave's cleanup is done; no dangling implementation worktrees or branches remain
- [ ] T569 [US4] Depends on T560. Verify each companion by exercising it against exact F8 on a live host - `d2b-toolkit`, `d2b-wlterm`, `d2b-wlcontrol`, `d2b-clip-picker`; `weezterm` consumes no d2b contract (FR-040, SC-024). The set exercised is the one T568 re-derives under FR-064, not this task's illustrative list. All seven FR-065 conditions must hold: live host and not a VM, container, or CI runner; the exact candidate snapshot named by commit; the companion at a pinned commit; every surface in the row exercised rather than sampled; every surface Conformant or Retired under FR-063; zero Blocked including zero unclassifiable; evidence in FR-063's shape. Source inspection, a version or tag match, a green docs check, a successful build, a green CI run in the companion's own repository, an exercise against a non-candidate build, an exercise off the live host, a partial exercise, and the fact that the contracts were published at W5 are each explicitly not evidence. A capability-conditional refusal is Conformant only if it names the false capability key or refusal state and at least one concrete operator action - a silently greyed control is Blocked. **If F8 moves for any reason, every verification recorded against the previous snapshot is void and must be re-run.** A failure here is the detection event FR-062 names, and its response is to hold the release, abandon F8 for a pre-panel correction, or amend FR-045, never to relax FR-040
- [ ] T565 [US4] Condition 4 + W8 SINGLE BINDING WORK GATE - depends on T560, T563, T564, T567, and T569. Require HEAD and tree to equal clean provisional F8, every release condition and evidence record to name F8, T558's unanimous entry plan-panel receipt to match the current feature snapshot, and the reviewed W8 entry base to be an ancestor of every W8 implementation head. T565's work panel is not a substitute. Against F8, first run the native read-only reviewer and rubber-duck pre-panel lanes; a content defect abandons provisional F8 and returns through the owning pre-freeze task, T566, and T560 before any binding panel request. Route findings through scoped fixes and the complete post-T566 integration/CI and release-condition sequence, then iterate delta/full-context `/d2b-panel-round plan` phase reviews to N/N sign-off for the selected lifecycle roster with zero recommendations before selecting final F8. Only then create the final snapshot and candidate-bound selection, issue W8's exactly one binding selected-roster `/d2b-panel-round work` request, run `make-records`, require unanimous sign-off with zero recommendations against F8, and panel-attest. T565 creates no seal, merge target, or eligibility result. A nonunanimous binding result permanently fails the W8 close: retain F8, its request, findings, and records, issue no second binding request for any candidate, and stop for integrator scope escalation; findings are not waived. From binding panel request through disposition of F8, F8 and its tree are immutable.
- [ ] T561 [US4] W8 MERGE + POST-MERGE CLOSE - depends on T565. Refuse unless HEAD and tree still equal exact F8, the request and panel attestation remain valid, and the already-open PR against `v3` still names that exact head with required CI green. T561 MUST NOT merge a slice, fold a changelog fragment, edit or regenerate content, rebase, rerun integration/CI as convergence, issue another panel request or panel round, create a tag, or dispatch publication. Import the final candidate's CI and local/host evidence, capture the exact green merge-target input, then merge the PR only if the merge preserves F8's tree byte-for-byte. The manual-only workflow makes this merge non-publishing. Only after the merge completes may T561 seal the wave, register the captured merge target, and pass merge eligibility. Any content or history change before merge, or any post-request identity failure, blocks W8 and requires integrator escalation rather than another binding panel. After the post-merge close passes, clean up external worktrees, branches, targets, and the Nix store, and audit for residue.

**Checkpoint**: the exact F8 tree that ships is merged to `v3`, sealed, and merge-eligible.

---

## Phase R: Publish d2b 3.0

**Story**: US4. Publication consumes the merged **W8** candidate, not W7 - gating earlier would
release a candidate a later wave still modifies.

- [ ] T573 [US4] IDENTITY + PREPUBLICATION - depends on T561. Resolve the current merged `v3` HEAD commit and confirm its tree is byte-identical to the sealed F8 tree; do not require that merged commit OID to equal the sealed feature-tip commit OID. Confirm the `v3.0.0` tag is absent; changelog, all six release-binary manifests, lockfiles, flake package versions, and artifact version inputs agree on 3.0.0; and the merged tree contains either a complete matching prebuilt manifest or the explicit source fallback selected by T566. Confirm the publication workflow has no push trigger or post-tag manifest repair and that every publication job depends on its merged-HEAD/sealed-tree identity check. Dispatch that workflow with the merged `v3` HEAD commit, sealed F8 tree, and version; require it to repeat the checks, build only from the merged HEAD checkout, verify every artifact name, embedded version, and hash, and only then tag that merged HEAD and create the GitHub release. Any mismatch publishes nothing and cannot be fixed by a post-tag manifest PR

**Checkpoint**: d2b 3.0 released.

---

## Pipelined implementation and sequential wave exit

Current delivery permits successor implementation to start after at least five predecessor
selected-roster reviews return and predecessor integration is green. The successor panel
request, merge, seal, and eligibility still wait for predecessor merge and seal plus successor
rebase. ADR-046 Wave 6 is the one historical predecessor exception: T221 uses the exact merged
Wave 5 boundary and retained no-seal state bound by the feature-owned validator/tooling
contract under generic Constitution 3.1.0. W8 is the other
explicit entry exception: T557 triage and T558 entry wait for W7 merge, seal, ordered cleanup,
and residue audit so the terminal work set reflects actual delivery friction.
The candidate-bound selection uses the current thirteen-seat role domain, may only widen over
fix deltas, and requires every selected seat to complete the lifecycle with no recommendations.
Historical fixed-ten counts remain legacy evidence rather than current selection authority.

---

### Pre-panel native review lanes

After a wave's slices converge and integration tests pass, but **before** any panel lane is
dispatched, the wave runs two read-only Copilot Task lanes **in parallel**:

```text
slices converge ──> integration tests ──┬──> reviewer Task ────────┐
                                        └──> rubber-duck Task ─────┴──> iterative /d2b-panel-round plan
                                                                          │ N/N selected, zero findings
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
one `/d2b-panel-round work` binding request, whose candidate-selected read-only seats bind
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

Both lanes are cheap and read-only. The selected panel lanes commonly
cost one to two times the coding duration. Sending a wave to panel with defects these lanes
would have caught spends the most expensive review capacity on the cheapest findings, and a
finding that arrives during panel forces a content change, which invalidates the snapshot and
every validation and panel record bound to it.

### Disposition of findings

- **Actionable finding from either lane, at any severity** - blocking. Fix it and return
  through convergence before dispatching the panel. Every constitution conflict is CRITICAL.
- **Nonblocking observation from either lane** - record it only in that lane's summary. It is
  not an actionable finding or recommendation. No round-count threshold applies to these
  native pre-panel gates.
- Anything either gate raises that is actually a process problem rather than a code problem
  belongs in the friction log.

---

## Panel convergence and delivery memory

Constitution 3.0 requires one comprehensive discovery from every selected seat, one shared
stable ledger, batched implementation responses and self-verification, and scoped
verification against that ledger and the full candidate. There is no round-count threshold
or later blocking-to-nonblocking transition. Pre-existing late MINOR and NIT observations
remain nonblocking history; admitted late BLOCKER and MAJOR findings remain blocking
(FR-051, FR-052).

The friction register is maintained continuously, not written at the end:

| Register | Purpose | Feeds |
| --- | --- | --- |
| [friction-log.md](./friction-log.md) | What slowed delivery, in W8's six categories | W8 triage directly |

The legacy [deferred-findings.md](./deferred-findings.md) file remains historical
compatibility data and receives no current lifecycle findings. Neither it nor the friction
log may contain panel transcripts, command output, or attestation payloads.

Per-wave obligations, folded into each wave's GATE task:

- Complete the shared ledger, responses, self-verification, and scoped verification
- Review the friction log at wave close
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
2. **Panel review** - the read-only lanes in the lifecycle selection artifact against the converged snapshot
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
| W5 | 12 manifest groups + completion graph | **12** for the manifest groups; after T595, T596-T599 run without claiming W6 work |
| W6 | 29 manifest/coordination groups + T604 acceptance | **up to 28** after dependencies - all 27 Provider dossiers, process-provider integration, core-controller coordination, and T604 stay scheduled in W6 |
| W7 | 5 | **5** |

Worktree setup per slice (cut from the wave integration branch, never from `v3`):

```bash
git worktree add -b adr046-w<N>-<slice> ../d2b-w<N>-<slice> adr046-w<N>-integrate
```

Before removing a worktree, delete its real `packages/target/` or the removal reclaims
nothing. Compiled-output dedup across worktrees comes from `sccache`, not a shared target dir.

### Panel fan-out

Panels run as exactly the read-only seats and profiles recorded by the candidate-bound lifecycle selection artifact, dispatched together on their recorded bindings. A current selection has no `rust` seat; Rust depth is a `software` profile. Strict historical fixed-ten records retain `rust` only as legacy data.

- Lanes are **read-only by contract**. They inspect the diff, the plan, and the integrator's
  supplied validation evidence. They MUST NOT run tests, builds, evals, or long validations
  unless the integrator explicitly asks a specific lane to.
- Because they are read-only they take **no heavy-gate slot**, so every selected lane runs concurrently
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
Dispatch the 26 remaining W6 Provider subagents freely, but serialize their `make test-integration` and
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

### Between waves - pipelined implementation, sequential exit

```text
historical W0-W5 -> T221 -> W6 -> W7 -> W8 -> Release
```

At least five predecessor selected-roster reviews plus green integration authorize successor
implementation start. The predecessor must still seal and merge, and the successor must
rebase, before the successor panel request, seal, or merge. Historical fixed-count records
do not define the current roster. For the one ADR-046 Wave 5 to Wave 6 transition, T221's
exact feature-owned historical-predecessor guard under generic Constitution 3.1.0 replaces
only the missing Wave 5 seal; it does not replace any Wave 6 gate.

### Within a wave - maximize parallelism

Parallel groups are file-disjoint by construction. Launch every ready group in the same
coordination cycle; a launch count below the ready count without a recorded blocker **fails
wave entry criteria**.

| Wave | Groups | Parallelism note |
| --- | --- | --- |
| W2 | 2 | Zero file-overlap edges; both groups start together. The 3 `primitives` items have no intra-wave dependency at all |
| W3 | 1 | Strictly serial by design; every Provider dossier depends on it |
| W4 | 6 | Five parallel member-spec groups plus `core-config-hub:w4`; all six start together |
| W5 (`adr046w5`) | historical only | Retain the unchecked completion graph as planning evidence; T219 records the exact merged no-seal disposition and no Wave 5 task is dispatched |
| W6 | all 27 Provider dossiers + T604 acceptance | T221 gates all Provider dossiers; T336-T355 implement the double-opt-in Network path and four-case matrix, then T604 consumes that merged implementation as acceptance-only work and T479/T480 bind its W6 result |
| W7 | 5 | Five file-disjoint closing groups; all five start together |

### The 14 manifest file-overlap ordering constraints

These are the manifest-derived edges that constrain shared files. Honor them as strict
ordering; the local completion graph additionally serializes `T591 -> T592` for
`transaction.rs` and `T595 -> T599` for `packages/d2b/src/dispatch.rs`:

- W5: `ADR046-device-006` -> `ADR046-nix-014` -> `ADR046-cli-011` -> `ADR046-nix-019` -> `ADR046-nix-031`
- W6: `ADR046-gpu-007` -> `ADR046-transport-unix-009` -> `ADR046-qemu-media-017` -> `ADR046-usbip-008`
- `ADR046-core-001` precedes `ADR046-device-007`, `-exec-013`, `-exec-015`, `-network-008`, `-telem-011`, `-zone-control-016`, `-zone-control-021`

### Unordered contended files - integrator only

The integrator-only rule applies when no explicit serial edge assigns every writer. A
contended file with named, non-overlapping-in-time owners is permitted: the plan must identify
all writers, state their order, and block the later branch until the earlier owner merges.
`transaction.rs` (`T591 -> T592`) is the representative slice-to-slice ownership transfer;
prep-to-slice ownership transfers are named in T589's file map and dependency chain.
`packages/d2b/src/dispatch.rs` transfers only after T595 freezes the host-generation
namespace; T599 is its sole later writer and may not remove or alias that namespace.
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

W3 was the narrowest historical point in the program. The current critical path begins at
T221. It must prove the fetched exact `origin/v3` base, accepted first-parent Constitution
3.1.0 integration commit, exact retained Wave 5 inventory and digests, and focused production
guard tests before the ordinary unanimous Wave 6 plan panel. The retained Wave 5 request is
already consumed with zero attestations and no seal; T219 records only that history. Final R9
keeps T336-T355 as authoritative W6 implementation under T221, followed by T604 and the
T479/T480 prospective acceptance and close gates.

### Incremental value

Each user story is independently demonstrable:

- **After historical `adr046w5`** - the only accepted output is the exact immutable merged
  boundary and retained no-seal state. T602 remains unchecked historical planning evidence;
  T219 is complete only as historical disposition. T336-T355 land the Network path
  prospectively in W6; T604 then consumes it on proposed F6 without owning production files,
  and T479/T480 accept the operator and Guest results together.
- **After W6** - full US1 completes only after T479/T480 accept exact-F6
  T604 operator activation/cleanup plus `Provider/runtime-cloud-hypervisor`
  production-boundary evidence for the declared Guest's real Cloud Hypervisor process effect,
  authenticated guest-control session, and ready state;
  missing, skipped, status-only, fake-boundary, other-family, or refusal evidence leaves US1 incomplete.
  US2 is complete. Capabilities arrive declaratively through Providers.
- **After W7** - US3 is complete. An existing host can move onto 3.0.
- **After W8 plus Release** - US4 is complete.

Note that no intermediate release ships (FR-045). These are internal checkpoints, not
deliverables.

### Task count

605 tasks: 18 pre-wave/process hygiene tasks (4 panel-model migration, 4 pipelined-wave
migration), 531 initial-scope work items, 18 wave entry/gate/merge tasks for W2-W7, 5 for the terminal wave,
4 added at W5/W7 by the earlier analysis remediation, 15 added to Wave 5 by the approved
production-completion amendment, 1 W6 T604 operator-acceptance task, 1 T603
feature-editor reconciliation task, and 12 for
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
