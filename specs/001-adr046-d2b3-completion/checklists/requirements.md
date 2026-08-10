# Specification Quality Checklist: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-07-29
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] Technical detail is limited to binding architecture, security, delivery, and validation contracts
- [x] Focused on operator value, user-visible outcomes, and program completion
- [x] Written for the technical implementers and operators who must apply the contracts
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are buildable and name technical bindings where mechanical proof requires them
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified
- [x] Generated authority owns Wave 6 provider implementation, including the merged
  `ADR046-nl-001` through `ADR046-nl-020` set and manifest-backed `ADR046-ch-001`. The exact
  machine-readable T604/T479/T480 local contract owns cross-provider acceptance coordination
  and names exactly
  `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`; each effect
  and production `Ready` projection is bound to that same identity. Guest runtime-effect
  acceptance remains the distinct T479/T480 coordination over the generated Cloud Hypervisor
  implementation
- [x] The feature accurately records that the committed delivery specification is Version 1
  and no validation-and-delivery traceability pair exists; current delivery tooling and
  accepted member specifications remain authority, and no active requirement cites
  nonexistent later authority artifacts
- [x] Retired Wave 5 rows are read-only fenced history. Prospective provider-implementation
  ownership resolves from accepted member specs, current generated manifests, and the exact
  machine-readable T606-T609/T604/T479/T480 local contract without changing historical state
- [x] Prospective host-generation recovery is broker-coordinator-owned before first mutation, transfers
  durably from bootstrap broker to target broker, survives broker/daemon startup failures
  through existing units only, and never treats daemon identity or euid 0 as authorization
- [x] Historical W2-W5 records are read-only. ADR 0012 and committed code require
  `effectiveEastWest = Network.spec.isolation.allowEastWest &&
  d2b.site.allowUnsafeEastWest`; T608 and `ADR046-nl-001` through
  `ADR046-nl-005` establish the prospective foundation before the remaining Network rows and
  four production cases, while T604/T479/T480 own acceptance coordination
- [x] Generic Constitution 3.1.0 contains no ADR-046 detail; FR-036 and the exact feature-owned
  validator/tooling contract preserve the immutable W0-W5 history, authorize no recovery
  or reconstructed seal, and make T221 the next executable gate
- [x] T221 distinguishes retained W5 predecessor identities from newly emitted W6 snapshot
  identities and requires nonempty/no-ignore/no-skip focused guard evidence, drift, policy,
  unit, flake, nix-unit, runtime-ledger, and heavy-gate evidence before panel dispatch
- [x] The machine-derived launch census proves 258 manifest nodes, 29 manifest groups, seven
  local tasks, 265 post-entry records, first ready task T606 only, and no pre-T221 launch
- [x] T607-T609 bind real peer identity, broker-only host effects, strict generated
  ResourceType validation, TPM migration-before-ensure, Host-global authority,
  transactionally durable privileged audit, forbidden-identity redaction, bounded telemetry,
  and closed metric descriptors to mechanically checkable completion evidence
- [x] Existing accepted ADRs and committed passing code override drifted Provider prose, and
  each affected dossier lane must reconcile the implementation before completion
- [x] The feature-local exception is closed over exactly seven tasks, with explicit
  foundation/coordination labels, state transitions, completion evidence, W6-only adoption
  substitution, and no manifest identity or historical-state substitution
- [x] All 29 manifest groups require T606-T609, and all shared local/manifest writers plus
  thirteen scaffold transfers have candidate-manifest-bound handoff order
- [x] Dispatch/readiness comes from an external 36-group ledger; structured command evidence
  and the durably written zero-recommendation plan receipt are required process records and
  are explicitly not authentication
- [x] T221 invalidation stops at pre-first-dispatch material changes; later status-only
  ledger projections cannot change requirements, dependencies, owners, destinations,
  validation, readiness, census, or guards
- [x] The public T221 sequence is discovery-first: external ledger/evidence paths, empty
  `--entry-prepare true`, externally recorded strict JSON, entry-prepare import with eight
  repeated `--command-evidence` flags, then ordinary snapshot validation/write; no evidence
  record is required or fabricated before candidate discovery

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] Every implementation-specific detail is deliberate and traceable to a binding contract or fail-closed gate

## Validation Notes

**Iteration 1 (2026-07-29)** - 15 of 16 passing; two scope-class [NEEDS CLARIFICATION]
markers outstanding (terminal milestone, and the W0/W1 seal gap). Both surfaced to the user
rather than guessed, because either answer materially changes the size of the program.

**Iteration 2 (2026-07-29)** - all 16 items passing. Both clarifications answered and folded
in:

