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

All 57 items are now resolved as requirements-quality questions. A checked item means its
requirement is present, explicitly delegated to a named wave owner, or deliberately retained
at a named external governance boundary. It does not mean the owning implementation,
validation, panel, seal, or merge has completed.

---

## Upstream Coverage Completeness

Does the feature spec capture the obligations that live in `docs/specs/`?

- [x] CHK001 Are requirements defined for all 19 standard ResourceTypes, or is per-type behavior explicitly delegated to the owning specs with a stated delegation boundary? [Coverage, Gap]
- [x] CHK002 Are requirements for `Quota`, `EmergencyPolicy`, and `Endpoint` present or explicitly delegated? None of these three terms appears anywhere in the spec. [Gap]
- [x] CHK003 Are requirements binding implementation to the 129 frozen decisions in the decision register documented? The register is never referenced in the spec. [Gap]
- [x] CHK004 Are requirements defined for the three distinct reset scopes the spec set names - Full Zone reset, Provider reset, and Guest reset - or only for host cutover? [Gap, Coverage, Spec §FR-020]
- [x] CHK005 Are requirements defined for the 11 feasibility work items, or does the spec address only the storage spike? [Gap, Spec §Context]
- [x] CHK006 Are requirements for the streamline and friction-closure scope defined, given the spec commits to delivering the terminal wave? [Gap, Spec §FR-037]
- [x] CHK007 Is the security-and-threat-model closing obligation represented as a requirement, such as the threat model being updated and re-validated at cutover? [Gap]
- [x] CHK008 Are telemetry and audit retention requirements specified, or only content redaction? [Gap, Spec §FR-018]
- [x] CHK009 Is the unsafe-local no-isolation posture rule captured - preserved in status, CLI, and audit, and prohibited as a telemetry label, span attribute, or log field? [Gap]
- [x] CHK010 Are requirements for RETAIN-until-parity paths' eventual deletion specified distinctly from DELETE-row retirement? [Coverage, Spec §FR-023]
- [x] CHK011 Are checkpoint identity and rollback-command requirements specified, beyond naming a rollback boundary? [Completeness, Spec §FR-022]
- [x] CHK012 Is the incident-hold requirement scoped - whether it applies Zone-wide during a cutover window or per-Volume? [Clarity, Spec §FR-021]
- [x] CHK013 Is Gate 0's standing re-evaluation obligation stated as a requirement rather than only as an assumption? [Completeness, Spec §Assumptions]

## Requirement Clarity and Measurability

Can these requirements be objectively assessed as written?

- [x] CHK014 Is "typical desktop-scale declaration" quantified with a resource count or shape? [Ambiguity, Spec §SC-002]
- [x] CHK015 Is "declared budget" identified by name and value rather than referenced abstractly? [Clarity, Spec §SC-012]
- [x] CHK016 Are the hard numeric targets that FR-030 governs enumerated in the spec itself? None of the ten targets appears in spec.md; they exist only in plan.md and spec-coverage.md. [Measurability, Spec §FR-030]
- [x] CHK017 Is "operator-facing capability" defined precisely enough to make the parity criterion checkable? [Clarity, Spec §SC-003]
- [x] CHK018 Is "desktop companion that consumes d2b's public operator contracts" defined by an objective test, so the release-blocking set cannot be argued? [Clarity, Spec §FR-039]
- [x] CHK019 Is "host recovery point" defined - what qualifies, and what evidence constitutes attestation? [Ambiguity, Spec §FR-043]
- [x] CHK020 Is "actionable next step" specified well enough to be assessed without reviewer judgment? [Measurability, Spec §FR-017, §SC-004]
- [x] CHK021 Is "reachable through the operator surface" defined well enough to decide when a foundation surface has stopped being deliberately unwired? [Clarity, Spec SC-021]
- [x] CHK022 Is "compatible version verified against the release candidate" defined with a pass condition? [Measurability, Spec §SC-024]

## Requirement Consistency and Conflicts

Do the requirements agree with each other and with the artifacts they depend on?

- [x] CHK023 Does fixing the total at 545 work items conflict with the plan's statement that the terminal wave's items are recorded later and are additional? [Conflict, Spec §SC-019, Plan §Wave sequencing]
- [x] CHK024 Is the relationship between the operator-perceived 2-second envelope and the tighter component-level budgets stated, or do they read as competing targets? [Consistency, Spec §SC-002]
- [x] CHK025 Is the tension between blocking release on external companions and forbidding a preview build they could adapt against resolved as a requirement, or only as plan-level mitigation? [Conflict, Spec §FR-039, §FR-045]
- [x] CHK026 Is the removal-proof obligation internally consistent - one required per path, the migration map supplying only 3 of 16, and every work item carrying a non-empty proof field? [Consistency, Spec §FR-023, Research §R4]
- [x] CHK027 Is the ordinary entry-evidence versus exit-evidence distinction explicit? The later FR-036 correction separately blocks both boundaries pending an external Principle VI amendment. [Consistency, Spec §FR-025, §FR-036]
- [x] CHK028 Is the FR-034 historical record's scope unambiguous regarding the nine delivery work items that remain Planned? [Ambiguity, Spec §FR-034, §FR-035]
- [x] CHK029 Are the wave-to-destination assignments in the spec set and the implementation graph reconciled, given the recorded drift where two crate paths are listed under a wave that owns no work item for them? [Conflict, Plan §Recorded drift]

## Scenario Coverage

Are requirements present for each scenario class, or explicitly excluded?

- [x] CHK030 Are requirements defined per distinct cutover phase, or only for the procedure as a whole? [Coverage, Spec §FR-020]
- [x] CHK031 Are requirements defined for a wave that repeatedly fails its panel or cannot reach unanimous sign-off? [Gap, Exception Flow]
- [x] CHK032 Are requirements defined for a specification amendment discovered mid-program, including its effect on in-flight validation evidence? [Coverage, Spec §Assumptions]
- [x] CHK033 Are requirements defined for partial or stalled companion adaptation, distinct from the binary release-block? [Gap, Spec §FR-039]
- [x] CHK034 Are requirements defined for the terminal case where a hard target cannot be met even after redesign? [Gap, Spec §FR-030]
- [x] CHK035 Are requirements defined for rollback or recovery of a wave already merged into the integration lineage? [Gap, Recovery Flow]

