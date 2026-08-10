# Friction Log

**Feature**: `001-adr046-d2b3-completion` | **Opened**: 2026-07-29

Durable record of what slowed delivery down. Required by constitution 2.1.0 Principle VI
("Delivery memory").

This is **not** a retrospective document written at the end. It is an input to planning,
maintained continuously, and it feeds the terminal wave directly: W8 has no spec members
because its contents *are* the accumulated friction, triaged at W7 close into the six
categories below.

## Categories

W8's destinations map one-to-one onto these, so categorize accordingly:

| Category | Typical destination |
| --- | --- |
| `signoff` | Panel process, review latency, roster, deferral pressure |
| `build` | Compile times, toolchain, sccache, worktree setup |
| `test` | Suite runtime, flakiness, gate placement, `tests/tools/` |
| `merge` | PR flow, stacking, rebasing, CI latency, `merge-eligibility` |
| `codegen` | `xtask gen-*`, drift gates, manifest regeneration |
| `disk` | Store growth, target dirs, GC, the 10 GiB preflight |

## What may be recorded

Classification metadata and impact. **No panel transcript, validation command output, or
attestation payload.** Describe the friction, not the review text that surfaced it.

## Log

<!-- RETIRED-READONLY-BEGIN -->

| ID | Date | Category | Wave | Friction | Impact | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F001 | 2026-07-29 | `signoff` | W1 | A single wave required 21 follow-up rounds (W0 required 14) | Review consumed a large multiple of implementation time | Constitution 3.0 superseded bounded deferral with one Discover-Fix-Verify lifecycle and a stable shared ledger |
| F002 | 2026-07-29 | `signoff` | all | Panel review commonly runs 1-2x the coding duration; strict serialization idles implementation capacity for more than half of each cycle | Motivated the pipelined-wave amendment | Mitigation landed in T585-T588: dependency-ready successor implementation may start at the qualified threshold, while panel request, seal, and merge remain ordered after predecessor merge and successor rebase. The exact ADR-046 contract instantiates generic Constitution 3.1.0 only for the Wave 5 to Wave 6 historical predecessor exception |
| F003 | 2026-07-29 | `signoff` | W0/W1 | Delivery state holds 10 competing W0 candidates, 1 panel-request, 0 receipts, 0 seals; W1 has no delivery state at all | Neither wave produced a seal; the contract's exit criteria were not met in practice | The exact ADR-046 validator/tooling contract applies generic Constitution 3.1.0 to the deviations only as closed history through merged Wave 5. It does not reconstruct a seal or establish precedent |
| F004 | 2026-07-29 | `test` | W1 | The storage spike missed its whole-process RSS budget by 640 KiB (2.6%), deferring the production engine and its watch consumer from W1 to W5 | Blocks the critical path; W5 cannot start its store chain until corrections are designed | T007 prototypes the corrections early so W5 confirms rather than discovers |
| F005 | 2026-07-29 | `codegen` | W2 | `ADR-046-validation-and-delivery` §3.2 names two crate paths under W2 that no W2 work item targets; the graph assigns them to W4 | Spec prose and generated graph disagree; an implementer following prose targets the wrong wave | T575 raises it as a separate amendment; FR-046 makes the graph authoritative |
| F006 | 2026-07-29 | `test` | all | The initial working note undercounted the migration map at 3 proofed rows out of 16 DELETE rows | FR-023 applies to every removed DELETE or REPLACE path; the current 48-row census records 5 proofed DELETE rows and 33 outstanding DELETE/REPLACE rows overall | T576 preserves the finding, corrects the count, and assigns the outstanding rows |
| F007 | 2026-08-02 | `merge` | W5 | Earlier W5 implementation was serialized through one integration lineage instead of launching every dependency-ready, file-disjoint slice in one coordination cycle; one committed execution slice remained on a sibling branch at the W5 entry audit | Integration had to reconstruct slice boundaries and audit unmerged sibling work before the wave could proceed | Corrected for this run: the execution slice was landed as its own merge commit, and all newly uncovered disjoint slices are dispatched in one batch |
| F008 | 2026-08-06 | `signoff` | all | Root `AGENTS.md` and `docs/contributing/**` still describe wave ordering in terms that conflict with the accepted pipelined implementation-start contract | Feature plans repeatedly need to restate the distinction between pipelined implementation and strictly ordered panel/seal/merge | Escalated to the contributor-document owner; outside this feature root and not edited by this batch |
| F009 | 2026-08-06 | `signoff` | W2-W5 | The external ADR/spec/tooling says one binding request per wave, while earlier feature-local recovery prose incorrectly described another request after findings | Wave 5 already consumed its request, and feature-local prose cannot free or replace it | The exact ADR-046 validator/tooling contract applies generic Constitution 3.1.0 to the state with zero attestations and no seal. T219 is historical disposition complete. All Wave 5 recovery and reconstructed-close instructions are retired |
| F010 | 2026-08-06 | `merge` | W2/W4 | T008 and T037 remained unchecked after dependent implementation tasks were completed | Their pre-dispatch conditions cannot be made true prospectively, so treating them as ordinary blockers would either deadlock the plan or invite a false retroactive check | Reclassified both as historical entry attestations. Only contemporaneous base-bound evidence may check them; otherwise they stay open. T029/T071 perform external historical adjudication only against retained records or an accepted external correction and may not create remedial evidence or request a panel, seal, or merge |
| F011 | 2026-08-06 | `signoff` | W0-W5 | Feature-local prose treated the W0/W1 historical record and late W2-W4 remedial panels as authority to continue despite Constitution Principle VI | An artifact below the feature root cannot amend the constitution; continuing would make the plan itself authorize a constitutional violation | Constitution 3.1.0 now supplies a generic disposition; the exact ADR-046 validator/tooling contract owns the history through merged Wave 5. T221 must prove the accepted first-parent generic-amendment lineage and retained bytes before prospective Wave 6 entry |
| F012 | 2026-08-06 | `signoff` | W4/W5 | Feature-local W4 prose asserted Host/site plus Network double opt-in, while untouched external `ADR-046-resources-network` normatively defines sole Network opt-in and committed code lacks the production adapter | Historical W4 close cannot honestly claim the four-case implementation | W4 bytes remain historical. T336-T355 and all four production cases remain prospective W6 work under T221; the accepted Wave 5 historical disposition does not claim those W6 results |
| F013 | 2026-08-07 | `signoff` | W5 | The source-generation compatibility floor had no closed owner or implementation | An incomplete or mixed source actor could become success-shaped | Code canon confirms the handoff was absent and the retired ownership was non-authorizing |
| F014 | 2026-08-07 | `signoff` | W5 | The external delivery contract did not yet own the complete SC-002 incident and source-floor contract | Feature tasks could silently invent normative behavior | Accepted Version 2 and the eight stable `VD2-SC002-*` identifiers are the sole authority; this feature retains no schema, fixture, registry, or transition copy |
| F015 | 2026-08-07 | `signoff` | W5 | Irreconcilable incident evidence and negative coverage were underspecified | Self-reference, copied evidence, or shared enumeration could false-green | The correction is owned by generated `VD2-SC002-INCIDENT`, `VD2-SC002-RECOVERY`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY` rows; missing or failing rows block T589 |
| F016 | 2026-08-07 | `signoff` | W5 | Successor binding, issuer authentication, retention scope, and cleanup authority were incomplete | A substituted successor, copied authority, or stale cleanup owner could authorize mutation | Generated `VD2-SC002-DISPOSITION`, `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY` rows solely own the correction |
| F017 | 2026-08-07 | `signoff` | W5 | Recovery actions, request transformation, recursive coverage, and publication safety were incomplete | A dead-end or partially covered recovery path could become success-shaped | Generated `VD2-SC002-PUBLICATION`, `VD2-SC002-DISPOSITION`, `VD2-SC002-RECOVERY`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY` rows solely own the correction |
| F018 | 2026-08-07 | `signoff` | W5 | Hard-ceiling recovery, publication replay, node identity, handoff state, and redaction coverage were incomplete | Crash residue, aliasing, or an incomplete handoff could be presented as recoverable | Accepted Version 2 and generated `VD2-SC002-*` ownership solely define the correction; feature tasks consume only their rows and cannot restate counts or matrices |
| F019 | 2026-08-08 | `signoff` | all | Separate post-round-22 consistency analysis found that feature-local round-nine LOW/MEDIUM deferral prose remained operative after Constitution 3.0 superseded it | Review and close tasks could authorize a deferral the current constitution forbids | Corrected in this separate consistency batch: Constitution 3.0 Discover-Fix-Verify now governs and the legacy deferral register is historical only |
| F020 | 2026-08-08 | `signoff` | all | Separate post-round-22 consistency analysis found fixed-ten, 5-of-10, and pipeline prohibitions conflicting with the selected thirteen-seat widen-only lifecycle | Entry and close instructions could select or gate the wrong lifecycle | Corrected in this separate consistency batch: current selection uses the thirteen-seat role domain, only widens over fix deltas, and keeps pipelined implementation with sequential exit |
| F021 | 2026-08-08 | `merge` | W5/W6 | Separate post-round-22 consistency analysis found a feature-local acceptance task trying to pull authoritative Network work into W5 | The feature graph could violate wave and panel ownership | Final R9 kept Network implementation authoritative and the local task acceptance-only |
| F022 | 2026-08-08 | `signoff` | all | Separate post-round-22 consistency analysis found quickstart clearance limited to HIGH and CRITICAL despite FR-054 covering every actionable content finding | Lower-severity actionable content defects could reach panel dispatch | Corrected in this separate consistency batch: every actionable content finding must clear before panel dispatch |
| F023 | 2026-08-08 | `test` | all | Separate post-round-22 consistency analysis found a 33-row checklist census where the checked table currently has 22 rows | Coverage reporting overstated the authoritative checklist census | Corrected in this separate consistency batch: the current checklist census is 22 of 22 |
| F024 | 2026-08-08 | `merge` | W4 | Separate post-round-22 consistency analysis found the implementation-debt summary reporting 32 W4 items instead of the authoritative 31 | Debt accounting disagreed with the wave task census | Corrected in this separate consistency batch: W4 has 31 work items |
| F025 | 2026-08-09 | `signoff` | W5/W6 | Feature artifacts still required an impossible Wave 5 recovery and seal after Wave 5 had merged | The program could not enter Wave 6 without fabricating retroactive evidence | The exact ADR-046 validator/tooling contract applies generic Constitution 3.1.0 only to the retained no-seal state through merge `177235ed37188b3be87525e7f016fb43401574c5`. T219 is historical disposition complete; T221 now requires exact fetched-base lineage/state validation and the ordinary unanimous Wave 6 plan panel |

