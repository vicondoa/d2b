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
- [x] CHK013 Is Gate 0's standing re-evaluation obligation stated as a requirement rather than only as an assumption? [Completeness, Spec §Assumptions]

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
- [x] CHK027 Do "no partial-wave advance" and "W2 entry not blocked by missing W0/W1 seals" conflict, or is the distinction between entry evidence and exit evidence stated explicitly? [Consistency, Spec §FR-025, §FR-036]
- [x] CHK028 Is the waiver's scope unambiguous regarding the nine delivery work items that remain Planned while owned by a waived wave? [Ambiguity, Spec §FR-034, §FR-035]
- [x] CHK029 Are the wave-to-destination assignments in the spec set and the implementation graph reconciled, given the recorded drift where two crate paths are listed under a wave that owns no work item for them? [Conflict, Plan §Recorded drift]

## Scenario Coverage

Are requirements present for each scenario class, or explicitly excluded?

- [ ] CHK030 Are requirements defined per distinct cutover phase, or only for the procedure as a whole? [Coverage, Spec §FR-020]
- [ ] CHK031 Are requirements defined for a wave that repeatedly fails its panel or cannot reach unanimous sign-off? [Gap, Exception Flow]
- [x] CHK032 Are requirements defined for a specification amendment discovered mid-program, including its effect on in-flight validation evidence? [Coverage, Spec §Assumptions]
- [ ] CHK033 Are requirements defined for partial or stalled companion adaptation, distinct from the binary release-block? [Gap, Spec §FR-039]
- [ ] CHK034 Are requirements defined for the terminal case where a hard target cannot be met even after redesign? [Gap, Spec §FR-030]
- [ ] CHK035 Are requirements defined for rollback or recovery of a wave already merged into the integration lineage? [Gap, Recovery Flow]

## Non-Functional Requirements Coverage

- [ ] CHK036 Are hermetic execution-budget and runtime-ledger obligations represented, or does the spec cover only test-layer placement? [Gap, Spec §FR-032]
- [ ] CHK037 Are the panel's pinned provider, model, and reasoning-effort constraints stated as requirements, or is only unanimity captured? [Completeness, Spec §FR-026]
- [ ] CHK038 Is host continuity for the operator during the implementation waves stated as a requirement rather than only an assumption? [Completeness, Spec §Assumptions]
- [x] CHK039 Are requirements defined for the contended-file prep discipline, so shared files are not concurrently edited by parallel slices? [Gap, Plan §Contended files]

## Traceability

- [x] CHK040 Is a mapping documented between this spec's FR and SC identifiers and the ADR-046 work-item IDs they correspond to? [Traceability, Gap]
- [ ] CHK041 Does every functional requirement trace to at least one owning spec in the 55-member set, so no requirement is locally invented? [Traceability]
- [x] CHK042 Is the delegation boundary stated explicitly - which obligations are restated here versus deliberately left in the spec set? [Traceability, Plan §Specification coverage]
- [ ] CHK043 Does the detail-preservation checklist have a named owner and a defined gate point at which it must pass? [Completeness, Coverage §Detail-preservation checklist]

## Dependencies and Assumptions

- [ ] CHK044 Is the assumption that companions can adapt without any published preview artifact validated or flagged as a risk with a mitigation? [Assumption, Spec §FR-045]
- [ ] CHK045 Is the assumption that the named design corrections will recover the memory deficit flagged as unvalidated, with a decision path if they do not? [Assumption, Research §RK-1]
- [ ] CHK046 Is the daily-driver validation risk acceptance recorded with an explicit accepter and a stated fallback? [Assumption, Spec §Assumptions]
- [x] CHK047 Are the external dependencies required for cloud-backed Provider validation identified, including whether the necessary accounts and access exist? [Dependency, Gap, Spec §SC-022]

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

### Gate 1 - closed before `/speckit-tasks` (2026-07-29)

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

`/speckit-analyze` found 16 issues across spec.md, plan.md, and tasks.md. Clear-cut fixes were
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

### Gate 2 - closed before W2 entry (2026-07-29)

