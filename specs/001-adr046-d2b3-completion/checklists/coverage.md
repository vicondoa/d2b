# Upstream Coverage Checklist: ADR-046 Provider Control Plane (d2b 3.0)

**Purpose**: Validate that the feature specification and its planning artifacts completely and
faithfully capture the ADR-046 normative set (55 specs, 545 work items, 129 frozen decisions).
This is a requirements-quality gate, not a verification pass - it tests whether the
requirements are written well enough to implement against, not whether anything works.

**Created**: 2026-07-29

**Feature**: [spec.md](../spec.md) | Also tests [plan.md](../plan.md) and
[spec-coverage.md](../spec-coverage.md)

**Depth**: Formal gate. This checklist SHOULD pass before W2 entry criteria are declared met
and before any implementation slice is dispatched. A defect found here is cheap; the same
defect found after a wave snapshot invalidates that wave's validation and panel evidence.

**How to use**: Each item asks whether something is *specified*, not whether it *works*. An
unchecked item means the requirements need work, not that the code is wrong. Record the
finding inline and either amend the artifact or record an explicit, justified delegation.

---

## Upstream Coverage Completeness

Does the feature spec capture the obligations that live in `docs/specs/`?

- [x] CHK001 Are requirements defined for all 19 standard ResourceTypes, or is per-type behavior explicitly delegated to the owning specs with a stated delegation boundary? [Coverage, Gap]
- [x] CHK002 Are requirements for `Quota`, `EmergencyPolicy`, and `Endpoint` present or explicitly delegated? None of these three terms appears anywhere in the spec. [Gap]
- [x] CHK003 Are requirements binding implementation to the 129 frozen decisions in the decision register documented? The register is never referenced in the spec. [Gap]
- [ ] CHK004 Are requirements defined for the three distinct reset scopes the spec set names - Full Zone reset, Provider reset, and Guest reset - or only for host cutover? [Gap, Coverage, Spec §FR-020]
- [ ] CHK005 Are requirements defined for the 11 feasibility work items, or does the spec address only the storage spike? [Gap, Spec §Context]
- [ ] CHK006 Are requirements for the streamline and friction-closure scope defined, given the spec commits to delivering the terminal wave? [Gap, Spec §FR-037]
- [ ] CHK007 Is the security-and-threat-model closing obligation represented as a requirement, such as the threat model being updated and re-validated at cutover? [Gap]
- [ ] CHK008 Are telemetry and audit retention requirements specified, or only content redaction? [Gap, Spec §FR-018]
- [ ] CHK009 Is the unsafe-local no-isolation posture rule captured - preserved in status, CLI, and audit, and prohibited as a telemetry label, span attribute, or log field? [Gap]
- [ ] CHK010 Are requirements for RETAIN-until-parity paths' eventual deletion specified distinctly from DELETE-row retirement? [Coverage, Spec §FR-023]
- [ ] CHK011 Are checkpoint identity and rollback-command requirements specified, beyond naming a rollback boundary? [Completeness, Spec §FR-022]
- [ ] CHK012 Is the incident-hold requirement scoped - whether it applies Zone-wide during a cutover window or per-Volume? [Clarity, Spec §FR-021]
- [ ] CHK013 Is Gate 0's standing re-evaluation obligation stated as a requirement rather than only as an assumption? [Completeness, Spec §Assumptions]

## Requirement Clarity and Measurability

Can these requirements be objectively assessed as written?

- [x] CHK014 Is "typical desktop-scale declaration" quantified with a resource count or shape? [Ambiguity, Spec §SC-002]
- [x] CHK015 Is "declared budget" identified by name and value rather than referenced abstractly? [Clarity, Spec §SC-012]
- [x] CHK016 Are the hard numeric targets that FR-030 governs enumerated in the spec itself? None of the ten targets appears in spec.md; they exist only in plan.md and spec-coverage.md. [Measurability, Spec §FR-030]
- [ ] CHK017 Is "operator-facing capability" defined precisely enough to make the parity criterion checkable? [Clarity, Spec §SC-003]
- [ ] CHK018 Is "desktop companion that consumes d2b's public operator contracts" defined by an objective test, so the release-blocking set cannot be argued? [Clarity, Spec §FR-039]
- [ ] CHK019 Is "host recovery point" defined - what qualifies, and what evidence constitutes attestation? [Ambiguity, Spec §FR-043]
- [x] CHK020 Is "actionable next step" specified well enough to be assessed without reviewer judgment? [Measurability, Spec §FR-017, §SC-004]
- [ ] CHK021 Is "reachable through the operator surface" defined well enough to decide when a foundation surface has stopped being deliberately unwired? [Clarity, Spec §SC-021]
- [ ] CHK022 Is "compatible version verified against the release candidate" defined with a pass condition? [Measurability, Spec §SC-024]