- **Terminal milestone**: full program, W2 through W8, six-condition release gate satisfied
  against the final candidate, d2b 3.0 tagged from `v3`. Landed as FR-037 and FR-038, a new
  SC-023, and an expanded delivery assumption noting that gating earlier would release a
  candidate a later wave still modifies.
- **W0/W1 seal gap (historical Iteration 2 disposition, superseded)**: Iteration 2 treated a
  feature-local written record as authority to begin sealed delivery at W2. The 2026-08-06
  analysis correction rejects that constitutional interpretation. FR-034 now makes the file
  historical evidence only. Generic Constitution 3.1.0 plus the exact feature-owned FR-036
  validator/tooling contract now preserve the accepted immutable history without requiring a
  historical seal.

Final shape: 38 functional requirements, 23 success criteria, 4 prioritized user stories,
9 key entities, 12 assumptions, explicit Out of Scope. No bracketed placeholder tokens
remain. Verified free of the non-ASCII dash codepoints the project constitution bans.

Ready for `/speckit-plan`.

**Iteration 3 (2026-07-29, post-clarify)** - all 16 items still passing after five
clarifications were integrated. No regressions.

Five decisions were recorded and applied:

- **Desktop companions block the release.** Previously out of scope entirely; now FR-039 and
  FR-040 require identifying the companion set, publishing replacement contracts early, and
  verifying each against the release candidate on a live host. Added SC-024, a fourth US4
  acceptance scenario, and rewrote the contradictory Out of Scope bullet so it scopes to
  companion source code rather than to companion compatibility.
- **Capability parity is enforced with exceptions.** SC-003 was absolute and would have
  contradicted the migration map's 15 DELETE rows. Now FR-041 enforces parity wherever a
  successor was promised, and FR-042 permits retirement only with an explicit listing,
  justification, and release-note entry.
- **Recovery-point attestation gates the irreversible cutover phase.** Added FR-043,
  SC-025, and a sixth US3 acceptance scenario. FR-043 now closes CHK019 with an exact
  full-host qualification, F7 candidate/commit/tree and daily-driver host binding, closed
  attestation fields, 86,400-second freshness and expiration, digest-bound evidence import,
  and fail-closed negative matrix. External snapshot/backup implementation remains outside
  the feature.
- **Live and hardware validation runs on the daily-driver host.** SC-022 now names the
  target, and a new assumption records this as deliberate risk acceptance that makes FR-043
  the primary safety net rather than a formality.
- **No intermediate releases; every wave lands by gated pull request.** Added FR-044 and
  FR-045 and SC-026. The former assumption about work landing through pull requests was
  promoted from an assumption to a requirement, since it is now enforced rather than
  presumed.

Three edge cases were added to cover the failure modes these requirements introduce: a
companion with no compatible version at release time, an operator who cannot attest to a
recovery point, and a capability discovered to have no successor only after its superseded
path is removed.

Shape at the end of Iteration 3: 45 functional requirements, 26 success criteria, 4
prioritized user stories, 15 edge cases, 9 key entities, 12 assumptions, and 5 recorded
clarifications. Those counts are an historical checkpoint and are superseded by Iteration 4.
At that checkpoint the artifacts were verified free of duplicate requirement ids,
placeholder tokens, banned dash codepoints, and statements contradicted by the
clarifications.

Ready for `/speckit-plan`.

**Iteration 4 (2026-08-06, current artifact reconciliation)** - all 22 current checklist rows
pass under the completion-program scope reflected by the current specification. The original
16-item baseline remains fully passing; 6 reconciliation checks were added after that baseline.
The current shape is **75 functional requirements and 35 buildable success criteria**. The
earlier 45/26 shape records the end of Iteration 3; it is not the current census.

Lifecycle state: **specification reconciled - T221 approval pending**. Passing these 22
current specification-quality checks does not record plan approval, implementation
completion, or permission to bypass the exact-base historical-predecessor guard and ordinary
selected-roster plan gate.

This is a technical completion and delivery contract, not a technology-agnostic greenfield
product brief. Exact APIs, paths, commands, protocol fields, timing bounds, candidate
bindings, and validation procedures are present where removing them would make an
architecture, security, or fail-closed delivery obligation ambiguous or untestable. The
content-quality checks above therefore reject accidental or gratuitous implementation detail,
while accepting the deliberate technical contract detail required by the current 75 FRs and
35 SCs. This reconciliation changes planning prose only, preserves every prior task ID, adds
T606-T609 for a current total of 609 task rows, and records no implementation completion.

## Notes

- Items marked incomplete require spec updates before `/speckit-clarify` or `/speckit-plan`