All four Gate 2 items are resolved by amendments to spec.md. One Gate 3 scenario item
(CHK032) closed as a side effect.

| Item | Resolution |
| --- | --- |
| CHK013 | **FR-056** added. Gate 0's re-evaluation obligation is now a numbered functional requirement rather than only an Assumptions bullet: any amendment to a member specification re-opens that spec's validation and panel evidence and re-triggers Gate 0 across the whole manifest, and Gate 0 must pass again before any wave depending on the amended spec may seal. Stated as a standing obligation for W2 through W8, not a one-time precondition. The Assumptions bullet now points at FR-056. |
| CHK027 | **FR-057** added, stating the entry-evidence versus exit-evidence distinction explicitly. Entry evidence is what a wave needs to START implementing; exit evidence is what a wave needs to be DELIVERED - sealed and merged. A missing predecessor seal blocks the successor's exit, never its entry. FR-025 was amended to say its strict ordering constrains exit only, and the Assumptions bullet on wave order was rewritten to match. This is consistent with the delivery contract §4, which now permits pipelined starts under four conditions already restated as FR-048 through FR-050. |
| CHK028 | **FR-058** added, bounding the FR-034 waiver narrowly. Verified against the manifests: there are **nine** `ADR046-delivery-001` through `ADR046-delivery-009` work items, **all** with `implementationState: Planned`, and the implementation graph assigns **all nine to wave W7** - not to W0 or W1. FR-058 therefore states that the waiver covers **only the absence of the W0/W1 seal artifacts** and waives no work item's own completion obligation; the nine delivery items sit entirely outside the waiver and must each reach `Merged` under W7's own seal. The Assumptions bullet now points at FR-058. |
| CHK039 | **FR-059** added, capturing the contended-file prep discipline from delivery contract §6.2/§7 and the repository's integrator-prep-first pattern: a file written by more than one parallel slice must be prepared by an integrator shared-prep commit BEFORE the claimant slices are dispatched, so each slice opens against a stable contract and no two slices concurrently edit the same file; integrator-only paths are never written by a slice at all; connected-component, launched-slice, and blocked-slice counts are recorded at wave entry and after every panel round. Binds immediately for W2, which has a single writer for `nixos-modules/assertions.nix`. |
| CHK032 | Closed by **FR-056**. The mid-program specification-amendment scenario, and specifically its effect on in-flight validation and panel evidence, is now a stated requirement rather than an unaddressed scenario class. |

**Verification commands run** (outputs are the basis for the count in CHK028; no number here
is estimated):

```
jq -r '.items[] | select(.workItemId | startswith("ADR046-delivery-")) | .workItemId + " " + .implementationState' docs/specs/ADR-046-work-items.json
ADR046-delivery-001 Planned ... ADR046-delivery-009 Planned      (9 rows, all Planned)

jq -r '.nodes[] | select(.kind=="work-item" and (.id | startswith("ADR046-delivery-"))) | .id + " " + .wave' docs/specs/ADR-046-implementation-graph.json
ADR046-delivery-001 W7 ... ADR046-delivery-009 W7                (9 rows, all W7)
```

### Gate 3 - deliberate deferrals, by owning wave (2026-07-29)

Every item still unchecked is listed here with the wave that owns it, so a **scheduled
obligation is never mistaken for a coverage gap**. Wave assignments are taken from
`docs/specs/ADR-046-implementation-graph.json` (authoritative per FR-046) by work-item
prefix. Where the owning wave cannot be determined confidently from the specification set,
the row reads **needs integrator** rather than guessing.

