# Specification Quality Checklist: Implement ADR 0052 Bazel Rust Gate

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-08-02
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

- Validation passed on the second review iteration after low-level cleanup and
  timeout mechanics were restated as observable safety outcomes.
- Bazel, Cargo, Make, and the stable Rust gate names appear because ADR 0052
  makes them binding scope and compatibility constraints. Low-level design
  mechanics remain authoritative in the ADR rather than being re-specified
  here.
- Revalidated after the round-one plan panel. FR-004 gained the single
  supported lock-regeneration path, FR-020 and FR-021 made the yanked-state
  carrier unconditional, and FR-052 gained the injected filesystem and clock
  boundary requirement. Three edge cases and three acceptance scenarios were
  added for the same three changes, and SC-011 gained the committed snapshot
  and its offline drift check so the unconditional rule is measurable rather
  than asserted.
- Revalidated after the round-three plan panel. FR-004 gained the separate
  no-argument module-lock refresh command and the fail-closed
  direct-dependency requirement; FR-021 gained the offline validator that the
  gate carriers and a contributor shell both run. Two acceptance scenarios and
  four edge cases were added for module lock drift, the upstream diagnostic
  that names an unsafe invocation, the warn-only direct-dependency check, and
  an unvalidated snapshot refresh. No functional requirement or success
  criterion was added or removed; the set remains FR-001 through FR-055 and
  SC-001 through SC-015. The three substrate claims behind these changes were
  measured against Bazel 8.6.0 rather than taken from documentation, and are
  recorded with their sources in `research.md`.
- Revalidated after the round-four plan panel. FR-021 gained the injectable
  index boundary for the networked snapshot refresh and the requirement that
  the offline validator cannot reach it; FR-052 extended the injected-boundary
  rule from filesystem and clock to networked registry-index responses and
  named the one contributor-run measurement that covers the real client. Two
  acceptance scenarios and three edge cases were added, for the two distinct
  ambient-control refusals, the index boundary, and the two ways an
  index-dependent design goes wrong. No functional requirement or success
  criterion was added or removed; the set remains FR-001 through FR-055 and
  SC-001 through SC-015.
- Revalidated after the round-seven test-seat finding. The wave-note
  command-shape rule moved from a hand-run shell scan plus a planted untracked
  note in W0, W1, and W2 validation to a type-5 policy lint in
  `packages/d2b-contract-tests/tests/policy_docs.rs`, carried by
  `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, with its planted
  cases held in-test rather than on disk. Two W0 tasks were added for the lint
  and one W0 task was merged into the generator implementation it was a clause
  of, so the artifact holds 174 tasks. The change is a carrier change, not a
  requirement change: FR-051, FR-052, and FR-053 already required the guard,
  the planted negative, and the use of an existing surface with no new
  top-level gate. No functional requirement or success criterion was added or
  removed; the set remains FR-001 through FR-055 and SC-001 through SC-015. The
  carrier claim was measured rather than assumed: `tests/test-rust.sh` excludes
  `d2b-contract-tests` from every workspace leaf, and `tests/test-policy.sh`
  runs seven contract-test binaries that do not include `policy_docs`.
- Revalidated after the round-eight panel. Three carrier and structure changes,
  no requirement change. First, a neutral internal crate
  `packages/d2b-bazel-support/` now holds the `FileSystem` boundary, the new
  `RunfilesView` boundary, and the one absolute startup-option construction, so
  the dependency direction is `xtask`, `d2b-bazel-runner`, and
  `d2b-test-locator` all depending on support and `xtask` never depending on
  the runner; `Clock` and `UptimeSource` stay in
  `packages/d2b-bazel-runner/src/clock.rs`, because no second crate reads them.
  The rule is enforced by extending the existing resolver-backed gate
  `tests/unit/meta/w0-dep-direction.sh`, which adds no new top-level shell gate
  and therefore stays inside FR-053. Second, the locator and topology provider
  negatives, absent, non-executable, stale, and wrong identity, are now
  supplied through the injected filesystem and runfiles fakes, and no test
  writes a stale executable into the live Cargo path; that is FR-052's existing
  injected-boundary rule applied to the one guard that had still been proven by
  arranging host state. Third, the wave-note lint's refusal now names the note,
  the one-based line, and the remediation and never the offending token, and
  its entry API returns `std::io::Result` at both levels instead of collapsing
  a failed read into `Option`; both are FR-029 and FR-052 properties of a
  refusal, not new requirements. Task count stays 174 and no task was renumbered.
  No functional requirement or success criterion was added or removed; the set
  remains FR-001 through FR-055 and SC-001 through SC-015.
