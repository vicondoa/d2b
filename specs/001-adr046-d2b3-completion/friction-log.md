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
| F006 | 2026-07-29 | `test` | all | The initial working note undercounted the migration map at 3 proofed rows out of 16 DELETE rows | FR-023 applies to every removed DELETE or REPLACE path; the current 48-row census records 5 proofed DELETE rows and 33 outstanding DELETE/REPLACE rows overall | T576 preserves the finding, corrects the count, and assigns the outstanding rows |
| F007 | 2026-08-02 | `merge` | W5 | Earlier W5 implementation was serialized through one integration lineage instead of launching every dependency-ready, file-disjoint slice in one coordination cycle; one committed execution slice remained on a sibling branch at the W5 entry audit | Integration had to reconstruct slice boundaries and audit unmerged sibling work before the wave could proceed | Corrected for this run: the execution slice was landed as its own merge commit, and all newly uncovered disjoint slices are dispatched in one batch |
| F008 | 2026-08-06 | `signoff` | all | Root `AGENTS.md` and `docs/contributing/**` still describe wave ordering in terms that conflict with the accepted pipelined implementation-start contract | Feature plans repeatedly need to restate the distinction between pipelined implementation and strictly ordered panel/seal/merge | Escalated to the contributor-document owner; outside this feature root and not edited by this batch |
| F009 | 2026-08-06 | `signoff` | W2-W5 | The external ADR/spec/tooling says one binding request per wave, while earlier feature-local recovery prose incorrectly described another request after findings | Wave 5 already consumed its request, and feature-local prose cannot free or replace it | T219 now performs non-authorizing historical adjudication only and has only the accepted-external-disposition path. Its actionable refusal requires that disposition to preserve and name the retained request and expressly authorize a non-request close action. T589's retained-state fixture proves nonbinding phase rounds leave delivery bytes unchanged. The external scopes remain escalated and deliberately unedited by this batch |
| F010 | 2026-08-06 | `merge` | W2/W4 | T008 and T037 remained unchecked after dependent implementation tasks were completed | Their pre-dispatch conditions cannot be made true prospectively, so treating them as ordinary blockers would either deadlock the plan or invite a false retroactive check | Reclassified both as historical entry attestations. Only contemporaneous base-bound evidence may check them; otherwise they stay open. T029/T071 perform external historical adjudication only against retained records or an accepted external correction and may not create remedial evidence or request a panel, seal, or merge |
| F011 | 2026-08-06 | `signoff` | W0-W5 | Feature-local prose treated the W0/W1 historical record and late W2-W4 remedial panels as authority to continue despite Constitution Principle VI | An artifact below the feature root cannot amend the constitution; continuing would make the plan itself authorize a constitutional violation | Escalated outside this exclusive feature root: FR-036 now requires a separate accepted Principle VI constitution amendment, ancestor-bound to the execution base, before any implementation, resume, fix, work-panel, seal, merge, or advance action |
| F012 | 2026-08-06 | `signoff` | W4 | Feature-local W4 prose asserted Host/site plus Network double opt-in, while untouched external `ADR-046-resources-network` normatively defines the Network field as the sole opt-in | Historical W4 close cannot honestly claim the four-case double-opt-in implementation without a preceding normative version/migration | W4 adjudication, T070, T071, and T220 remain blocked on an accepted external correction. It must either prove a versioned amendment and migration preceded actual F4 and bind Network false/Host false, Network false/Host true, Network true/Host false, and Network true/Host true, or preserve sole Network opt-in as W4's authoritative behavior and leave double opt-in prospectively unimplemented. No feature status correction can unblock it |
| F013 | 2026-08-07 | `signoff` | W5 | The external source-generation compatibility floor was described as an open-ended artifact family, so evidence could not prove a complete atomic floor or distinguish a stale or cross-disposition member | T589 dispatch could become success-shaped with an incomplete or mixed source actor even though no feature task owns its repair | Feature artifacts now define the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census and the closed `missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and `cross-disposition` poison cases. Production or repair of every member remains escalated to the external source-generation owner; this planning-only batch edits no external artifact, normative spec, source test, or implementation |
| F014 | 2026-08-07 | `signoff` | W5 | The accepted external delivery contract does not yet pin the complete incident mismatch lifecycle or source-floor hash/issuer byte oracle required by the Wave 5 plan | T589 cannot implement or attest stable incident recovery, copied-issuer rejection, or exact hash compatibility without silently inventing normative behavior | Escalated to the external `ADR-046-validation-and-delivery` owner: Version 2 must pin the stable-id cause/remediation/exit projections, no-unlink `mismatch-retained` branch, publication/recovery sync protocol, exact 15-digest/four-signature vectors, and disposition-pinned copied-issuer rejection before T589. The source producer/installer and validator remain separate conforming authorities. This feature-only batch edits none of those external artifacts |
| F015 | 2026-08-07 | `signoff` | W5 | Round 19 found that the planned irreconcilable-resolution census included resolution leaves that embed its digest, used a non-identity-bearing over-bound sentinel, left invalid/unstable census recovery underspecified, and did not pin complete independent negative/canary registries | A self-referential digest cannot be constructed; a copied sentinel could become success-shaped; cleanup could race a live incident owner; and malformed receipt, census, source-floor, unit, or post-mutation cases could disappear behind shared enumeration | Feature artifacts now define a frozen primary-evidence scope excluding resolution/disposition leaves, complete and identity-bearing bounded-failure encodings with raw `01ff` non-authorizing, inspect/apply/successor convergence for `evidence-census-conflict`, persisted anchor/metadata/status/resolution preimages, every-ancestor durability, one cleanup/live-owner lock matrix, one shared typed SC-002 hash oracle, exact 61 receipt/45 census/32 source-floor receipt registries, independent mutation/unit matrices, and the complete fifteen-row apply-peer registry. The accepted external Version 2 contract, generated manifests, source/tests, and schema bytes remain out of scope and must adopt these contracts on an ancestor of T589 before implementation |
| F016 | 2026-08-07 | `signoff` | W5 | Round 20 found that successor selection was not frozen before disposition signing, source-floor issuer authentication lacked its own unforgeable result, incident replay/retention did not bind every structured kind-specific preimage component or recursively enumerate paths, cleanup serialization lacked a Rust live-owner authority, and source-floor/mutation negative prose exceeded the closed registries | An authority could sign one successor while apply/admit used another; copied authority digests or decoded DTOs could become success-shaped; nested incident evidence could mutate outside a top-level census; cleanup helpers could bypass lock ownership; and omitted negative cases could false-green | Feature artifacts now require a durable pre-signing successor freeze plus canonical authority request and one triplet through apply/admit; private nonserializable `AuthenticatedSourceFloorIssuerProvenance` consumed into the private validated-floor result; one complete structured incident preimage repeated across all durable records; recursive primary-evidence and retention scopes with bounded-failure path identity/replay; private `SidecarCleanupOwner`; and exact source-floor 32/20/21 plus 15-case post-mutation registries. The external Version 2 contract, generated manifests, source/tests, schemas, ADRs, constitution, contributor docs, and panel artifacts remain unedited and must adopt these contracts before T589 |
| F017 | 2026-08-07 | `signoff` | W5 | Round 21 found dead-end recovery projections, a contradictory request-to-disposition transform, a flat/recursive census grammar split, partial bounded-failure authority, cleanup authority not lifetime-bound to its OFD guard, non-atomic request output, and incompletely pinned source-floor/mutation/pre-start/root/unit/redaction matrices | An operator could be parked without a successor action; an authority and verifier could sign different bytes; omitted descendants or a stale cleanup owner could authorize mutation; a crash could expose ambiguous request output; and shared enumeration could false-green | Feature planning now pins every cause and handoff recovery-pending/irreconcilable inspect/action/status/successor row; the exact 19-field request and 19-to-22 transform; one recursive directory/file grammar; full-descendant coverage or admission denial; `CandidateSidecarGuard` plus `SidecarCleanupOwner<'guard>`; anchored crash-safe request output and all-descriptor CLOEXEC; literal 15-edge, 90-case, and 91-case matrices; 15 pre-start/root, 27 unit, 56 census, 25 output, and thirteen recovery-redaction entries. External source, tests, Nix, schemas, normative/reference docs, ADRs, constitution, contributor guidance, changelog, and panel artifacts remain unedited and must move only through their owners before T589 |

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