| Item | Subject | Owning wave | Basis |
| --- | --- | --- | --- |
| CHK004 | Three reset scopes - Full Zone, Provider, Guest | W7 | `ADR046-reset-*` items are W7 |
| CHK005 | The 11 feasibility work items | W7 | `ADR046-feasibility-*` spans W1 and W7; the outstanding ones are W7 |
| CHK006 | Streamline and friction-closure scope | W7 | `ADR046-streamline-*` items are W7 |
| CHK007 | Security-and-threat-model closing obligation | W7 | `ADR046-security-*` spans W6 and W7; the closing/re-validation obligation lands with cutover in W7 |
| CHK008 | Telemetry and audit retention | W5 | `ADR046-telem-*` and `ADR046-audit-*` items are W5 |
| CHK009 | Unsafe-local no-isolation posture rule | needs integrator | No work-item prefix maps cleanly to this posture rule; owner must be named before it can be scheduled |
| CHK010 | RETAIN-until-parity eventual deletion, distinct from DELETE-row retirement | W7 | Path retirement is governed by the cutover/streamline waves; `ADR046-reuse-*` (W5) supplies only the reuse decision, not the deletion |
| CHK011 | Checkpoint identity and rollback-command detail | W7 | `ADR046-reset-*` items are W7 |
| CHK012 | Incident-hold scope - Zone-wide versus per-Volume | W7 | `ADR046-reset-*` items are W7 |
| CHK017 | "Operator-facing capability" parity criterion | W7 | Parity is evaluated at cutover, so the definition must bind no later than W7 |
| CHK018 | Objective test for "desktop companion consuming public operator contracts" | needs integrator | FR-039 is locally added with no upstream ADR-046 owner; the release-blocking companion set is an operator decision |
| CHK019 | "Host recovery point" definition and attestation evidence | W7 | FR-043 is program-local (outside the work-item manifest per the recorded operator decision) but is exercised by the W7 destructive runs |
| CHK021 | "Reachable through the operator surface" for deliberately unwired foundations | needs integrator | Spans W0 foundation surfaces and W6 Provider wiring; no single owning wave is evident |
| CHK022 | Pass condition for "compatible version verified against the release candidate" | needs integrator | Same locally-added companion family as CHK018 |
| CHK024 | 2-second operator envelope versus component-level budgets | W5 | The component budgets are measured against the W5 storage engine and its watch consumer |
| CHK025 | Companion adaptation without a published preview artifact | W5 | Date-bound: must be resolved before W5 publishes the replacement contracts, the last moment companions can begin adapting |
| CHK026 | Removal-proof consistency - one per path, 3 of 16 supplied, non-empty proof fields | W7 | Removal proofs are consumed by the cutover and streamline waves |
| CHK030 | Requirements per distinct cutover phase | W7 | `ADR046-reset-*` items are W7 |
| CHK031 | A wave that repeatedly fails its panel or cannot reach unanimity | needs integrator | Partially mitigated by FR-051 through FR-053 (round-nine deferral of LOW/MEDIUM findings); the terminal non-convergence case remains a governance decision |
| CHK033 | Partial or stalled companion adaptation, distinct from the binary block | needs integrator | Same locally-added companion family as CHK018 |
| CHK034 | Terminal case where a hard target cannot be met even after redesign | W5 | The first hard footprint target is re-measured against the corrected W5 design; the escalation path must exist by then |
| CHK035 | Rollback or recovery of a wave already merged into the integration lineage | needs integrator | Delivery-contract governance; no work item owns it |
| CHK036 | Hermetic execution-budget and runtime-ledger obligations | needs integrator | Binds from W2 onward as test discipline but no work item owns the requirement text |
| CHK037 | Panel's pinned provider, model, and reasoning-effort constraints | W2 | The recorded operator decision pins the panel model and requires a spec amendment plus a code change before **any** wave can seal, so it binds at W2 |
| CHK038 | Host continuity during the implementation waves as a requirement | needs integrator | Currently an Assumptions bullet covering W2 through W6; promoting it is a program decision, not a wave deliverable |
| CHK041 | Every FR traces to at least one owning spec | needs integrator | Gate 1 established the mapping and surfaced four locally-added requirements; completing the trace is an integrator sweep |
| CHK043 | Detail-preservation checklist owner and gate point | needs integrator | Requires naming an owner |
| CHK044 | Companion-adaptation assumption validated or risk-flagged | W5 | Same date-bound trigger as CHK025 |
| CHK045 | Memory-deficit recovery assumption and its decision path | W5 | The corrected storage design is delivered and re-measured in W5 |
| CHK046 | Daily-driver risk acceptance - explicit accepter and fallback | W7 | The first destructive live run is the cutover in W7 |
| CHK047 | Cloud-backed Provider validation dependencies | **closed** | Answered by the operator: access is reached through entrablau sign-in from a dev-realm VM, not host-side credentials. The cloud tier is not a wave-exit lane, so it gates only the release gate. See below |

