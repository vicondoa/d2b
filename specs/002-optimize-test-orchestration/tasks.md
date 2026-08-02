# Tasks: Optimize Test Orchestration

**Input**: Design documents from `/specs/002-optimize-test-orchestration/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`,
`contracts/local-validation-targets.md`, `quickstart.md`

**Tests**: Test tasks are included because the specification requires exact
coverage preservation, failure propagation, bounded concurrency, and measured
performance acceptance.

**Organization**: Tasks are grouped by user story so each target speedup can be
implemented and measured independently before the cross-cutting resource and
evidence phases.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel because it touches a different file and does not
  depend on an incomplete task.
- **[Story]**: Maps the task to a user story in `spec.md`.
- Every task names the exact file or artifact it changes.

## Phase 1: Setup

**Purpose**: Capture the immutable baseline environment and test contracts
before implementation changes begin.

- [ ] T001 Record the baseline commit, CPU, memory, Rust, Cargo, nextest, Nix, and Make versions in `.scratch/test-speedup-baseline/environment.txt`
- [ ] T002 Capture the complete Rust, Nix unit, and flake source inventories plus full command traces from actual public target runs; derive trace-cited baseline execution manifests in `.scratch/test-speedup-baseline/` as described by `specs/002-optimize-test-orchestration/quickstart.md`
- [ ] T003 Capture one priming run plus three warm-cache samples and three best-effort cold-cache samples for Rust, Nix unit, the direct flake target, and the legacy local Layer-1 flake shard path into `.scratch/test-speedup-baseline/test-rust.json`, `.scratch/test-speedup-baseline/test-nix-unit.json`, `.scratch/test-speedup-baseline/test-flake-direct.json`, and `.scratch/test-speedup-baseline/test-flake-layer1.json`
- [ ] T004 Capture Cargo timing output, active linker evidence, per-phase Rust durations, and duplicate feature/build observations in `.scratch/test-speedup-baseline/cargo-timings/` and `.scratch/test-speedup-baseline/rust-analysis.md`

---

## Phase 2: Foundational

**Purpose**: Lock the measured contract and resolve any drift between planning
artifacts and committed code before changing orchestration.

**CRITICAL**: User story implementation begins only after this phase records a
valid baseline against committed `v3` behavior.

- [ ] T005 Create `specs/002-optimize-test-orchestration/benchmark-results.md` with the baseline environment, sample medians, inventory digests, cache definitions, and invalidated-sample rules from `data-model.md`
- [ ] T006 Reconcile any baseline coverage or command-contract drift by updating `specs/002-optimize-test-orchestration/contracts/local-validation-targets.md` and `specs/002-optimize-test-orchestration/quickstart.md` to match committed passing code
- [ ] T007 Record the final Rust execution-leaf ownership, target-directory conflicts, broker serial chain, Nix installable sets, and realized flake check set in `specs/002-optimize-test-orchestration/benchmark-results.md`

**Checkpoint**: Baselines, inventories, and execution boundaries are recorded
before any target implementation changes.

**Panel Gate**: Run the ten-seat d2b plan panel. Do not begin Phase 3 until
every seat signs off with no recommendations.

---

## Phase 3: User Story 1 - Faster Rust Validation (Priority: P1) MVP

**Goal**: Make `make test-rust` a bounded GNU Make DAG that completes in at
most half the baseline warm-cache time while preserving every Rust surface.

**Independent Test**: Compare `make test-rust` against the baseline on the
representative host; require every baseline inventory item to remain present,
classify added tests, preserve grouped multi-failure output and serial broker
passes, and achieve a warm median no greater than 50% of baseline.

### Tests for User Story 1

