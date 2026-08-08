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
- [ ] T003 Update roster and binding drift fixtures for a selection-table-driven pool in `scripts/copilot/check-bindings.mjs` and `scripts/copilot/test-check-bindings.mjs`, including the 35-file and sixteen-agent prompt-corpus shape with current `build` and without current `rust`, plus focused prompt-source contract checks that require ADR 0055 selected-roster discovery and scoped verification guidance and reject superseded fixed-roster, `relevant`/`signoff`/`recommendations`/`prior_resolutions`, held-reviewer, repeated-round, and old verification requirements unless explicitly marked withdrawn

**Checkpoint**: The repository can parse and mechanically validate the planned
thirteen-seat selection contract before lifecycle behavior changes.

---

## Phase 2: Foundational Lifecycle

**Purpose**: Build the shared deterministic lifecycle functions used by every
user story.

- [ ] T004 Implement candidate-bound selection schema version 1, deterministic rendering under `.scratch/panel/<lifecycle>/selections/<candidate-id>/<snapshot-sha256>.json`, create-or-compare writes, and monotonic lifecycle roster validation in `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs`
- [ ] T005 Add selection floor, optional trigger, ambiguity, build-positive, citation-negative, deterministic rendering, and candidate, selection-schema, selection-table-version, and roster mismatch cases to `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T006 Update `.github/skills/d2b-panel-round/SKILL.md` to define the one lifecycle-selection artifact shape, its generated address, the `--selection` handoff to both consumers, selected-roster dispatch, and the atomic compatibility cutover

**Checkpoint**: Selection and artifact generation primitives are deterministic
and fail closed, but no finished panel is considered migrated yet.

---

## Phase 3: User Story 1 - Batch Panel Findings (Priority: P1)

**Goal**: Run one comprehensive discovery and produce one complete stable issue
ledger.

**Independent Test**: Feed overlapping reviewer findings and orchestrator
deduplication groups into the lifecycle helper and verify stable `R` identifiers,
complete source mappings, retained attribution, and byte-identical regeneration.

- [ ] T007 [US1] Add discovery, deduplication, stable identifier, source completeness, conflicting regeneration, missing selected-seat result refusal, and explicit complete zero-finding seat acceptance cases to `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T008 [US1] Implement discovery request, explicit complete seat result, source finding, orchestrator grouping, and merged ledger generation in `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs`
- [ ] T009 [US1] Replace first-round delta-only instructions with full-candidate comprehensive discovery staging, record `lifecycle_id` in `address.json`, and update `.github/skills/d2b-panel-round/scripts/stage-diffs.sh` and `scripts/copilot/test-stage-diffs.mjs`

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

- [ ] T010 [US2] Add planted missing-ledger-response and incomplete required justification or evidence refusals; positive Fixed, verified Invalid and Withdrawn BLOCKER cases; refusal Intentionally rejected and Deferred BLOCKER cases; positive Fixed and verified Invalid and Withdrawn MAJOR cases without acceptance; positive accepted Intentionally rejected and Deferred MAJOR cases; refusal unverified Invalid and Withdrawn and unaccepted Intentionally rejected and Deferred MAJOR cases; for each unresolved Intentionally rejected and Deferred MAJOR disposition, planted structural acceptance negatives covering missing acceptance, null, array, and scalar values, each missing required field, and an extra field; type negatives covering a non-string value in each field; content and whitespace negatives covering empty and whitespace-only `accepter` and `justification` values and empty, whitespace-only, and otherwise out-of-enum `capacity` values; and self-verification, scope, monotonic roster, late issue, metric, and merge-readiness cases to `scripts/copilot/test-panel-lifecycle.mjs`
- [ ] T011 [US2] Implement exact response coverage, the unchanged Fixed, Intentionally rejected, Deferred, Withdrawn, and Invalid dispositions, disposition-specific justification and evidence validation, verified factual status, and plain maintainer or merge-owner acceptance only for unresolved Intentionally rejected or Deferred MAJOR responses; validate acceptance as a strict closed JSON object with exactly `accepter`, `capacity`, and `justification`, no extra fields, non-blank string `accepter` and `justification` values, and string `capacity` exactly `repository maintainer` or `merge owner`; add scope validation, per-seat verification artifacts, late issue admission, metrics, and approval checks in `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs`; shape-check acceptance without identity verification, signatures, GitHub API lookup, services, or authority
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
- [ ] T015 [US3] Add `.github/agents/panel-build.agent.md` and retire `.github/agents/panel-rust.agent.md` from current selection while preserving legacy Rust attribution and `software` Rust-profile responsibility
- [ ] T016 [US3] Make `.github/skills/d2b-panel-round/scripts/make-records.mjs` require and validate the same lifecycle-selection artifact consumed by xtask, emit current schema-version-2 `PanelRecord` objects with `panel_format_version: 1` for exactly its ordered roles, and update `scripts/copilot/test-make-records.mjs` for current variable rosters and candidate, selection-schema, selection-table-version, roster, panel-format-version, and mismatched-selection refusals

**Checkpoint**: Relevant experts are deterministic, legacy work is retained
automatically, and current records match the lifecycle roster.

---

## Phase 6: Delivery Integration and Contributor Contract