## Requirement Consistency and Conflicts

Do the requirements agree with each other and with the artifacts they depend on?

- [x] CHK023 Does fixing the total at 545 work items conflict with the plan's statement that the terminal wave's items are recorded later and are additional? [Conflict, Spec §SC-019, Plan §Wave sequencing]
- [ ] CHK024 Is the relationship between the operator-perceived 2-second envelope and the tighter component-level budgets stated, or do they read as competing targets? [Consistency, Spec §SC-002]
- [ ] CHK025 Is the tension between blocking release on external companions and forbidding a preview build they could adapt against resolved as a requirement, or only as plan-level mitigation? [Conflict, Spec §FR-039, §FR-045]
- [ ] CHK026 Is the removal-proof obligation internally consistent - one required per path, the migration map supplying only 3 of 16, and every work item carrying a non-empty proof field? [Consistency, Spec §FR-023, Research §R4]
- [ ] CHK027 Do "no partial-wave advance" and "W2 entry not blocked by missing W0/W1 seals" conflict, or is the distinction between entry evidence and exit evidence stated explicitly? [Consistency, Spec §FR-025, §FR-036]
- [ ] CHK028 Is the waiver's scope unambiguous regarding the nine delivery work items that remain Planned while owned by a waived wave? [Ambiguity, Spec §FR-034, §FR-035]
- [x] CHK029 Are the wave-to-destination assignments in the spec set and the implementation graph reconciled, given the recorded drift where two crate paths are listed under a wave that owns no work item for them? [Conflict, Plan §Recorded drift]

## Scenario Coverage

Are requirements present for each scenario class, or explicitly excluded?

- [ ] CHK030 Are requirements defined per distinct cutover phase, or only for the procedure as a whole? [Coverage, Spec §FR-020]
- [ ] CHK031 Are requirements defined for a wave that repeatedly fails its panel or cannot reach unanimous sign-off? [Gap, Exception Flow]
- [ ] CHK032 Are requirements defined for a specification amendment discovered mid-program, including its effect on in-flight validation evidence? [Coverage, Spec §Assumptions]
- [ ] CHK033 Are requirements defined for partial or stalled companion adaptation, distinct from the binary release-block? [Gap, Spec §FR-039]
- [ ] CHK034 Are requirements defined for the terminal case where a hard target cannot be met even after redesign? [Gap, Spec §FR-030]
- [ ] CHK035 Are requirements defined for rollback or recovery of a wave already merged into the integration lineage? [Gap, Recovery Flow]

## Non-Functional Requirements Coverage

- [ ] CHK036 Are hermetic execution-budget and runtime-ledger obligations represented, or does the spec cover only test-layer placement? [Gap, Spec §FR-032]
- [ ] CHK037 Are the panel's pinned provider, model, and reasoning-effort constraints stated as requirements, or is only unanimity captured? [Completeness, Spec §FR-026]
- [ ] CHK038 Is host continuity for the operator during the implementation waves stated as a requirement rather than only an assumption? [Completeness, Spec §Assumptions]
- [ ] CHK039 Are requirements defined for the contended-file prep discipline, so shared files are not concurrently edited by parallel slices? [Gap, Plan §Contended files]

## Traceability

- [x] CHK040 Is a mapping documented between this spec's FR and SC identifiers and the ADR-046 work-item IDs they correspond to? [Traceability, Gap]
- [ ] CHK041 Does every functional requirement trace to at least one owning spec in the 55-member set, so no requirement is locally invented? [Traceability]
- [x] CHK042 Is the delegation boundary stated explicitly - which obligations are restated here versus deliberately left in the spec set? [Traceability, Plan §Specification coverage]
- [ ] CHK043 Does the detail-preservation checklist have a named owner and a defined gate point at which it must pass? [Completeness, Coverage §Detail-preservation checklist]

## Dependencies and Assumptions

- [ ] CHK044 Is the assumption that companions can adapt without any published preview artifact validated or flagged as a risk with a mitigation? [Assumption, Spec §FR-045]
- [ ] CHK045 Is the assumption that the named design corrections will recover the memory deficit flagged as unvalidated, with a decision path if they do not? [Assumption, Research §RK-1]
- [ ] CHK046 Is the daily-driver validation risk acceptance recorded with an explicit accepter and a stated fallback? [Assumption, Spec §Assumptions]
- [ ] CHK047 Are the external dependencies required for cloud-backed Provider validation identified, including whether the necessary accounts and access exist? [Dependency, Gap, Spec §SC-022]