- [ ] T008 [P] [US1] Add failing static contract assertions for the Make-owned Rust DAG, grouped keep-going execution, `D2B_RUST_BUDGET` validation, memory-aware default, execution-manifest output, and removal of the serial `all` scheduler in `tests/unit/meta/ci-runner-regression.py`
- [ ] T009 [P] [US1] Add failing policy assertions for retained doctests, discovered `harness = false` binaries with explicit empty-discovery failure, serial broker feature passes, same-target dependency edges, runtime frontier quota bounds, leaf-only driver modes, and unchanged excluded workspaces in `packages/xtask/tests/policy_workspace.rs`

### Implementation for User Story 1

- [ ] T010 [US1] Refactor `tests/test-rust.sh` into environment setup plus explicit leaf modes for API surface, main workspace, broker, guest shell runner, no-bash AST scan, schema reproducibility, supply-chain checks, and inventory/stub checks; remove the no-argument serial `all` scheduler
- [ ] T011 [US1] Implement the bounded recursive GNU Make Rust DAG, cgroup-aware `D2B_RUST_BUDGET`, runtime active-lane and quota calculation valid down to budget `1`, grouped output, keep-going behavior, recursive-Make jobserver propagation, and stable public shard targets in `Makefile`
- [ ] T012 [US1] Pass each Rust lane's runtime quota through Cargo `--jobs` and nextest test threads, dependency-order same-target operations, unset `MAKEFLAGS`, `MFLAGS`, and `MAKELEVEL` for quota-controlled Cargo/nextest children, and write completed leaves to `D2B_EXECUTION_MANIFEST` in `tests/test-rust.sh`
- [ ] T013 [US1] Route monolithic and background Rust callers through `make test-rust` instead of invoking the removed Bash aggregate in `tests/static.sh`
- [ ] T014 [US1] Update target-directory and cache synchronization assertions for the new Rust leaf layout in `tests/unit/gates/ci-rust-cache-sync.sh`; if Rust manifest wiring changes, update `tests/layer1-jobs.json` and regenerate `.github/workflows/pr-l1-static-fast.yml`
- [ ] T015 [US1] Commit the Rust implementation and contract-test changes with tag `( spec002w1 )` in `Makefile`, `tests/test-rust.sh`, `tests/static.sh`, `tests/unit/meta/ci-runner-regression.py`, `tests/unit/gates/ci-rust-cache-sync.sh`, and `packages/xtask/tests/policy_workspace.rs` before running Nix-backed validation
- [ ] T016 [US1] Run the Rust contract tests, all Rust leaf targets, full `make test-rust`, source-inventory and executed-manifest baseline-subset comparisons, and added-test classification; record results in `specs/002-optimize-test-orchestration/benchmark-results.md`
- [ ] T017 [US1] If the Rust warm median remains above 50% of baseline, run the mold contingency using `.scratch/test-speedup-rust-linker/`, require at least a 10% whole-target warm-median improvement with no supported-platform regression, record the decision in `specs/002-optimize-test-orchestration/benchmark-results.md`, and, when adopted, commit `packages/.cargo/config.toml` plus `flake.nix` with tag `( spec002w1 )` before any acceptance run
- [ ] T018 [US1] Capture the final three-sample Rust warm median and cold observation in `.scratch/test-speedup-optimized/test-rust.json` and mark the US1 acceptance result in `specs/002-optimize-test-orchestration/benchmark-results.md`

**Checkpoint**: Rust validation is independently complete, coverage-equivalent,
resource-bounded, and at least twice as fast when warm.

**Panel Gate**: Run the ten-seat d2b work panel on the integrated US1 diff.
Do not begin Phase 4 until every seat signs off with no recommendations.

---

## Phase 4: User Story 2 - Faster Nix Unit Validation (Priority: P1)

**Goal**: Experiment with established Nix/Lix runners and evaluators, then
implement the fastest measured design that preserves the full contract.

**Independent Test**: Compare `make test-nix-unit` against the baseline;
require every baseline case and pin to remain present, classify added tests,
preserve correct failure attribution and output scope, and achieve a warm
median no greater than 50% of baseline.

### Tests for User Story 2