## Non-Functional Requirements Coverage

- [x] CHK036 Are hermetic execution-budget and runtime-ledger obligations represented, or does the spec cover only test-layer placement? [Gap, Spec §FR-032]
- [x] CHK037 Are the panel's pinned provider, model, and reasoning-effort constraints stated as requirements, or is only unanimity captured? [Completeness, Spec §FR-026]
- [x] CHK038 Is host continuity for the operator during the implementation waves stated as a requirement rather than only an assumption? [Completeness, Spec §Assumptions]
- [x] CHK039 Are requirements defined for the contended-file prep discipline, so shared files are not concurrently edited by parallel slices? [Gap, Plan §Contended files]

## Traceability

- [x] CHK040 Is a mapping documented between this spec's FR and SC identifiers and the ADR-046 work-item IDs they correspond to? [Traceability, Gap]
- [x] CHK041 Does every functional requirement trace to at least one owning spec in the 55-member set, so no requirement is locally invented? [Traceability]
- [x] CHK042 Is the delegation boundary stated explicitly - which obligations are restated here versus deliberately left in the spec set? [Traceability, Plan §Specification coverage]
- [x] CHK043 Does the detail-preservation checklist have a named owner and a defined gate point at which it must pass? [Completeness, Coverage §Detail-preservation checklist]

## Dependencies and Assumptions

- [x] CHK044 Is the assumption that companions can adapt without any published preview artifact validated or flagged as a risk with a mitigation? [Assumption, Spec §FR-045]
- [x] CHK045 Is the assumption that the named design corrections will recover the memory deficit flagged as unvalidated, with a decision path if they do not? [Assumption, Research §RK-1]
- [x] CHK046 Is the daily-driver validation risk acceptance recorded with an explicit accepter and a stated fallback? [Assumption, Spec §Assumptions]
- [x] CHK047 Are the external dependencies required for cloud-backed Provider validation identified, including whether the necessary accounts and access exist? [Dependency, Gap, Spec §SC-022]

## Wave 5 Production-Completion Remediation

- [x] CHK048 Is the first policy install and restart path specified without requiring a policy-authorized read before the first `PolicySet`, while preserving authenticated normal access, private-issuer compiler/API capability seals, and D106? [Consistency, Spec FR-067]
- [x] CHK049 Is the Wave 5 Provider readiness member the exact `Provider/system-core` registration and its two owned handler-health handles rather than all Wave 6 dossiers or a boolean? [Clarity, Spec FR-069]
- [x] CHK050 Is resumed work mechanically gated by pre-validator A/P0 analysis/panel, validator-and-fragment V/B with T603's unique changelog path, rerun B/P analysis/panel, T603's immutable repository-relative B/P authorization, validator-derived P-to-Q `/d2b-spec-edit` batch, exact child commit C, crash-safe finalize receipt, fresh exact-C/Q analysis and unanimous plan review before T589, and every T073-T218 obligation, while final evidence binds separate descendant F? [Traceability, Plan "Approved adr046w5 production-completion graph"]
- [x] CHK051 Does commit create immutable authoritative audit in the same transaction, keep export completion separate, and make export-pending behavior explicit about the protobuf-represented committed state, exact replay binding, fixed digests, retention health, restart replay, and no rollback claim? [Scenario, Spec FR-070]
- [x] CHK052 Does the Constitution Check record FR-036 only as a non-authorizing external
  scope escalation: W0/W1 and W2-W5 Principle VI gaps remain blocking, no feature-local
  exception, waiver, checklist state, receipt, or plan panel grants continuation, and only a
  separately accepted constitution amendment on the execution lineage can define later
  authorization? [Consistency, Plan "Constitution Check"]
- [x] CHK053 Do current Wave 5 panel, checkpoint, resume, and commit-tag instructions use qualified lowercase `adr046w5` while preserving labeled historical identifiers? [Consistency, Plan/Tasks wave addressing]
- [x] CHK054 Does the approved Constitution 2.2.0 correction bound T605 to its owned pre-consumer artifacts, assign T595/T599 downstream reconciliation and T220 generated-manifest/full-drift convergence, and preserve exact `Zone.status.handlers[]` semantics without claiming implementation? [Constitution, Spec/Plan "C1 correction and version impact"]
- [x] CHK055 Does `CommittedPendingAudit` preserve the layered `ResourceStatus` composite without claiming phase/code members on `ResourceUpdateStatus`, while assigning the additive protobuf field to every mutation response including delete and recording the ResourceService fingerprint impact? [Consistency, Spec FR-070]
- [x] CHK056 Does T604 prove automatic startup/declaration/removal ingestion and same-identity owned effect/Ready evidence for exactly `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm` in the canonical lowest feasible and host-integration destinations, with Guest runtime-effect acceptance preserved as a Wave 6 deferral, refusals separate, and no top-level gate or new unit? [Coverage, Tasks T604]
- [x] CHK057 Does T220 freeze F before evidence, do T600/T601 exclusively own the exact
  eight closed validation identifiers, does T602 reject every set/mapping mismatch, and does
  T219 refuse every new binding action while awaiting the external disposition
  of the consumed request? [Traceability, Tasks T600-T602/T219-T220]

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
| CHK023 | SC-019 no longer fixes the total at 545. It reads the manifest at release time and accounts for terminal-wave items recorded at W7 close. The Context preserves 531 as the initial W2-W7 scope and separately records the current 68 `Merged` / 477 `Planned` census. |
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

### Gate disposition

- **Gate 2 - closed as requirements quality**: CHK013, CHK027, CHK028, and CHK039 have the
  resolutions recorded below. CHK003 closed earlier through FR-047.
- **Gate 3 - closed as explicit delegation or retention**: every formerly deferred item now
  names a wave owner or a fail-closed external governance boundary in the table below. These
  checklist closures schedule or retain work; they do not check any implementation task or
  claim a validation result.