## Notes

- Check items off as completed: `[x]`
- An unchecked item is a **requirements defect**, not an implementation defect. Resolve it by
  amending the spec or plan, or by recording an explicit, justified delegation to the owning
  ADR-046 spec.
- Many gaps here may be legitimate delegation rather than omission. The point is that
  delegation must be **stated**, because an unstated gap and a deliberate delegation look
  identical to a reviewer and to a later implementer.
- Items referencing the decision register, ResourceTypes, or reset scopes were confirmed by
  scanning spec.md: those terms currently appear zero times.

## Resolution log

### Gate 1 - closed before `/speckit.tasks` (2026-07-29)

Six items resolved. These were blocking because an open defect here produces a *wrong task
list*, not merely an incomplete one.

| Item | Resolution |
| --- | --- |
| CHK001 | Delegation boundary section added to spec.md naming all 19 ResourceTypes and stating that FR-001 through FR-009 apply to them uniformly. |
| CHK002 | `Quota`, `EmergencyPolicy`, `Endpoint`, `ResourceExport`, and `ResourceImport` are now named explicitly, so they are visible rather than silently absent. |
| CHK023 | SC-019 no longer fixes the total at 545. It reads the manifest at release time and accounts for terminal-wave items recorded at W7 close. The Context paragraph was corrected to say 531 planned across W2-W7, not W2-W8. |
| CHK029 | FR-046 added: generated manifests are authoritative over spec prose on wave assignment, destination, and work-item identity; drift is raised as a separate amendment and never corrected inside a wave. |
| CHK040 | FR-to-owner and SC-to-owner traceability tables added to spec-coverage.md, including work-item prefixes and a `jq` retrieval command. |
| CHK042 | Delegation boundary stated explicitly, with the standing rule that delegation is not omission. |

**Finding surfaced while closing CHK040**: four requirements have **no upstream ADR-046
owner** - FR-039 and FR-040 (companion release blocker), FR-043 (recovery-point attestation),
and FR-046 (manifest authority). All four originate from recorded clarification decisions
rather than from the specification set. They are now labelled as locally added, so a reviewer
checking upstream fidelity will not hunt for counterparts that do not exist. FR-043 in
particular is *stricter* than its owning spec, which permits proceeding past the rollback
boundary without attestation.

**Also added**: FR-047, binding implementation to the 129 resolved decisions in the register
(closes CHK003, which was Gate 2).

### Remaining gates

- **Gate 2 - before W2 entry is declared met**: CHK013, CHK027, CHK028, CHK039. (CHK003 closed
  early by FR-047.)
- **Gate 3 - rides with its owning wave**: CHK004-CHK012 (W5/W7 content), plus the clarity,
  scenario, and assumption items whose subject matter is waves away. Record each as a
  deliberate deferral naming its wave, so a scheduled obligation is never mistaken for a gap.
- **Date-bound regardless of gate**: CHK025 must be resolved before W5 publishes replacement
  contracts, since that is the last moment companions can begin adapting. CHK047 (cloud
  account access) is cheap now and expensive at W6.

### Analysis remediation (2026-07-29)

`/speckit.analyze` found 16 issues across spec.md, plan.md, and tasks.md. Clear-cut fixes were
applied; two ambiguous decisions were escalated. Three further checklist items closed as a
side effect:

| Item | Resolution |
| --- | --- |
| CHK015 | SC-012 now names the exact budget: whole-process resident memory at or below 24,576 KiB with no baseline subtraction, against the 25,216 KiB current measurement. |
| CHK016 | FR-030 now enumerates all ten hard targets in a table inside spec.md, rather than leaving them only in plan.md and spec-coverage.md. |
| CHK020 | FR-017 now defines "actionable next step" as naming at least one concrete operator action - a command, a configuration change, or a named artifact to inspect - and explicitly rejects a bare failure notice or generic retry. |

CHK014 (SC-002's "typical desktop-scale declaration") remains **open and escalated**: it needs
a number, which is a product decision rather than a defect with an obvious correction.

### Operator decisions (2026-07-29)

| Decision | Outcome |
| --- | --- |
| FR-043 governance | **Program-local**, outside the work-item manifest. Accepted consequence: the W7 seal does not enforce it. Recorded as a tracked deviation in plan.md Complexity Tracking. |
| CHK014 - SC-002 scale | **Resolved**: a single-Zone declaration of 10 to 20 resources. |
| Panel model | **`gemini-3.1-pro-preview`**, run as 10 read-only subagent lanes. Requires a spec amendment plus a code change (T581-T584) before any wave can seal. |