- [ ] T019 [P] [US2] Add runner-neutral failing assertions for complete multi-failure reporting, execution-manifest output, retained `D2B_NIX_UNIT_CHECK`, explicit failure on empty discovery, bounded resource controls, actionable handling of any retired knob, and absence of any new repository-specific scheduler in `tests/unit/meta/ci-runner-regression.py`
- [ ] T020 [US2] Add failing Nix unit cases for shared-corpus missing-file, duplicate-name, shard-coverage, and pin-integrity behavior in `tests/unit/nix/cases/test-infrastructure.nix`

### Implementation for User Story 2

- [ ] T021 [US2] Add `tests/unit/nix/cases/test-infrastructure.nix`, regenerate its case-presence pins, commit the case and pins together with tag `( spec002w2 )`, then run the focused expected-failure probe against the committed test
- [ ] T022 [US2] Create isolated experiment branches or worktrees from one committed candidate base for the tuned current pool, pure-Nix aggregate, `lix-unit` flake adapter, bounded `nix-eval-jobs`, consolidated Lix flake check, and conditionally `nix-fast-build`; commit each candidate before evaluation
- [ ] T023 [US2] Run the common candidate protocol from `research.md` for every viable design, including repeated refinement when results expose bottlenecks; store commands, tool versions, warm/cold timings, CPU/RSS, failure attribution, output scope, and dependency cost under `.scratch/test-speedup-nix-candidates/`
- [ ] T024 [US2] Record the candidate comparison and selected design in `specs/002-optimize-test-orchestration/benchmark-results.md`; select only a design that meets the full contract and 50% warm target, or continue iterating without changing the public target
- [ ] T025 [US2] Implement the selected runner/evaluator and only its measured supporting changes in `flake.nix`, `tests/unit/nix/`, `tests/test-nix-unit.sh`, and `tests/unit/meta/ci-runner-regression.py`, retaining CI shard selection and fail-closed pin/shard integrity
- [ ] T026 [US2] Commit the selected Nix unit implementation with tag `( spec002w2 )` before regenerating pins or running Nix acceptance validation
- [ ] T027 [US2] Regenerate and commit Nix unit pins with tag `( spec002w2 )` only if the selected implementation changes case discovery after the infrastructure-case pins committed in T021
- [ ] T028 [US2] Run the new infrastructure cases, including simultaneous multi-shard failure and empty-discovery probes, pin check, CI regression test, `make test-nix-unit`, source-inventory and executed-manifest baseline-subset comparisons, and added-test classification; record results in `specs/002-optimize-test-orchestration/benchmark-results.md`
- [ ] T029 [US2] Capture the final three-sample Nix unit warm median and cold observation in `.scratch/test-speedup-optimized/test-nix-unit.json` and mark the US2 acceptance result in `specs/002-optimize-test-orchestration/benchmark-results.md`

**Checkpoint**: Nix unit validation is independently complete, coverage
equivalent, free of the Bash worker pool, and at least twice as fast when warm.

**Panel Gate**: Run the ten-seat d2b work panel on the integrated US2 diff.
Do not begin Phase 5 until every seat signs off with no recommendations.

---

## Phase 5: User Story 3 - Faster Flake Validation (Priority: P1)

**Goal**: Use one local native flake evaluator and realize only the committed
realized-check class while leaving CI sharding intact.

**Independent Test**: Compare the optimized local Layer-1 path against the
legacy shard baseline; require every baseline check to remain present, classify
added tests, retain video realization, avoid unrelated builds, preserve CI
selectors, and achieve a warm median no greater than 50% of baseline. Measure
the direct `make test-flake` path separately and allow no more than 20%
regression.

### Tests for User Story 3