<!-- RETIRED-READONLY-END -->

| ID | Date | Category | Wave | What happened | Cost | Resolution |
| --- | --- | --- | --- | --- | --- | --- |
| F026 | 2026-08-10 | `signoff` | W6 entry | T221 discovery found that active feature prose cited an accepted Version 2 delivery contract and generated `VD2-SC002-*` traceability that are absent from the committed tree, while fifteen consumed foundations retain Planned W5 labels and thirteen dossier Provider crates are absent | The plan could either fabricate external authority, reopen immutable W5, or launch Provider lanes without their prerequisites | Preserve F014-F018 as historical observations, remove their stale active authority claims, adopt the fifteen obligations prospectively through T607-T609, create shared contracts and all missing scaffolds through T606, and require a replacement T221 snapshot/panel that authorizes T606 only |
| F027 | 2026-08-10 | `signoff` | W6 entry | Replacement discovery found the new local foundation exception, completion states/evidence, all-group foundation map, writer handoffs, dispatch readiness, T221 invalidation boundary, command evidence, and durable approval record incomplete; it also found stale task/edge and fixed-agent counts | Parallel lanes could race shared writers, status prose could invalidate or widen approval, and a hand-copied zero-launch claim could replace measured dispatch state | Close the exception over exactly seven tasks, bind every manifest group to T606-T609, derive handoffs from candidate manifests, derive readiness from an external ledger, distinguish material from status-only updates, require structured command evidence and durable non-authentication approval receipt, and correct counts to 609 tasks/600 nodes/1963 edges/36 W6 groups |
| F028 | 2026-08-10 | `signoff` | W6 entry | Final discovery found entry/final/work selections conflated, approval consumption unclear, hand-authored command/census/graph inputs, incomplete writer orders, and readiness/disk handling ambiguous; it also found stale system-core state and incomplete release treatment | An entry approval could leak into work selection, Completed could be presented as Merged, eligibility could be pre-recorded, and parallel writers or disk exhaustion could corrupt the candidate | Separate the entry/final plan and work selections using current snapshot/plan-approval/panel verbs; consume canonical `d2b-panel/approval`; use validate/complete accepted commit/tree evidence with T479/T480 prospective Merged reconciliation; auto-derive graph/material; close profiles/census; retain eligibility only after evaluation; enforce one-path-one-order and the existing 10 GiB preflight; state system-core Version 2/Planned accurately; and require per-group changelog plus final fold |

