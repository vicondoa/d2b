# Tasks: Pragmatic ADR 0055 Panel Review

**Input**: Design documents in `specs/004-adr0055-panel-review/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`,
`contracts/panel-artifacts.md`, and `quickstart.md`

**Delivery**: One atomic Track B wave, `spec004w1`. Tasks are grouped by user
story for traceability, but the workflow does not merge until all stories and
cross-cutting delivery changes are complete.

## Phase 1: Setup

**Purpose**: Establish the authoritative selection contract and test entrypoint.

- [ ] T001 Add version 2 seat classes, floors, fill order, focus, profiles, and triggers to `.github/skills/d2b-panel-round/selection-table.json`
- [ ] T002 Add panel lifecycle behavior test invocation to `tests/test-lint.sh` and create the initial harness in `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T003 Update roster and binding drift fixtures for a selection-table-driven pool in `scripts/copilot/check-bindings.mjs` and `scripts/copilot/test-check-bindings.mjs`

**Checkpoint**: The repository can parse and mechanically validate the planned
thirteen-seat selection contract before lifecycle behavior changes.

---

## Phase 2: Foundational Lifecycle

**Purpose**: Build the shared deterministic lifecycle functions used by every
user story.

- [ ] T004 Implement selection, deterministic rendering, create-or-compare writes, and lifecycle state validation in `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs`
- [ ] T005 Add selection floor, optional trigger, ambiguity, build-positive, citation-negative, and deterministic rendering cases to `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T006 Update `.github/skills/d2b-panel-round/SKILL.md` to define generated lifecycle addresses, selected-roster dispatch, and the atomic compatibility cutover

**Checkpoint**: Selection and artifact generation primitives are deterministic
and fail closed, but no finished panel is considered migrated yet.

---

## Phase 3: User Story 1 - Batch Panel Findings (Priority: P1)

**Goal**: Run one comprehensive discovery and produce one complete stable issue
ledger.

**Independent Test**: Feed overlapping reviewer findings and orchestrator
deduplication groups into the lifecycle helper and verify stable `R` identifiers,
complete source mappings, retained attribution, and byte-identical regeneration.

- [ ] T007 [US1] Add discovery, deduplication, stable identifier, source completeness, and conflicting regeneration cases to `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T008 [US1] Implement discovery request, source finding, orchestrator grouping, and merged ledger generation in `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs`
- [ ] T009 [US1] Replace first-round delta-only instructions with full-candidate comprehensive discovery staging in `.github/skills/d2b-panel-round/scripts/stage-diffs.sh` and update `scripts/copilot/test-stage-diffs.mjs`

**Checkpoint**: One discovery accounts for every selected reviewer finding in a
stable shared ledger.

---

## Phase 4: User Story 2 - Fix and Verify the Ledger (Priority: P1)

**Goal**: Record every implementation response and run scoped verification
without reopening discovery.

**Independent Test**: Generate responses and per-seat verification requests
from a complete ledger, then prove unsupported dispositions, unrelated fix
scope, roster narrowing, late MINOR/NIT blockers, and conflicting regeneration
are refused.

- [ ] T010 [US2] Add disposition, self-verification, scope, monotonic roster, late issue, metric, and merge-readiness cases to `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T011 [US2] Implement response templates, evidence capture, scope validation, per-seat verification artifacts, late issue admission, metrics, and approval checks in `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs`
- [ ] T012 [US2] Update retained panel agent prompts and add `.github/agents/panel-simplicity.agent.md`, `.github/agents/panel-reliability.agent.md`, and `.github/agents/panel-agentic.agent.md` with comprehensive discovery and scoped verification contracts

**Checkpoint**: Every issue has a visible disposition, and verification blocks
only unresolved or newly introduced merge-risk conditions.

---

## Phase 5: User Story 3 - Select Experts and Continue Legacy Work (Priority: P2)

**Goal**: Include relevant experts, including build, and automatically continue
complete or partial legacy rounds.

**Independent Test**: Import complete and partial ten-seat fixtures twice and
verify stable source identities, raw evidence preservation, severity mapping,
Rust responsibility transfer, triggered build inclusion, and monotonic roster
union.