- [ ] T030 [P] [US3] Add failing policy assertions for one local `nix flake check --no-build --keep-going`, retained CI shard/output selectors, execution-manifest output, and narrow realized-check execution in `packages/xtask/tests/policy_ci.rs`
- [ ] T031 [P] [US3] Replace local-shard scheduler expectations with failing assertions that the local manifest no longer sets `D2B_FLAKE_LOCAL_SHARDS`, retired local flake knobs fail with migration messages, and the realized video check remains enforced in `tests/unit/meta/ci-runner-regression.py`

### Implementation for User Story 3

- [ ] T032 [US3] Remove the local process-per-check scheduler and implement one native flake check followed by one multi-installable realized-check build, write evaluated and realized classes to `D2B_EXECUTION_MANIFEST`, reject retired local flake knobs with migration messages, and preserve CI modes plus segfault diagnostics in `tests/test-flake.sh`
- [ ] T033 [US3] Remove `D2B_FLAKE_LOCAL_SHARDS` from the local `test-flake` environment without changing CI matrix jobs or required contexts in `tests/layer1-jobs.json`
- [ ] T034 [US3] Regenerate `.github/workflows/pr-l1-static-fast.yml` from `tests/layer1-jobs.json` using the existing Layer-1 workflow generator
- [ ] T035 [US3] Update Make target documentation for the single-evaluator local contract and retained CI selectors in `Makefile`
- [ ] T036 [US3] Commit the flake implementation and contract tests with tag `( spec002w3 )` in `tests/test-flake.sh`, `tests/layer1-jobs.json`, `.github/workflows/pr-l1-static-fast.yml`, `Makefile`, `packages/xtask/tests/policy_ci.rs`, and `tests/unit/meta/ci-runner-regression.py` before evaluation
- [ ] T037 [US3] Run the policy tests, CI runner regression test, workflow drift check, optimized local Layer-1 flake path, direct `make test-flake`, realized-check proof, source-inventory and executed-manifest baseline-subset comparisons, and added-test classification; record results in `specs/002-optimize-test-orchestration/benchmark-results.md`
- [ ] T038 [US3] Capture the final three-sample Layer-1 and direct flake warm medians plus cold observation in `.scratch/test-speedup-optimized/test-flake-layer1.json` and `.scratch/test-speedup-optimized/test-flake-direct.json`, then mark the US3 acceptance and direct non-regression results in `specs/002-optimize-test-orchestration/benchmark-results.md`

**Checkpoint**: Flake validation is independently complete, coverage
equivalent, free of the local Bash shard scheduler, and at least twice as fast
when warm.

**Panel Gate**: Run the ten-seat d2b work panel on the integrated US3 diff.
Do not begin Phase 6 until every seat signs off with no recommendations.

---

## Phase 6: User Story 4 - Stable Resource Use (Priority: P2)

**Goal**: Prove that the combined design uses available capacity without
unbounded workers, memory exhaustion, or unstable run times.

**Independent Test**: Run all three targets repeatedly at default and
constrained budgets; require successful bounded execution, no host
out-of-memory event, and slowest-run variance within 20% of the median absent
recorded external contention.

### Tests for User Story 4

- [ ] T039 [P] [US4] Add failing assertions for positive `D2B_RUST_BUDGET` validation, cgroup-constrained memory behavior, grouped output, runtime frontier bounds for budgets including `1`, and actionable deprecation failures for obsolete local Nix worker knobs in `tests/unit/meta/ci-runner-regression.py`
- [ ] T040 [P] [US4] Add policy assertions that broker passes remain serial and same-target-directory Rust leaves cannot overlap in `packages/xtask/tests/policy_workspace.rs`

### Implementation for User Story 4

