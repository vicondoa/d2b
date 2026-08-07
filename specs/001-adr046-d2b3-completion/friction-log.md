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

| ID | Date | Category | Wave | Friction | Impact | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F001 | 2026-07-29 | `signoff` | W1 | A single wave required 21 follow-up rounds (W0 required 14) | Review consumed a large multiple of implementation time | Mitigated by constitution 2.1.0 bounded deferral; verify the effect at W2 close |
| F002 | 2026-07-29 | `signoff` | all | Panel review commonly runs 1-2x the coding duration; strict serialization idles implementation capacity for more than half of each cycle | Motivated the pipelined-wave amendment | Mitigation landed in T585-T588: dependency-ready successor implementation may start at the qualified threshold, while panel request, seal, and merge remain ordered after predecessor merge and successor rebase; verify the effect at W2 close |
| F003 | 2026-07-29 | `signoff` | W0/W1 | Delivery state holds 10 competing W0 candidates, 1 panel-request, 0 receipts, 0 seals; W1 has no delivery state at all | Neither wave produced a seal; the contract's exit criteria were not met in practice | Historical evidence retained under FR-034; it is non-authorizing and FR-036 now blocks continuation pending an external constitution amendment |
| F004 | 2026-07-29 | `test` | W1 | The storage spike missed its whole-process RSS budget by 640 KiB (2.6%), deferring the production engine and its watch consumer from W1 to W5 | Blocks the critical path; W5 cannot start its store chain until corrections are designed | T007 prototypes the corrections early so W5 confirms rather than discovers |
| F005 | 2026-07-29 | `codegen` | W2 | `ADR-046-validation-and-delivery` §3.2 names two crate paths under W2 that no W2 work item targets; the graph assigns them to W4 | Spec prose and generated graph disagree; an implementer following prose targets the wrong wave | T575 raises it as a separate amendment; FR-046 makes the graph authoritative |
| F006 | 2026-07-29 | `test` | all | The migration map supplies explicit removal proofs for only 3 of its 16 DELETE rows | FR-023 requires one per path; 13 must be authored | T576 inventories and assigns them |
| F007 | 2026-08-02 | `merge` | W5 | Earlier W5 implementation was serialized through one integration lineage instead of launching every dependency-ready, file-disjoint slice in one coordination cycle; one committed execution slice remained on a sibling branch at the W5 entry audit | Integration had to reconstruct slice boundaries and audit unmerged sibling work before the wave could proceed | Corrected for this run: the execution slice was landed as its own merge commit, and all newly uncovered disjoint slices are dispatched in one batch |
| F008 | 2026-08-06 | `signoff` | all | Root `AGENTS.md` and `docs/contributing/**` still describe wave ordering in terms that conflict with the accepted pipelined implementation-start contract | Feature plans repeatedly need to restate the distinction between pipelined implementation and strictly ordered panel/seal/merge | Escalated to the contributor-document owner; outside this feature root and not edited by this batch |
| F009 | 2026-08-06 | `signoff` | W2-W5 | The external ADR/spec/tooling says one binding request per wave, while earlier feature-local recovery prose incorrectly described a distinct-successor request after findings | Wave 5 already consumed its request, and feature-local prose cannot free it or authorize a successor | T219 now has no binding or successor-request path. It remains blocked until an accepted external disposition preserves the retained request and expressly authorizes a non-request close action. The external scopes remain escalated and deliberately unedited by this batch |
| F010 | 2026-08-06 | `merge` | W2/W4 | T008 and T037 remained unchecked after dependent implementation tasks were completed | Their pre-dispatch conditions cannot be made true prospectively, so treating them as ordinary blockers would either deadlock the plan or invite a false retroactive check | Reclassified both as historical entry attestations. Only contemporaneous base-bound evidence may check them; otherwise they stay open and exact F2/F4 must carry one candidate-bound remedial requalification record before T029/T071 may request a panel, seal, or merge |
| F011 | 2026-08-06 | `signoff` | W0-W5 | Feature-local prose treated the W0/W1 historical record and late W2-W4 remedial panels as authority to continue despite Constitution Principle VI | An artifact below the feature root cannot amend the constitution; continuing would make the plan itself authorize a constitutional violation | Escalated outside this exclusive feature root: FR-036 now requires a separate accepted Principle VI constitution amendment, ancestor-bound to the execution base, before any implementation, resume, fix, work-panel, seal, merge, or advance action |
| F012 | 2026-08-06 | `signoff` | W4 | Feature-local W4 prose asserted Host/site plus Network double opt-in, while untouched external `ADR-046-resources-network` normatively defines the Network field as the sole opt-in | Historical W4 close cannot honestly claim the four-case double-opt-in implementation without a preceding normative version/migration | T070/T071 remain blocked on an accepted external correction. It must either prove a versioned amendment and migration preceded actual F4 and bind all four cases, or preserve sole Network opt-in as W4's authoritative behavior and leave double opt-in prospectively unimplemented |

## Standing obligations

- **Log continuously.** A friction point recorded at W7 close from memory is worth much less
  than one recorded when it was felt. Add the entry in the wave where it happened.
- **Categorize on entry.** An uncategorized entry cannot be triaged into W8.
- **Record mitigation and verify it.** An entry whose Status claims mitigation should be
  re-checked at the next wave close. F001 and F002 both carry unverified mitigations right now.
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
  mandatory successor rebase.
- **FR-043 remains program-local rather than manifest-counted.** T580's candidate-bound
  primary recovery guard and the T555/T556 close checks are the required fail-closed
  enforcement before W7 panel, seal, or merge.
- **W6 is 258 work items across 27 crates** - nearly half the program in one wave. If wave
  size proves to be the driver of deferral pressure, W6 is where it will show.