- **Date-bound regardless of gate**: CHK025 must be resolved before W5 publishes replacement
  contracts, since that is the last moment companions can begin adapting. CHK047 (cloud
  account access) is cheap now and expensive at W6. **Both are now closed**; CHK047 below,
  CHK025 with CHK044 in the W5 gate at the end of this file.

### Analysis remediation (2026-07-29)

`/speckit-analyze` found 16 issues across spec.md, plan.md, and tasks.md. Clear-cut fixes were
applied; two ambiguous decisions were escalated. Three further checklist items closed as a
side effect:

| Item | Resolution |
| --- | --- |
| CHK015 | SC-012 names the exact budget: whole-process resident memory at or below 24,576 KiB with no baseline subtraction. The corrected proof and production-fixture measurements passed at their recorded tips; T601 owns the current completed-publication-path measurement on F. |
| CHK016 | FR-030 now enumerates all ten hard targets in a table inside spec.md, rather than leaving them only in plan.md and spec-coverage.md. |
| CHK020 | FR-017 now defines "actionable next step" as naming at least one concrete operator action - a command, a configuration change, or a named artifact to inspect - and explicitly rejects a bare failure notice or generic retry. |

CHK014 (SC-002's "typical desktop-scale declaration") remains **open and escalated**: it needs
a number, which is a product decision rather than a defect with an obvious correction.

### Operator decisions (2026-07-29)

| Decision | Outcome |
| --- | --- |
| FR-043 governance | **Program-local**, outside the work-item manifest. A qualifying point is an externally verified full-host snapshot or backup bound to exact F7 candidate/commit/tree, preview, and daily-driver host with closed record fields, 86,400-second freshness, and explicit expiration. T580 imports one digest-bound record; T555/T556 refuse missing, duplicate, malformed, partial, failed, stale, expired, wrong-identity, or unresolvable evidence. No external backup implementation is claimed. |
| CHK014 - SC-002 scale | **Resolved**: a single-Zone declaration of 10 to 20 resources. |
| Panel model | **`gemini-3.1-pro-preview`**, run as 10 read-only subagent lanes. Requires a spec amendment plus a code change (T581-T584) before any wave can seal. |

### Gate 2 - closed before W2 entry (2026-07-29)

All four Gate 2 items are resolved by amendments to spec.md. One Gate 3 scenario item
(CHK032) closed as a side effect.

| Item | Resolution |
| --- | --- |
| CHK013 | **FR-056** added. Gate 0's re-evaluation obligation is now a numbered functional requirement rather than only an Assumptions bullet: any amendment to a member specification re-opens that spec's validation and panel evidence and re-triggers Gate 0 across the whole manifest, and Gate 0 must pass again before any wave depending on the amended spec may seal. Stated as a standing obligation for W2 through W8, not a one-time precondition. The Assumptions bullet now points at FR-056. |
| CHK027 | **FR-057** added, stating the entry-evidence versus exit-evidence distinction explicitly. Entry evidence is what a wave needs to START implementing; exit evidence is what a wave needs to be DELIVERED - sealed and merged. A missing predecessor seal blocks the successor's exit, never its entry. FR-025 was amended to say its strict ordering constrains exit only, and the Assumptions bullet on wave order was rewritten to match. This is consistent with the delivery contract §4, which now permits pipelined starts under four conditions already restated as FR-048 through FR-050. |
| CHK028 | **FR-058** bounds the FR-034 historical record narrowly. Verified against the manifests: there are **nine** `ADR046-delivery-001` through `ADR046-delivery-009` work items, **all** with `implementationState: Planned`, and the implementation graph assigns **all nine to wave W7** - not to W0 or W1. The record covers only the absent W0/W1 seal artifacts, waives no work-item completion obligation, and authorizes no phase action; the nine delivery items must each reach `Merged` under W7's own seal if FR-036's external amendment permits continuation. |
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

### Gate 3 - explicit delegations and retained governance boundaries (2026-07-29)

Every item initially deferred at this gate is listed here with its current disposition and
the wave or governance boundary that owns it, so a **scheduled obligation is never mistaken
for a coverage gap**.
Wave assignments are taken from
`docs/specs/ADR-046-implementation-graph.json` (authoritative per FR-046) by work-item
prefix. A row with no manifest work-item owner is either assigned to the named wave
integrator as program-local convergence work or explicitly retained at an external
governance boundary. Retained rows authorize no feature implementation; the named gate
continues to refuse until that external owner resolves them.

| Item | Subject | Owning wave | Basis |
| --- | --- | --- | --- |
| CHK004 | Three reset scopes - Full Zone, Provider, Guest | W7 | Delegated to W7's `ADR046-reset-*` owners |
| CHK005 | The 11 feasibility work items | W7 | Delegated to the outstanding W7 `ADR046-feasibility-*` owners; the W1 members remain historical |
| CHK006 | Streamline and friction-closure scope | W7 | Delegated to W7's `ADR046-streamline-*` owners |
| CHK007 | Security-and-threat-model closing obligation | W7 | Delegated to the W7 cutover close over the W6/W7 `ADR046-security-*` family |
| CHK008 | Telemetry and audit retention | W5 - **closed** | FR-070 fixes bounded post-export journal and segment retention, prune behavior, restart replay, and typed degraded health; T592/T600/T601/T602 own implementation and exact-candidate evidence without claiming a result here |
| CHK009 | Unsafe-local no-isolation posture rule | W5 integrator | Program-local convergence assignment: T136 owns the no-isolation Host resource, T598/T599 own audit/telemetry and CLI/reference propagation, and T220 checks the combined candidate; this row claims no implementation |
| CHK010 | RETAIN-until-parity eventual deletion, distinct from DELETE-row retirement | W7 | Delegated to W7 cutover/streamline retirement; `ADR046-reuse-*` in W5 supplies only the reuse decision |
| CHK011 | Checkpoint identity and rollback-command detail | W7 | Delegated to W7's `ADR046-reset-*` owners |
| CHK012 | Incident-hold scope - Zone-wide versus per-Volume | W7 | Delegated to W7's `ADR046-reset-*` owners |
| CHK017 | "Operator-facing capability" parity criterion | W7 | Delegated to W7 parity evaluation before cutover/release |
| CHK018 | Objective test for "desktop companion consuming public operator contracts" | W5 | **closed** - FR-064; see the companion-membership gate below |
| CHK019 | "Host recovery point" definition and attestation evidence | W7 - **closed** | FR-043 defines qualification, exact record fields, candidate/commit/tree and host binding, freshness/expiration, evidence import, and fail-closed refusal; T580/T555/T556 exercise it without claiming the external backup implementation |
| CHK021 | "Reachable through the operator surface" for deliberately unwired foundations | W5 - **closed** | FR-066-FR-072 define the complete production boundary; T603 binds amended-plan resume and T600-T602 bind exact-candidate evidence before T219 |
| CHK022 | Pass condition for "compatible version verified against the release candidate" | W5 | **closed** - FR-065; see the companion-membership gate below |
| CHK024 | 2-second operator envelope versus component-level budgets | W5 - **closed** | SC-002 now fixes one monotonic start/stop clock, includes activation ingestion through operator projection, and requires the outer 2,000 ms ceiling and every applicable FR-030 component p95 to pass independently |
| CHK025 | Companion adaptation without a published preview artifact | W5 | **closed** - see the W5 date-bound gate below |
| CHK026 | Removal-proof consistency - one per path, 3 of 16 supplied, non-empty proof fields | W7 | Delegated to W7 cutover/streamline removal owners; every removed path still owes its own proof |
| CHK030 | Requirements per distinct cutover phase | W7 | Delegated to W7's `ADR046-reset-*` owners |
| CHK031 | A wave that repeatedly fails its panel or cannot reach unanimity | External delivery governance - **retained** | FR-025 and the close tasks fail closed after a binding finding; terminal non-convergence before a request remains an external governance decision and schedules no feature-local implementation |
| CHK033 | Partial or stalled companion adaptation, distinct from the binary block | W5 | **closed** - FR-063; see the W5 date-bound gate below |
| CHK034 | Terminal case where a hard target cannot be met even after redesign | W5 integrator | T601 measures and T220 converges; FR-030 forbids weakening, so an unmet target keeps F unfrozen and is explicitly retained as a blocking result rather than relabelled success |
| CHK035 | Rollback or recovery of a wave already merged into the integration lineage | External delivery governance - **retained** | No feature task may rewrite merged history or invent a recovery action; the delivery-contract owner must authorize any correction and the affected boundary otherwise refuses |
| CHK036 | Hermetic execution-budget and runtime-ledger obligations | W2-W8 wave integrators | Program-local process assignment at each wave's validation boundary; accepted delivery tooling and D094 remain authoritative rather than being copied into a manifest work item |
| CHK037 | Panel's pinned provider, model, and reasoning-effort constraints | External delivery tooling for every wave | Explicitly delegated to the binding delivery policy and tooling; feature close tasks consume and revalidate that binding but do not redefine it |
| CHK038 | Host continuity during the implementation waves as a requirement | W2-W6 - **closed** | FR-075 and SC-035 promote continuity from assumption to exact-candidate close requirement; tasks map the existing no-skip VM survival attr into every W2-W6 freeze/close pair |
| CHK041 | Every FR traces to at least one owning spec | **closed** | `spec-coverage.md` maps every FR range to owning specs/work-item prefixes or explicitly labels the locally added requirement and its constraining contracts; FR-075 is included |
| CHK043 | Detail-preservation checklist owner and gate point | Feature-plan integrator at every wave entry | Run `spec-coverage.md`'s checklist before implementation dispatch; for resumed W5 it is part of the fresh analysis gate, and prospective wave entry owners repeat it |
| CHK044 | Companion-adaptation assumption validated or risk-flagged | W5 | **closed** - see the W5 date-bound gate below |
| CHK045 | Memory-deficit recovery assumption and its decision path | W5 - **closed** | SC-012 requires T601 to measure final candidate F at <=24,576 KiB with no baseline subtraction; FR-030 requires redesign and forbids durability/authz/audit weakening, sleeps, timeouts, or exclusions if it fails |
| CHK046 | Daily-driver risk acceptance - explicit accepter and fallback | W7 operator and cutover integrator | The operator is the accepter at the first destructive W7 run; FR-043's current recovery point and the cutover rollback boundary are the mandatory fallback |
| CHK047 | Cloud-backed Provider validation dependencies | **closed** | Answered by the operator: access is reached through entrablau sign-in from a dev-realm VM, not host-side credentials. The cloud tier is not a wave-exit lane, so it gates only the release gate. See below |

This reconciliation closes the requirements-quality status of CHK004-CHK007, CHK009-CHK012,
CHK017, CHK026, CHK030-CHK031, CHK034-CHK037, CHK043, and CHK046. It changes no task
checkbox and claims no implementation, validation, panel, seal, merge, or release result.
Rows marked **retained** deliberately have no feature implementation owner and continue to
fail closed at the stated external governance boundary.

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

### The W5 date-bound gate - closed (2026-08-03)

CHK025 and CHK044 are the only two items whose deadline was set by a publication event rather
than by a wave boundary: once W5 published the replacement contracts, the choice they name had
already been made implicitly. Both are now closed, and the order in which they were closed is
worth recording because it was wrong.

**What actually happened.** T577 and T578 published the inventory and the replacement contracts
at `b72b205f`. T579, which was supposed to resolve the FR-039 / FR-045 tension *before* those
contracts published, had not been done. The publication therefore encoded a resolution in
shipped prose - `docs/reference/zone-cli-contract.md` states "This is the intended resolution of
the FR-039 and FR-045 tension" - that no requirement in this program's spec yet said. That is
exactly the shape CHK025 was written to catch: a conflict answered by mitigation rather than by
a requirement.

| Item | Resolution |
| --- | --- |
| CHK025 | **FR-061** added. The conflict is resolved as a requirement by drawing a binding boundary between a *contract* (committed text, schema, or typed definition at a public git ref - publishing one is not a release) and an *artifact* (a tag, release, binary, substituter output, or version-pinned flake output - publishing one is). FR-039 and FR-045 are both retained unchanged; they simply govern different objects. FR-061 also fixes the publish/adapt/verify order, names the refusal at each stage, and closes the escape hatches: source inspection, a version match, and the publication of the contracts themselves are each explicitly not verification evidence. If adaptation stalls, exactly two outcomes are lawful - hold the release, or amend FR-045 through the amendment path - and FR-045 now points at FR-061 so the resolution is visible from both ends. |
| CHK044 | **FR-062** added. The assumption is **not** validated, and the requirement says so rather than papering over it: validating it needs evidence from repositories this program does not own, and none has been gathered. It is carried as a named risk with a mitigation (contracts point at the generated schema or typed definition rather than paraphrasing it, so a maintainer implements against the same bytes the implementation validates against), a detection point (the first live-host verification in W8, which is late, and the requirement says that too), and an escalation path (a failure there is recorded against FR-062 rather than absorbed into a wave fix round, because it falsifies a program premise rather than an implementation). |

**The no-preview constraint is preserved, not amended.** Nothing found while closing these two
items is evidence that FR-045 must be relaxed. What such evidence would look like is now stated
in `contracts/companion-contracts.md`: a specific companion, a specific surface, and a specific
reason the published contract is insufficient to implement against. Absent that, the constraint
stands, and FR-061 makes the amendment path the only lawful way to change it.

**No external repository was verified, and nothing here claims otherwise.** All five rows of the
published inventory read "Pending live-host verification". CO-1 and CO-2 are recorded as done
because the documents are committed and reachable; CO-3 and CO-4 are open, and they are the ones
that carry the compatibility claim.

**Still open in this family, and deliberately not closed here**: CHK018 (an objective test for
"desktop companion that consumes d2b's public operator contracts") and CHK022 (a pass condition
for "compatible version verified"). Both concern the *membership* and *entry bar* of the
inventory rather than what happens when a member falls short. CHK033, which asked what happens
when adaptation is partial, is closed below. **Both were closed in the following pass**; see
"CHK018 and CHK022 - membership and pass condition" at the end of this file.

### CHK033 - partial adaptation, decided (2026-08-03)

CHK033 asked for requirements covering partial or stalled adaptation as a scenario class,
distinct from the binary release-block. It was previously parked as "needs integrator" on the
grounds that it is a product decision. It is a product decision, and it is now made.

**The answer is no: a degraded required companion holds the release, exactly as an absent one
does.** SC-024 exists so that an operator's desktop is not degraded by adopting 3.0, and
nothing here carves an exception into it. There is no tolerance band and no per-surface partial
credit; a row with one Blocked surface is Blocked.

**What changed is the boundary, not the strictness.** Two different things were being called
degradation, and separating them is the whole content of the decision:

- A companion that reads a published capability key, finds it false, and declines the action is
  **conforming to the contract**. `runtime.operationCapabilities` is a committed manifest field
  emitted by `nixos-modules/lib.nix`, and `docs/reference/zone-cli-contract.md` already binds
  the shell client to check `runtime.operationCapabilities.guest.shell` before offering a shell
  action, with `PoolUnavailable` and `FeatureDisabled` as distinct required states. Classifying
  that as a defect would hold the release on a companion for obeying d2b and would make the
  capability surface pointless.
- **Degradation** is the other case: the surface is available and the companion cannot use it.
  That is what SC-024 names, and it blocks.

| Item | Resolution |
| --- | --- |
| CHK033 | **FR-063** added. Every named surface is classified at W8 as **Conformant** (works, or is unavailable through a published capability key or named refusal state, refused actionably, with no fallback), **Blocked** (anything else, including absent, crash, hang, silent wrong result, fallback to another transport or privilege path, unactionable refusal, undocumented workaround, or an outcome that cannot be classified), or **Retired** (a Blocked surface converted to an explicit FR-042 capability retirement decided before the tag). Conformant and Retired ship; Blocked holds. **Unclassified defaults to Blocked**, because an inconclusive exercise and a broken one are indistinguishable from the gate's position. A conformant refusal must name the false capability key and at least one concrete operator action per FR-017; a bare "not supported", a generic retry, a message naming only the companion, and a silently greyed control are each unactionable and therefore Blocked. Retirement is unavailable where FR-041 promised a successor, must carry a justification, an owner, the restoring condition, and a release-note line, and must never be a failed exercise relabelled afterwards. SC-024 was amended to define "verified" as exercised and classified, so it and FR-063 cannot be read against each other. |

**No shipped document changed, and that is a finding rather than a convenience.** The published
inventory's release-record requirement already reads "the result, including any capability
refusal or degraded behavior". The shipped page anticipated this classification; the program
spec was the side missing the rule. Existing docs are canon, so the rule was written to fit the
evidence shape that already ships.

**Recorded count drift.** Closing CHK033 moved this checklist to 20 of 47, one ahead of the
"19/47" then recorded in `plan.md`'s Project Structure listing. That count was corrected to the
current total in the pass that closed CHK018 and CHK022; the two items were not editable in the
CHK033 pass because its file ownership was the spec, checklist, and companion-contract set.

**No deprecation ladder was invented.** FR-045 leaves exactly one release, and this repository
deliberately retired its staged warning, fail-loud, and removal calendar at the clean break -
`docs/reference/default-switch-and-deprecation.md` is a historical landing page for that
reason. A retirement is therefore an enumerated, release-note-named fact and not the first step
of a timeline. The inventory row must not read as verified while a surface is retired, so the
gap stays visible instead of aging into silence.

**One rule deliberately not written**: a security carve-out. A missing security-key indicator
or a missing `unsafe-local` no-isolation posture reads to an operator as "no ceremony in
progress" and "isolated", so a silent absence already lands in the unactionable class and is
Blocked. The general rule reaches the strict answer on its own; a named exception would only
create an edge to argue about.

### CHK018 and CHK022 - membership and pass condition, closed (2026-08-03)

These are the last two items in the locally-added companion family. CHK018 asked for an
objective membership test so the release-blocking set cannot be argued; CHK022 asked for a pass
condition so "verified against the release candidate" is measurable. Both were parked as "needs
integrator" on the same reasoning that parked CHK033, and closing CHK033 removed that reasoning.

| Item | Resolution |
| --- | --- |
| CHK018 | **FR-064** added: a two-limb membership test, both limbs required. Limb 1 is discovery from a closed list - the validation host's own flake inputs, the currently published inventory, and any repository a d2b reference doc, example, template, or how-to names as consuming a d2b surface. Limb 2 is consumption of at least one surface from a closed list of public operator surfaces: the public daemon socket wire; the CLI contract including `--json` and exit codes, and its v3 replacement; the public `vms.json` manifest; `/etc/d2b/ui-colors.{json,css}`; the clipboard picker protocol over the inherited `socketpair()`; public launcher metadata served through the authorized public daemon API; and the flake's public outputs. Each row carries repository, pinned **commit** rather than a tag or version string, maintainer of record, discovery source, and consumed surfaces. An addition needs both limbs; a **removal needs a recorded negative determination** at a named revision and date. A candidate that satisfies limb 1 but whose consumption cannot be determined is **in the set and blocks** until that determination exists. |
| CHK022 | **FR-065** added: seven conditions, all required, no aggregate or majority reading. Live host, not a VM or container or CI runner; the exact release-candidate snapshot named by commit; the companion at a pinned commit; every surface in the row exercised rather than sampled; every surface Conformant or Retired under FR-063; zero Blocked including zero unclassifiable; evidence in FR-063's shape. Nine named non-passes are listed explicitly, including a green CI run in the companion's own repository and an exercise against a non-candidate build. **A moved candidate voids every verification against the previous snapshot**, mirroring the rule that any content change invalidates prior panel sign-off - without it, "the candidate" is whichever build was convenient when someone looked. |

**Two design choices worth defending, since both could have gone the other way.**

*Host flake inputs as the primary discovery source.* d2b targets a single trusted host with one
operator, so the set that adopting 3.0 can actually break is what that host runs. That is
enumerable and not arguable, where a curated prose list is neither.

*Uncertainty resolves into the set, not out of it.* Wrongly including a candidate costs one
negative determination. Wrongly excluding one ships a broken desktop and is discovered by the
operator rather than by the gate. The asymmetry only points one way.

**Prose was measured before it was rejected as a source**, because "README is unreliable" is the
kind of claim that should not be asserted. `AGENTS.md` names no companion at all - there is no
sibling-flake section, contrary to what `contracts/companion-contracts.md` previously stated.
`README.md` names them exactly once, at line 38, inside a sentence about colour output: it lists
three of the five published members under non-canonical short names (`wlcontrol`, `wlterm`,
`clip-picker`), adds two upstream projects that are not members (`niri`, `Waybar`), and omits
`d2b-toolkit` and `weezterm`. One line, three of five, two false positives, two omissions. That
is the evidence for a mechanical test, and the stale claim in the contract file was corrected in
the same pass.

**One negative rule that does real work.** Reading a private bundle artifact is **not**
membership; it is a defect to report. `docs/reference/manifest-bundle.md` fixes the
public/private boundary and every private artifact installs `root:d2bd` `0640`. Admitting such a
consumer to the inventory would record an unauthorised read as a supported contract and quietly
convert a security finding into a compatibility obligation.

**No external repository was verified, and nothing here implies otherwise.** FR-064 says how to
decide membership and FR-065 says what passing means; neither has been applied. No candidate has
been discovered from host flake inputs, no revision pinned, no negative determination recorded,
and no companion exercised. All five published rows remain Pending, and the five-row inventory
is itself an unverified starting set that FR-064 will confirm or change at W8.

**Companion family status: closed.** CHK018, CHK022, CHK025, CHK033, and CHK044 are all
resolved. The remaining open items in this checklist belong to other families.

### CHK021 - production reachability boundary, closed (2026-08-06)

The earlier artifacts treated an opened production store, an in-process watch, and readiness
fields as progress toward reachability but did not define the point at which the foundation
became production-reachable. The approved Wave 5 completion amendment closes that ambiguity.

FR-066 through FR-072 and contracts/resource-api.md now require one complete path:
registrar-consumed authenticated ComponentSession using T592's typed
`OpenPeerPidfdFromAcceptedSocket` broker operation and the approved broker `sys.rs` FFI
quarantine, authoritative subject, exact ZoneBus route,
matching installed policy revision after a one-shot private bootstrap read, registered
controller endpoint, admitted production watch, durable effect/adoption and audit recovery,
the exact `Provider/system-core` registration plus both required handler handles, and one
aggregate readiness projection. T604 adds the original operator boundary from Nix declaration
and emitted bundle through automatic startup/declaration/removal ingestion, owned effect and
readiness for exact `Volume/acceptance-state`, `Network/acceptance-net`, and
`Device/acceptance-tpm`, plus the pinned state-preserving Device cleanup while the acceptance
Volume/Network and unrelated resources remain ready, identity-stable, and unrecreated. Guest
runtime-effect acceptance is deferred specifically to Wave 6
`Provider/runtime-cloud-hypervisor` T384/T479/T480; Guest emission, status,
or refusal cannot satisfy the Wave 5 positive. This is a partial US1 production-plane
checkpoint, not story completion, and Network implementation remains owned by Wave 4.
Refusals remain separate negative cases.
T219 cannot accept
a fabricable or reusable bootstrap reader, numeric-PID-only identity, direct service call,
`ProductionWatchHarness`, fake endpoint, constructed subject, independent readiness bit,
status-only Provider substitute, disabled audit owner, missing immutable authoritative row,
incomplete export reported as success, a fictitious `ResourceUpdateStatus` phase/code shape,
manual-restart operator evidence, or stale/dirty candidate evidence.

SC-030 through SC-034, T603, T600-T602, and T604 make the resume and stopping conditions
mechanical and bind them to exact artifact/candidate identities. CHK021 is therefore closed
as a specification-quality item; no implementation or validation result is claimed by this
transition.

### Post-amendment remediation closure (2026-08-06)

CHK048 through CHK053 are closed by normative artifact text in this batch:

| Item | Artifact resolution |
| --- | --- |
| CHK048 | FR-067, the plan data flow, T589-T591, and the Resource API contract define private-issuer, compiler/API-sealed one-shot `PolicyBootstrapRead`, the bootstrap-to-authenticated transition, restart/failure behavior, and the D106 nonempty/poison guard. |
| CHK049 | FR-069/SC-033, plan/tasks, and contracts name `Provider/system-core`, its `d2b-core-controller` registration owner, exactly one `Zone.status.handlers[]` record named `system-core-host` and one named `system-core-user`, each with phase/timestamp from the live `HostReconciler` or `UserReconciler`; other Wave 6 dossiers are excluded. |
| CHK050 | T603 is unchecked, precedes T589, and is T589's sole direct prerequisite. Pre-validator A/P0 analysis/panel authorize only its validator paths plus unique fragment; validator-and-fragment V becomes B; analysis/panel rerun at B/P; only those post-validator receipts permit immutable repository-relative B/P authorization, exact T073-T218 accounting, validator-derived P-to-Q progress, and exact child C. Because C changes content, fresh analysis and unanimous plan review at exact C/Q are additionally required before T589. Final F evidence remains separate. |
| CHK051 | FR-070/SC-032 and contract/task acceptance require transactionally immutable authoritative rows, separate export completion, `CommittedPendingAudit` through the exact `ResourceStatus` layers and additive protobuf field including delete, exact replay binding, fixed digests, retention health, and one export per digest/ordinal. |
| CHK052 | The Constitution Check and Complexity Tracking record FR-036 as a non-authorizing external escalation only. The gaps remain blocking; this checked specification-quality item, feature prose, and current receipts grant no continuation. A separately accepted constitution amendment on the execution lineage must define any later authorization; FR-043 and constitution-2.0.0-authorized pipelining remain distinct program risks. |
| CHK053 | Current plan/task instructions use `adr046w5` and qualified template forms; preserved `ADR046-W5` occurrences are explicitly labeled legacy or historical. |

These are specification-quality transitions only. They do not mark implementation, T219,
T220, or T603 complete and do not convert historical evidence into current evidence.

### Latest analysis remediation status (2026-08-06)

| Item | Status |
| --- | --- |
| CHK054 / C1 | **Closed as specification quality only.** Constitution 2.2.0 authorizes the coordinated repair. Unchecked T605 owns `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`, kebab-case round-trip, exactly-one-each handler-list acceptance and rejection, `ProviderLifecycle` non-substitution, compiler-derived public/private API snapshots, paired reference, targeted contract coverage, and unchanged desired-Zone-schema proof. T605 completes without later consumers or the full drift gate; T595 emits, T599 reconciles consumers, and T220 reconciles generated manifests/full drift. No implementation, test result, or delivery is claimed. |
| CHK055 / C2 | **Closed as specification text only.** FR-070, SC-032, plan/tasks, and contracts now use the actual layered `ResourceStatus` composite and explicitly reject a fictitious `ResourceUpdateStatus` phase/code shape or schema change. |
| CHK056 / G1 | **Closed as task coverage only.** Unchecked T604 owns disjoint fixture-contract, Type-3 d2bd production-boundary, and `runNixOSTest` destinations, names the existing public validation targets including `make test-host-integration`, and feeds T600's sole `operator-nix-activation-cleanup` record, T602, and T219. No test result is claimed. |
| CHK057 / I1 and U1 | **Closed as process specification only.** T220 converges all repository content before F; T600/T601 have disjoint ownership of the exact eight validation identifiers; and T602 compares the exact lane/identifier multiset. Wave 5's retained request is already consumed, so T219 performs no binding action and remains blocked until an accepted external disposition expressly authorizes a non-request close action. T603 separately requires pre-validator A/P0 gates, validator-and-fragment V/B, post-validator B/P gates, the closed external receipt, deterministic snapshot, atomic lifecycle, and sole editor checkbox transition. The P-to-Q content change invalidates B/P sign-off for dispatch, so T589 additionally requires fresh exact-C/Q analysis and unanimous plan review; T602 retains the later stale checks. No receipt, authorization, or evidence result is claimed. |

CHK054 no longer blocks analysis. Run the pre-T603 read-only `/speckit-analyze`; if clean,
request the unanimous A/P0 plan panel. After validator-and-fragment V becomes B, rerun both gates at
B/P before T603 may create the receipt or authorize progress. After exact child C commits the
P-to-Q content change, rerun analysis and the unanimous plan review at exact clean C/Q before
T589 dispatch. Implementation remains pending.

### Committed-HEAD analysis receipt remediation (2026-08-06)

| Finding | Feature-artifact disposition |
| --- | --- |
| COV1 | Wave 5 positive operator acceptance covers exact `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm` as a partial US1 production-plane checkpoint. Device is the removal case. Network implementation remains owned by Wave 4. Guest runtime-effect acceptance is explicitly deferred to Wave 6 `Provider/runtime-cloud-hypervisor` T384/T479/T480; Guest emission, status, or refusal cannot satisfy FR-072, SC-034, T604, T602, or T219, and full US1 remains incomplete until exact-F6 positive Guest runtime-effect acceptance after W6. |
| UND1 | T592 owns typed `OpenPeerPidfdFromAcceptedSocket`, accepted-socket/pidfd `SCM_RIGHTS`, and the sole raw syscall wrapper in the approved broker `sys.rs` FFI quarantine with narrow allowances and per-block `SAFETY:` comments. A nonempty source policy plus three independent poison fixtures enforce exclusive quarantine, second-site rejection, and immediate `SAFETY:` comments through both policy and fixture-contract gates. T593 remains unsafe-forbid and consumes only the safe typed result. The `nix` wrapper, a new project FFI crate, raw/numeric-PID fallback, and a local session syscall are ineligible. |
| INC1 | Every source-writing completion owner T589-T599, T603-T605 has one exact, unique fragment path; T220 requires the closed fourteen-row owner/path set and alone folds it. T603 owns `changelog.d/delivery-resume-reconciliation.md` in addition to its exact two validator source paths, and evidence-only tasks write no repository file. |
| INC2 | Pipelined implementation-start provenance now cites Constitution 2.0.0 only. Constitution 2.1.0 remains attached only to bounded deferral and delivery memory. |

These dispositions amend planning contracts only. They check no task, claim no implementation
or validation result, and do not authorize T603 or any later source dispatch.

### Exact-HEAD analysis remediation (2026-08-06, `a30acf15cc36fa44122e78658ebf0b076c042e25`)

| Finding | Feature-artifact disposition |
| --- | --- |
| A1 | The Wave 5 acceptance set is no longer representative or selectable. `spec.md` pins Zone `acceptance`, `Volume/acceptance-state` with `Provider/volume-local`, `Network/acceptance-net` with `Provider/network-local`, and `Device/acceptance-tpm` with `Provider/device-tpm`; it fixes each Provider artifact/config, decisive resource fields, owned effect, readiness predicate, and the Device finalizer outcome that deletes the stopped swtpm/flush children, leaves the Endpoint unresolvable, and preserves the same TPM state Volume. Plan/tasks/evidence require those exact identities. Support resources cannot substitute, and Network implementation remains W4-owned. |
| A2 | SC-002 now uses one monotonic clock from durable target-generation transition-intent commit before publication/ingestion to the later of real-effect observation and production operator `Ready` projection. It includes automatic activation ingestion and requires the 2,000 ms outer ceiling plus every applicable FR-030 component p95 independently. Its separately versioned typed receipt is referenced by an unchanged schema-v2 `EvidenceRecord`; a failed operator record imports without a receipt but cannot close. CHK024 is closed. |
| U1 | FR-075 and SC-035 promote W2-W6 host continuity to a fail-closed exact-candidate close gate. Historical T028/T035/T070 inspect retained evidence only; T604 is the sole prospective owner of the existing host VM case and its Makefile discovery/build recipe. The no-skip evidence requires explicit Ready/Stopped states, same PID/start-time through a newly acquired pidfd, PID reuse/mismatch/ambiguity quarantine with no adoption/signal/cleanup, and full `d2b*`/`microvm*` namespace enumeration whose sorted unit-name set, after excluding only the canonical `d2b.slice`, equals exactly the three ADR-0015 units; every other unexpected lifecycle unit is rejected. A nonzero `systemctl list-units --all` result fails before filtering and cannot be masked by the later pipeline. Separate injected unexpected-slice and unexpected-service cases survive that sole exclusion and fail equality. Querying only the expected names is ineligible. The task preamble maps that result to every W2-W6 freeze/close pair without a new task ID or W5 evidence identifier. CHK038 is closed. |
| I1 | CHK008 is closed by FR-070's bounded journal/segment retention and prune/replay requirements; CHK041 is closed by complete owner-or-explicit-local traceability including FR-075; CHK045 is closed by SC-012's final-F measurement and FR-030's mandatory redesign path. No implementation or measurement result is claimed. |

### Current plan-panel recommendation disposition (2026-08-06)

| Class | Planning disposition |
| --- | --- |
| Host authorization and recovery | Initial public-socket `SO_PEERCRED`/group Admin classification is consumed into one sealed nonfabricable capability. The accepted external floor atomically owns the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census and pre-transfer coordinator ownership. Missing, stale, and cross-disposition members plus bare protocol 4 refuse. T592 consumes the source set read-only and owns only target-v5 adoption, target artifacts, and post-transfer recovery; target broker/daemon startup and bootstrap crash boundaries run without daemon recovery ownership or a new unit. |
| SC-002 compatibility | The global `EvidenceRecord` schema and v2 decoder remain unchanged. A passed operator record references one typed version-1 receipt; a failed record imports without one but cannot satisfy panel, seal, eligibility, or close. Import, cleanup, and the retention guard share one exclusive candidate OFD lock. Ordinary cleanup empties both ephemeral namespaces. Every incident state remains inspectable by stable id, closed cause/remediation, human/JSON projection, and exits. A verified expected payload reaches `parked`; an authenticated no-unlink exact residue census reaches `mismatch-retained` outside ephemeral/staging namespaces. A rename/reopen race remains recovery-pending with all names preserved and no false terminal status. Neither path unlinks; all incident states block parked-candidate publication and close while the external disposition plus fresh-successor path remains available. |
| Delivery-state preservation | T589 seeds the retained consumed request, runs unanimous and finding-plus-rerun nonbinding phase sequences, and proves byte-identical delivery state with no reservation or request mutation. T219 remains non-authorizing historical adjudication and names only the external-disposition remediation. |
| Network escalation | The untouched external Network spec remains sole-opt-in canon. W4 adjudication, T070, T071, and T220 require an external versioned correction/migration and all four Network/Host cases; no feature status can unblock them. |
| Host continuity and pidfd | T604 solely owns the existing daemon-restart VM case and Makefile discovery/build recipe. Empty discovery and every skip are fatal; Ready/Stopped, exactly three lifecycle units after excluding only canonical `d2b.slice`, unexpected-slice and unexpected-service poison cases, fresh-pidfd adoption, and reuse/mismatch/ambiguity negatives are mandatory. The pidfd quarantine and immediate `SAFETY:` comments are policy/fixture-enforced. |
| Operator procedure and T603 scope | Bootstrap, stable-reference, and rollback procedures validate grammar, bounds, identity, and existence before authorization or `sudo`; failure prevents the privileged command. The caller-flake target executable remains unprivileged; only the separately pinned installed apply object runs under `sudo`. Its connection-scoped peer pidfd and live executable identity must match the pin before every mutation. The independent closed registry has 15 edge ids, six pre-first cases, and exactly 84 literal post-first cases over the fourteen later edges; each refuses before the selected edge and persists no pidfd. Raw PID/start/store/NAR identity stays absent from every output surface; fixed correlation digests only, with no identity metric label. T603 owns exactly two Rust source paths plus `changelog.d/delivery-resume-reconciliation.md`, and T220 alone folds that exact fragment. |

FR-036 remains open. Its separate accepted Constitution Principle VI amendment and ancestor
check remain prerequisites for every implementation, resume, fix, work-panel, seal, merge, or
advance action. This planning-only batch supplies no authorization, does not change T072 or any
other checkbox, and cannot substitute for that external amendment or for W2-W4 historical
panel/seal evidence.