- [ ] T041 [US4] Tune the aggregate Rust lane count and runtime per-lane Cargo/nextest quotas from CPU plus the smaller of `MemAvailable` and remaining cgroup v2 allowance, preserve the `D2B_RUST_BUDGET` constrained-host override, and prove every active frontier stays within budget in `Makefile` and `tests/test-rust.sh`
- [ ] T042 [US4] Remove only scheduler variables made obsolete by the selected implementations from `tests/layer1-jobs.json` and `Makefile`; retain compatibility aliases or fail-fast migration checks in the drivers as recorded by the candidate decision, plus all CI selectors
- [ ] T043 [US4] Commit the final resource-budget and policy changes with tag `( spec002w4 )` in `Makefile`, `tests/test-rust.sh`, `tests/test-nix-unit.sh`, `tests/test-flake.sh`, `tests/layer1-jobs.json`, `tests/unit/meta/ci-runner-regression.py`, and `packages/xtask/tests/policy_workspace.rs` before resource validation
- [ ] T044 [US4] Run warm-cache default-budget and constrained-budget repetitions of all three targets, capture CPU, memory, load, exit status, and variance in `.scratch/test-speedup-optimized/resource-stability.json`, require the slowest valid warm run to remain within 20% of its median, and record conclusions in `specs/002-optimize-test-orchestration/benchmark-results.md`

**Checkpoint**: All targets remain stable under default and constrained
capacity without uncontrolled oversubscription.

**Panel Gate**: Run the ten-seat d2b work panel on the integrated US4 diff.
Do not begin Phase 7 until every seat signs off with no recommendations.

---

## Phase 7: User Story 5 - Actionable Performance Evidence (Priority: P3)

**Goal**: Deliver reproducible evidence showing the speedup, retained coverage,
removed duplicate work, cold-cache observations, and any justified exceptions.

**Independent Test**: A maintainer follows the committed quickstart, reproduces
the inventory comparison and three warm medians, and can account for every
removed, combined, or reused operation.

### Tests for User Story 5

- [ ] T045 [P] [US5] Add documentation-policy assertions for the three stable Make targets, benchmark method, warm hard gate, cold non-blocking result, and CI/local distinction in `packages/d2b-contract-tests/tests/policy_docs.rs`

### Implementation for User Story 5

- [ ] T046 [US5] Finalize before/after medians, sample variance, inventory digests, removed duplicate operations, realized outputs, resource observations, and acceptance verdicts in `specs/002-optimize-test-orchestration/benchmark-results.md`
- [ ] T047 [US5] Update the runnable benchmark, inventory, constrained-host, failure, and cold-cache instructions with the implemented command surface in `specs/002-optimize-test-orchestration/quickstart.md`
- [ ] T048 [US5] Document the optimized local target behavior, retained CI split, supported tuning knobs, and coverage caveats in `tests/README.md`, `tests/AGENTS.md`, and `docs/contributing/gates-and-lints.md`
- [ ] T049 [US5] Add the user-facing test-speed and orchestration changes without internal process markers to `changelog.d/test-orchestration-speed.md`
- [ ] T050 [US5] Commit the evidence, documentation, policy test, and changelog with tag `( spec002w5 )` in `specs/002-optimize-test-orchestration/benchmark-results.md`, `specs/002-optimize-test-orchestration/quickstart.md`, `tests/README.md`, `tests/AGENTS.md`, `docs/contributing/gates-and-lints.md`, `packages/d2b-contract-tests/tests/policy_docs.rs`, and `changelog.d/test-orchestration-speed.md`
- [ ] T051 [US5] Run `make test-fixture-contracts` plus the final targeted policy, drift, Rust, Nix unit, and flake validation commands from `specs/002-optimize-test-orchestration/quickstart.md`; record final evidence references in `specs/002-optimize-test-orchestration/benchmark-results.md`, update the durable deferred-findings register and friction log in `specs/002-optimize-test-orchestration/plan.md`, run the d2b memory workflow for any remaining open entries, and commit the final evidence and register updates with tag `( spec002w5 )`

**Checkpoint**: The speedup is reproducible, reviewable, and ready for the
d2b plan/work panel gate.