## Standing obligations

- **Log continuously.** A friction point recorded at W7 close from memory is worth much less
  than one recorded when it was felt. Add the entry in the wave where it happened.
- **Categorize on entry.** An uncategorized entry cannot be triaged into W8.
- **Record mitigation and verify it.** An entry whose Status claims mitigation should be
  re-checked at the next wave close. F002 still carries an unverified mitigation.
- **Escalate structural friction.** If the same category recurs across three waves, it is not
  friction, it is a design problem in the delivery process, and it should become a task rather
  than another log line.

## Known friction status

Tracked here so it is not lost between waves:

- **Panel model migration was resolved.** T581-T584 landed the prior Gemini
  binding; current panel requests use `gpt-5.6-sol` at `xhigh`, while the
  Gemini/high pair remains readable for compatibility.
- **Pipelined implementation start is executable.** T585-T588 landed the accepted §4 and
  delivery-tool changes. Only dependency-ready implementation start is pipelined; successor
  panel request, seal, and merge remain strictly ordered after predecessor seal/merge and the
  mandatory successor rebase, except for the exact feature-owned Wave 5 to Wave 6 historical
  predecessor disposition under generic Constitution 3.1.0.
- **FR-043 remains program-local rather than manifest-counted.** T580's candidate-bound
  primary recovery guard and the T555/T556 close checks are the required fail-closed
  enforcement before W7 panel, seal, or merge.
- **W6 is 258 work items across 27 crates** - nearly half the program in one wave. If wave
  size proves to be the driver of deferral pressure, W6 is where it will show.