- [ ] T013 [US3] Add complete and partial legacy import, repeated import, raw evidence, severity prefix, Rust attribution, and build-trigger cases to `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T014 [US3] Implement version-first legacy import, source identity, severity normalization, partial discovery continuation, Rust profile responsibility, and roster union in `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs`
- [ ] T015 [US3] Add `.github/agents/panel-build.agent.md`, retire `.github/agents/panel-rust.agent.md` from current selection, and update `scripts/copilot/prompt-corpus-manifest.json` for the current agent pool
- [ ] T016 [US3] Make `.github/skills/d2b-panel-round/scripts/make-records.mjs` consume the selected roster and update `scripts/copilot/test-make-records.mjs` for variable rosters and legacy compatibility

**Checkpoint**: Relevant experts are deterministic, legacy work is retained
automatically, and current records match the lifecycle roster.

---

## Phase 6: Delivery Integration and Contributor Contract

**Purpose**: Bind the selected roster into existing delivery validation and
update all contributor-facing process owners.

- [ ] T017 Add request-bound selected-roster, missing/extra record, variable-size attestation, legacy fixed-ten, and seal propagation tests in `packages/xtask/src/delivery/panel.rs`, `packages/xtask/src/delivery/seal.rs`, and `packages/xtask/src/delivery/evidence.rs`
- [ ] T018 Update current panel roles and request-bound roster validation in `packages/xtask/src/delivery/model.rs`, `packages/xtask/src/delivery/panel.rs`, `packages/xtask/src/delivery/seal.rs`, `packages/xtask/src/delivery/evidence.rs`, `packages/xtask/src/delivery/command.rs`, and `packages/xtask/src/delivery/mod.rs`
- [ ] T019 Update lifecycle ownership and prompts in `.github/agents/d2b-integrator.agent.md`, `.github/skills/d2b-autopilot/SKILL.md`, and `.github/skills/d2b-wave-delivery/SKILL.md`
- [ ] T020 Update binding process text in `AGENTS.md`, `.specify/memory/constitution.md`, `docs/contributing/README.md`, `docs/contributing/panel-review.md`, `docs/contributing/copilot-agents.md`, and `docs/specs/ADR-046-validation-and-delivery.md`
- [ ] T021 Add the implementation release note in `changelog.d/adr055-panel-review.md` and recapture governed prompt inputs with `scripts/copilot/prompt-corpus.mjs`

**Checkpoint**: Standard Copilot tooling, delivery validation, and binding docs
describe and enforce one selected-roster Discover-Fix-Verify lifecycle.

---

## Phase 7: Validation and Review

**Purpose**: Prove the atomic cutover and close the Track B gate.

- [ ] T022 Run `make test-lint`, focused xtask test/clippy/fmt, `make check-tier0`, `make test-changelog`, and `make test-policy`, recording exact results in panel evidence
- [ ] T023 Confirm the feature diff adds no runtime, broker, contract-crate, or workspace dependency surface using the path guard in `specs/004-adr0055-panel-review/plan.md`
- [ ] T024 Run the finished-diff panel lifecycle with only HIGH or CRITICAL merge blockers admitted, resolve its complete ledger in batches, and obtain unanimous selected-roster verification
- [ ] T025 Push `spec004-panel-review-pragmatic`, open one Track B PR to `v3`, wait for required checks, and merge after the panel and CI are green

---

## Dependencies and Execution Order

### Phase Dependencies

- Phase 1 has no dependencies.
- Phase 2 depends on Phase 1.
- User Story 1 depends on Phase 2.
- User Story 2 depends on User Story 1 because verification consumes its ledger.
- User Story 3 depends on Phase 2 and integrates with the lifecycle from User
  Stories 1 and 2.
- Delivery integration depends on all three user stories.
- Validation and review depend on the complete atomic implementation.

### Parallel Opportunities

- T017 can start after the selected-roster contract is stable while T019 and
  T020 update disjoint prompt and documentation files.
- Within T012, the four new or retained agent files can be edited in parallel
  if each lane owns distinct files.
- T019 and T020 can proceed in parallel after lifecycle behavior and field
  names are final.

## Parallel Example: Delivery Integration

```text
Task: "Implement request-bound roster tests and validation in packages/xtask/src/delivery/"
Task: "Update integrator and skill ownership in .github/agents/ and .github/skills/"
Task: "Update binding contributor process in AGENTS.md and docs/contributing/"
```

These tasks use disjoint file ownership but converge before prompt capture and
validation.

## Implementation Strategy

1. Build and test selection and deterministic artifact primitives.
2. Add discovery and the stable ledger.
3. Add responses, self-verification, and scoped verification.
4. Add legacy import and the build expert.
5. Integrate the selected roster with existing delivery validation and docs.
6. Commit the atomic implementation, run focused enforcing validation, and run
   one finished-diff Discover-Fix-Verify panel.

The minimum useful behavior is User Story 1, but it is not independently
mergeable because ADR 0055 requires atomic compatibility and delivery cutover.

## Format Validation

- All tasks use checkbox, sequential ID, story label where required, and exact
  file paths.
- `[P]` is omitted because the implementation plan serializes the cutover;
  parallel opportunities are described only where file ownership is disjoint.