**Panel Gate**: Run the ten-seat d2b work panel on the integrated US5 diff.
The feature may advance to delivery only after unanimous sign-off.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on T001-T004 and blocks implementation.
- **US1 Rust (Phase 3)**: Depends on T005-T007.
- **US2 Nix unit (Phase 4)**: Depends on unanimous US1 work-panel sign-off.
- **US3 Flake (Phase 5)**: Depends on unanimous US2 work-panel sign-off and the
  shared `flake.nix` Nix unit graph.
- **US4 Resources (Phase 6)**: Depends on accepted US1-US3 implementations.
- **US5 Evidence (Phase 7)**: Depends on US1-US4 and closes the feature.

### User Story Dependency Graph

```text
Setup -> Foundation -> US1 -> US2 -> US3 -> US4 -> US5
```

- **US1** has no implementation dependency on US2 or US3.
- **US2** is independently testable but starts after the US1 panel gate.
- **US3** is independently testable but implementation-dependent on the shared
  Nix graph established by US2.
- **US4** measures and tunes all three completed target implementations.
- **US5** consolidates the accepted measurements and documentation.

### Within Each User Story

- Add contract or policy assertions first and confirm they fail for the
  intended missing behavior.
- Implement the smallest target-specific change that satisfies those tests.
- Commit all new or changed Nix-visible files before evaluation.
- Run the independent target and inventory comparison.
- Record the three-sample warm median before declaring the story complete.

### Parallel Opportunities

- T008 and T009 can run in parallel.
- T019 and T020 can run in parallel.
- T030 and T031 can run in parallel.
- T039 and T040 can run in parallel.
- Documentation policy work T045 can begin after the final command surface is
  stable while T044 resource sampling runs.

---

## Parallel Example: User Story 1

```text
Task T008: Add Rust DAG contract assertions in tests/unit/meta/ci-runner-regression.py
Task T009: Add Rust coverage policy assertions in packages/xtask/tests/policy_workspace.rs
```

## Parallel Example: User Story 2

```text
Task T019: Add single-invocation driver assertions in tests/unit/meta/ci-runner-regression.py
Task T020: Add corpus integrity cases in tests/unit/nix/cases/test-infrastructure.nix
```

## Parallel Example: User Story 3

```text
Task T030: Add flake contract assertions in packages/xtask/tests/policy_ci.rs
Task T031: Add local-manifest assertions in tests/unit/meta/ci-runner-regression.py
```

## Parallel Example: User Story 4

```text
Task T039: Add resource-budget regression assertions in tests/unit/meta/ci-runner-regression.py
Task T040: Add serial-boundary policy assertions in packages/xtask/tests/policy_workspace.rs
```

---

## Implementation Strategy

### MVP First

1. Complete Setup and Foundational tasks T001-T007.
2. Complete US1 tasks T008-T018.
3. Stop and validate `make test-rust` independently.
4. Treat the Rust speedup as the MVP only when every baseline inventory item
   remains present, added tests are classified, and the warm median meets the
   50% target.

### Incremental Delivery

1. Baseline and lock inventories.
2. Deliver the Rust Make DAG and prove US1 independently.
3. Deliver the shared Nix unit graph and batched invocation for US2.
4. Deliver the single-evaluator flake path for US3.
5. Tune resource behavior across all targets for US4.
6. Publish reproducible evidence and documentation for US5.

### Parallel Team Strategy

1. One owner completes Setup and Foundational work.
2. Implement and panel US1 before dispatching US2.
3. Implement and panel US2 before dispatching US3.
4. Run resource tuning and evidence work only on the converged implementation.

---

## Notes

- `[P]` tasks touch different files and have no incomplete dependency.
- No task adds a new top-level shell gate or new scheduler implementation.
- Use the current `tests/layer1-jobs.json` enforcement classification rather
  than assuming CI layout.
- Place all throwaway timing and inventory artifacts under `.scratch/`.
- Commit each story's Nix-visible paths before running its acceptance commands.
- Update the deferred-findings register and friction log after every panel.
- Follow the strict phase panel gates above; this plan does not use pipelined
  dispatch.