**Purpose**: Bind the selected roster into existing delivery validation and
update all contributor-facing process owners.

- [ ] T017 Add exactly two compact compatibility fixture bundles, `packages/xtask/src/delivery/testdata/panel-legacy-ten.json` and `packages/xtask/src/delivery/testdata/panel-current-variable.json`, and focused xtask tests for candidate, selection-schema, selection-table-version, ordered-roster, panel-format-version, mixed-family, and mismatched-selection refusal; pin one strict legacy ten-seat set containing `rust` and one strict current variable-roster set with an expanded-domain seat, covering request, records, attestation, and the seal panel object without a broader fixture or migration family
- [ ] T018 Implement strict `PanelSelectionV1` parsing and `panel-request --selection` in `packages/xtask/src/delivery/model.rs`, `packages/xtask/src/delivery/panel.rs`, `packages/xtask/src/delivery/command.rs`, and `packages/xtask/src/delivery/mod.rs`; validate candidate identity and ordered roster, populate request `roles` and `record_files`, write `panel_format_version: 1`, define the exact thirteen-seat current role domain and exact fixed-ten legacy role domain including `rust`, and dispatch by probing the discriminator from existing bounded JSON before deserializing strict current or legacy request and record DTOs without fallback
- [ ] T019 Make `panel-attest` validate exactly the roles and record files stored in the request, add missing, extra, out-of-order, unknown-version, mixed-family, and variable-roster tests, and update strict current and legacy attestation plus seal parsing in `packages/xtask/src/delivery/panel.rs`, `packages/xtask/src/delivery/seal.rs`, and `packages/xtask/src/delivery/evidence.rs`; current attestations and seal panel objects carry `panel_format_version: 1`, legacy fixed-ten objects omit it, the workspace schema remains 2, and no lifecycle, selection digest, service, authority, or broad migration machinery is added
- [ ] T020 Update lifecycle ownership and prompts in `.github/agents/d2b-integrator.agent.md`, `.github/skills/d2b-autopilot/SKILL.md`, and `.github/skills/d2b-wave-delivery/SKILL.md`
- [ ] T021 Update binding process text in `AGENTS.md`, `.specify/memory/constitution.md`, `docs/contributing/README.md`, `docs/contributing/panel-review.md`, `docs/contributing/copilot-agents.md`, and `docs/specs/ADR-046-validation-and-delivery.md`; run `cargo run -p xtask -- spec-registry` after changing the validation document and require it to update exactly `docs/specs/ADR-046-spec-set.json` and `docs/specs/ADR-046-work-items.json`; in `docs/adr/specs/0053-panel-prompt-sources.md`, add the build seat source guidance and ownership boundary, replace or explicitly withdraw every superseded fixed-roster, `relevant`/`signoff`/`recommendations`/`prior_resolutions`, held-reviewer, repeated-round, and old verification contract, and make ADR 0055 selected roster, complete discovery results, shared ledger and responses, and scoped verification operative
- [ ] T022 Update the exact 35-file and sixteen-agent shape in `scripts/copilot/prompt-corpus.mjs`, satisfy the corresponding `scripts/copilot/test-check-bindings.mjs` corpus and prompt-source stale-contract cases from T003, and recapture `scripts/copilot/prompt-corpus-manifest.json` only after prompts and contributor docs are final
- [ ] T023 Add the implementation release note in `changelog.d/adr055-panel-review.md`

**Checkpoint**: Standard Copilot tooling, delivery validation, and binding docs
describe and enforce one selected-roster Discover-Fix-Verify lifecycle.

---

## Phase 7: Validation and Review

**Purpose**: Prove the atomic cutover and close the Track B gate.

- [ ] T024 Run `make test-lint`, focused xtask test/clippy/fmt, `make check-tier0`, `make test-changelog`, and `make test-policy`, recording exact results in panel evidence
- [ ] T025 Expand the literal changed-path allowlist in `specs/004-adr0055-panel-review/plan.md`, compare it with `git diff --name-only "$(git merge-base origin/v3 HEAD)"...HEAD`, print every undeclared changed path, and fail if any path is outside the declared feature artifacts, contributor tooling, docs, xtask delivery files, tests, or changelog set
- [ ] T026 Run the finished-diff panel lifecycle with only HIGH or CRITICAL merge blockers admitted, resolve its complete ledger in batches, and obtain unanimous selected-roster verification
- [ ] T027 Push `spec004-panel-review-pragmatic`, open one Track B PR to `v3`, wait for required checks, and merge after the panel and CI are green

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

- T017, T018, and T019 stay serial because their fixture, parser, attestation,
  and seal tests share delivery files.
- Within T012 and T015, agent files can be edited in parallel if each lane owns
  distinct files.
- T020 and T021 can proceed in parallel after lifecycle behavior and field
  names are final and while the serial delivery work runs in its disjoint
  files. T022 waits for both and performs the one final corpus capture.

## Parallel Example: Delivery Integration

```text
Task: "Implement request-bound roster parsing and compatibility tests in packages/xtask/"
Task: "Update integrator and skill ownership in .github/agents/ and .github/skills/"
Task: "Update binding contributor process and panel prompt sources in AGENTS.md and docs/"
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