### Open question escalated to the operator (2026-07-29)

**CHK047 - external dependencies for cloud-backed Provider validation. Deliberately left
unticked; it cannot be resolved from the repository.**

- **What is needed**: confirmation that **Microsoft Azure** accounts, subscriptions, and
  access credentials exist, and that they are usable from the validation host, for the
  cloud-backed Provider dossiers in the specification set. The dossiers that name Azure
  services are `ADR-046-provider-runtime-azure-container-apps`,
  `ADR-046-provider-runtime-azure-virtual-machine`, `ADR-046-provider-transport-azure-relay`,
  `ADR-046-provider-credential-entra`, and
  `ADR-046-provider-credential-managed-identity`. The corresponding work-item prefixes
  (`ADR046-aca-*`, `ADR046-azure-*`, `ADR046-mi-*`, `ADR046-cred-*`) are all assigned to
  **W6** by the implementation graph.
- **Specifically required**: a subscription that can create Azure Container Apps and Azure
  Virtual Machine resources, an Azure Relay namespace, an Entra ID tenant that permits the
  device/machine join the credential dossier exercises, and a managed-identity assignment on
  the runtime resources. Each needs a named owner for the spend and for credential custody.
- **Why it is cheap now and expensive at W6**: obtaining a tenant, a subscription, and the
  necessary directory roles has an approval latency that is unrelated to engineering
  progress. Discovered now it runs concurrently with W2 through W5 at zero schedule cost.
  Discovered at W6 it stalls five Provider slices that are otherwise ready to launch, and a
  stalled ready slice is itself a recorded process failure under FR-028.
- **What it blocks**: the **release gate**, via the cloud-backed Provider validation that
  SC-022 requires. It does **not** block W2 entry, and it is not a Gate 2 item.

**Operator answer, recorded. CHK047 is closed.**

Access is not a host-side credential and is not obtained by provisioning a
subscription against this checklist. It is reached through the existing
identity path:

- Validation runs from the `dev-general` microVM on the operator's host, with
  the security key attached to that VM and the operator connected through it.
- That VM is a member of the **dev realm**, so the cloud-backed Providers are
  exercised from inside a realm rather than from the host.
- Microsoft sign-in is performed by **entrablau** on that VM, which yields
  access to Azure Relay, Azure Container Apps, and Azure Virtual Machines.

This satisfies the constraint the checklist could not verify from the
repository, and it does so in the shape ADR 0032 already requires: realm
credentials live inside the realm, never in `d2bd`, the broker, the host
bundle, or any host-side activation artifact. No new credential custody
question arises, because custody is the one the dev realm already has.

The Azure **resources** do not exist yet and are not being pre-provisioned.
They are created as part of the owning wave's plan, with operator input at
that point.

**Sequencing.** The mechanism depends on the host running the `v3` lineage,
which happens at the W7 cutover, so the cloud tier is expected to run after
that rather than during W6. That ordering is already what the delivery
contract requires: section 10.11 records the cloud row as a **manual tier,
never run in CI or as a required wave-exit lane, recorded as external
evidence only**. The cloud validation is therefore **not** a W6 exit
criterion and cannot stall a W6 slice. It feeds SC-022 and the release gate,
both of which are evaluated against the final candidate snapshot - which by
construction exists only after the cutover.

The residual obligation is on the release gate alone: SC-022 requires the
cloud tier to have executed at least once against the final candidate with
recorded external evidence. If the resources are not stood up before the
release gate is evaluated, the alternative is a recorded reduced-scope
validation or an explicit deferral of those five dossiers with a stated
effect on the gate, per FR-042's rule that a capability is never retired
silently.
