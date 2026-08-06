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
- **W0/W1 seal gap**: recorded as delivered-without-seal under an explicit one-time written
  waiver; sealed delivery begins at W2. Landed as FR-034, FR-035, and FR-036, with SC-020
  amended so the every-wave-sealed criterion scopes to W2 through W8 and explicitly forbids
  any later wave relying on the waiver. Context section updated so the gap is stated up
  front rather than buried in a requirement.

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

**Iteration 4 (2026-08-06, current artifact reconciliation)** - all 16 items pass under the
completion-program scope reflected by the current specification. The current shape is **74
functional requirements and 34 buildable success criteria**. The earlier 45/26 shape records
the end of Iteration 3; it is not the current census.

This is a technical completion and delivery contract, not a technology-agnostic greenfield
product brief. Exact APIs, paths, commands, protocol fields, timing bounds, candidate
bindings, and validation procedures are present where removing them would make an
architecture, security, or fail-closed delivery obligation ambiguous or untestable. The
content-quality checks above therefore reject accidental or gratuitous implementation detail,
while accepting the deliberate technical contract detail required by the current 74 FRs and
34 SCs. This reconciliation changes planning prose only and records no implementation
completion.

## Notes

- Items marked incomplete require spec updates before `/speckit-clarify` or `/speckit-plan`
