# Feature Specification: Optimize Test Orchestration

**Feature Branch**: `test-speedup`

**Created**: 2026-08-01

**Status**: Draft

**Input**: User description: "Make the Rust, Nix unit, and flake test targets run substantially faster by improving orchestration, increasing safe resource utilization, avoiding duplicate work, and preserving coverage without building unnecessary artifacts. Warm-cache execution must take at most half the current time. Cold-cache execution should be minimized but does not block completion. Local orchestration may be consolidated even if CI remains split. Prefer established external approaches over Bash or custom orchestration code."

## Clarifications

### Session 2026-08-01

- Q: Which cache conditions must meet the 50% elapsed-time reduction target? → A: Warm-cache runs must meet the 50% target; cold-cache time must be minimized and reported but does not block completion.
- Q: What additional scope and orchestration constraints apply? → A: Include the flake test, allow CI to remain split, and avoid Bash or custom orchestration unless no suitable established external approach exists.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Faster Rust Validation (Priority: P1)

As a d2b contributor, I can run the complete Rust validation target through one coordinated execution so that independent test work overlaps safely, shared preparation is reused, and the command finishes substantially faster without losing coverage.

**Why this priority**: Rust validation is a frequent development gate. Serial crate or group execution leaves available compute idle and lengthens every feedback cycle.

**Independent Test**: Run the existing Rust validation target on the representative development host and compare its elapsed time, completed test inventory, and result against the accepted baseline.

**Acceptance Scenarios**:

1. **Given** the current Rust validation baseline, **When** a contributor runs the optimized target under equivalent conditions, **Then** it completes in no more than 50% of the baseline elapsed time.
2. **Given** the optimized Rust target, **When** all tests pass, **Then** every test and companion check required by the current target has run exactly once unless an explicitly documented shared prerequisite is intentionally reused.
3. **Given** a failing Rust test, **When** the optimized target runs, **Then** the target fails and identifies the failing test surface clearly.

---

### User Story 2 - Faster Nix Unit Validation (Priority: P1)

As a d2b contributor, I can run the complete Nix unit validation target through one coordinated execution so that evaluation and build work is shared where possible, unrelated cases overlap safely, and unnecessary whole-project builds are avoided.

**Why this priority**: Nix unit coverage is another frequent gate whose current orchestration can repeat preparation and leave compute resources unused.

**Independent Test**: Run the existing Nix unit target on the representative development host and compare its elapsed time, case inventory, realized artifacts, and result against the accepted baseline.

**Acceptance Scenarios**:

1. **Given** the current Nix unit validation baseline, **When** a contributor runs the optimized target under equivalent conditions, **Then** it completes in no more than 50% of the baseline elapsed time.
2. **Given** the optimized Nix unit target, **When** all cases pass, **Then** it preserves the current enforcing case inventory and does not realize unrelated project outputs solely to obtain unit coverage.
3. **Given** a failing Nix unit case, **When** the optimized target runs, **Then** the target fails and identifies the failing case clearly.

---

### User Story 3 - Faster Flake Validation (Priority: P1)

As a d2b contributor, I can run the complete flake validation target through coordinated evaluation that reuses common work, overlaps independent checks safely, and avoids evaluating or realizing outputs unrelated to its coverage contract.

**Why this priority**: Flake validation is part of the frequent local feedback loop and is subject to the same avoidable evaluation, scheduling, and resource-utilization costs as the other target surfaces.

**Independent Test**: Compare the optimized local Layer-1 flake path with the
legacy local-shard baseline on the representative development host. Measure
the direct `make test-flake` path separately for non-regression.

**Acceptance Scenarios**:

1. **Given** the legacy local Layer-1 shard baseline, **When** a contributor runs the optimized local Layer-1 path under equivalent warm-cache conditions, **Then** it completes in no more than 50% of the baseline elapsed time and the direct `make test-flake` path regresses by no more than 20%.
2. **Given** the optimized flake target, **When** all checks pass, **Then** it preserves the current enforcing check inventory and avoids unrelated project outputs.
3. **Given** a failing flake check, **When** the optimized target runs, **Then** the target fails and identifies the failing check clearly.

---

### User Story 4 - Stable Resource Use (Priority: P2)

As a contributor sharing a development machine with other work, I can run any optimized target without uncontrolled oversubscription, memory exhaustion, or resource contention that makes elapsed time unstable.

**Why this priority**: Higher concurrency only provides value when it uses available resources predictably rather than causing thrashing or shifting failures into the host environment.

**Independent Test**: Repeat all three targets while observing elapsed time, peak memory pressure, load, and failure behavior on the representative host.

**Acceptance Scenarios**:

1. **Given** the representative host's available compute and memory, **When** an optimized target runs, **Then** it keeps independent work progressing concurrently without sustained resource exhaustion or repeated worker starvation.
2. **Given** a host with less available capacity, **When** an optimized target runs, **Then** concurrency remains bounded and the target completes correctly rather than relying on a fixed high worker count.
3. **Given** three equivalent benchmark runs, **When** their elapsed times are compared, **Then** the slowest run is no more than 20% slower than the median unless external contention is recorded.

---

### User Story 5 - Actionable Performance Evidence (Priority: P3)

As a maintainer, I can review reproducible before-and-after evidence showing where time was removed, what work was deduplicated, and that the faster targets retain their required coverage.

**Why this priority**: A speedup is not durable if maintainers cannot verify the measurement or determine whether it came from skipped validation.

**Independent Test**: Follow the documented measurement procedure and reproduce the baseline, optimized timing, coverage comparison, and work-inventory comparison.

**Acceptance Scenarios**:

1. **Given** the completed optimization, **When** a maintainer follows the measurement procedure, **Then** the reported timing and coverage comparison can be reproduced on the representative host.
2. **Given** a proposed reduction in executed work, **When** maintainers inspect the evidence, **Then** each removed or shared operation is classified as duplicate, unnecessary for the target's contract, or a reusable prerequisite.

### Edge Cases

- The host exposes many logical CPUs but has insufficient memory for one worker per CPU.
- The host is already under unrelated load when a target starts.
- A test process hangs, crashes, or exits while other test work is still running.
- Multiple test groups fail concurrently and their output is interleaved.
- Warm caches hide duplicate work that remains expensive on a cold or partially warm run.
- A Nix unit case or flake check accidentally requests a broad project output or copies the working tree through an unsafe source reference.
- The available test inventory changes between baseline and optimized measurements.
- A required Rust companion run is not part of the primary test runner surface.
- An established external orchestration approach covers most but not all required test surfaces.
- Local consolidation conflicts with CI's intentionally split job structure.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The feature MUST establish reproducible elapsed-time baselines for the existing Rust, Nix unit, and flake test targets before optimization.
- **FR-002**: The baseline and optimized measurements MUST use the same representative host, repository revision ancestry, retained baseline test inventory, and matching warm or cold-cache condition. Tests added by this feature MUST be classified separately.
- **FR-003**: The Rust target MUST coordinate all of its required test surfaces through one top-level orchestration.
- **FR-004**: The Nix unit target MUST coordinate all of its required unit-test surfaces through one top-level orchestration.
- **FR-005**: The flake test target MUST coordinate all of its required check surfaces through one top-level orchestration.
- **FR-006**: The local orchestration MAY differ from CI orchestration, and this feature MUST NOT require CI to collapse its intentionally split jobs.
- **FR-007**: The planning phase MUST research established external approaches for Rust workspace testing, Nix unit testing, flake evaluation, and resource-aware scheduling before considering repository-specific orchestration code.
- **FR-008**: The selected design MUST use an established external orchestration capability when one satisfies the target contracts.
- **FR-009**: New Bash orchestration or custom orchestration code MUST be introduced only when the planning evidence shows that no suitable established approach can satisfy a required surface.
- **FR-010**: Any unavoidable repository-specific orchestration MUST be narrowly scoped to the unsupported surface and MUST justify why the external approach cannot cover it.
- **FR-011**: Each orchestration MUST run independent work concurrently when host capacity permits.
- **FR-012**: Each orchestration MUST bound concurrency according to available host capacity and MUST avoid unbounded worker creation.
- **FR-013**: The optimized Rust target MUST preserve every enforcing test and companion check in the current Rust target contract.
- **FR-014**: The optimized Nix unit target MUST preserve every enforcing case in the current Nix unit target contract.
- **FR-015**: The optimized flake test target MUST preserve every enforcing check in the current flake target contract.
- **FR-016**: The feature MUST identify and remove, combine, or reuse duplicate preparation, compilation, evaluation, discovery, and setup work within each target.
- **FR-017**: The Nix unit and flake test targets MUST avoid realizing unrelated project outputs when those outputs are not required to prove their coverage.
- **FR-018**: The optimized targets MUST preserve nonzero exit status when any required test surface fails.
- **FR-019**: Failure output MUST identify every observed failing test surface without requiring contributors to rerun the entire target to discover the first failure.
- **FR-020**: Concurrent execution MUST preserve readable diagnostics and MUST prevent one worker's output from corrupting another worker's result.
- **FR-021**: The feature MUST provide a documented, repeatable method for comparing elapsed time, executed test inventory, and duplicate or unnecessary work before and after optimization.
- **FR-022**: The selected design MUST explain how it increases useful parallel work, reuses expensive prerequisites, and limits memory or I/O contention.
- **FR-023**: The optimized targets MUST remain compatible with the repository's existing public Make target names and enforcing gate classifications.
- **FR-024**: The feature MUST distinguish performance improvements from skipped validation by comparing both the before-and-after source inventory and an executed-surface manifest produced by the actual aggregate target runs.
- **FR-025**: The feature MUST record any required test surface that cannot participate in the consolidated execution and justify how it is scheduled without serializing unrelated work.
- **FR-026**: The feature MUST not weaken failure handling, enforcement status, or coverage to meet the elapsed-time goal.
- **FR-027**: Each optimized target MUST record CPU use, effective CPU budget, peak memory, applicable cgroup memory events, and worker bounds during representative warm runs. Rust measurement MUST cover the target process scope; Nix measurements MUST include work delegated to the Nix daemon through daemon-cgroup counters when readable or baseline-adjusted host counters during an externally idle benchmark.
- **FR-028**: During the measured heavy interval, each optimized target MUST achieve at least 80% median utilization of its effective CPU budget unless the evidence identifies a narrower non-CPU bottleneck and the selected design has exhausted viable concurrency for that interval. The heavy interval begins at the first scheduler-admitted CPU-heavy leaf and ends when the last such leaf completes.
- **FR-029**: Resource acceptance MUST fail on orchestration-attributable out-of-memory events, sustained memory-pressure stalls, swap thrashing, worker growth beyond the declared bound, or configured CPU-consuming quotas whose active frontier exceeds the effective CPU budget.

### Key Entities

- **Test Surface**: A required group of tests or companion checks, including its enforcing status, prerequisites, execution constraints, and result.
- **Orchestration Run**: One invocation of a top-level validation target, including start and end time, host capacity, cache condition, scheduled surfaces, and outcomes.
- **Work Unit**: Independently schedulable validation work with declared prerequisites and resource needs.
- **Performance Baseline**: The accepted pre-change measurements and test inventory used for comparison.
- **Coverage Inventory**: The complete set of enforcing cases, tests, and companion checks that must remain represented after optimization.
- **Orchestration Approach**: An established external capability or, only when necessary, a narrowly scoped repository-specific mechanism that schedules required work and aggregates results.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The median elapsed time of three equivalent warm-cache Rust validation runs is at most 50% of the accepted warm-cache baseline median.
- **SC-002**: The median elapsed time of three equivalent warm-cache Nix unit validation runs is at most 50% of the accepted warm-cache baseline median.
- **SC-003**: The median elapsed time of three warm-cache local Layer-1 flake validation runs is at most 50% of the legacy local-shard baseline median. The direct `make test-flake` path is measured separately and MUST NOT regress by more than 20%.
- **SC-004**: Cold-cache elapsed time is measured for all three targets and reduced as far as the selected design reasonably permits without weakening coverage; the cold-cache result does not block completion.
- **SC-005**: All three optimized targets execute 100% of their pre-change enforcing coverage inventory, with no required surface silently omitted.
- **SC-006**: Repeated runs complete without host out-of-memory termination, uncontrolled worker growth, or sustained resource thrashing attributable to the orchestration.
- **SC-007**: The slowest of three equivalent optimized warm-cache runs for each target is no more than 20% slower than that target's optimized warm median unless documented external contention invalidates the run. Cold-cache variance is diagnostic and non-blocking.
- **SC-008**: Every observed test failure is reported in the same invocation, and each target returns a failing status whenever any required surface fails.
- **SC-009**: The Nix unit and flake test targets realize no unrelated whole-project output solely to provide their required coverage.
- **SC-010**: Maintainers can reproduce the before-and-after comparison using one documented procedure and can account for every removed, combined, or reused operation.
- **SC-011**: The completed design introduces no new Bash or custom orchestration unless the planning evidence identifies a required surface unsupported by suitable established external approaches.
- **SC-012**: Representative warm runs show at least 80% median effective-budget CPU utilization over the CPU-heavy interval, while active CPU quotas remain within budget, worker counts remain bounded, peak memory remains within the calculated envelope, and no orchestration-attributable OOM, sustained memory-pressure stall, or swap-thrashing event occurs.

## Assumptions

- The representative benchmark host is the contributor machine on which the current underutilization was observed.
- Baselines and optimized results use the median of three runs under documented, equivalent cache conditions; runs with known external contention are discarded and repeated.
- Existing Make target names remain the contributor-facing entry points.
- Current committed, passing target behavior and the authoritative test inventory define required coverage.
- The optimization may change internal orchestration and test grouping but not enforcement semantics.
- Warm-cache behavior is the blocking performance criterion. Cold-cache behavior is measured separately and optimized on a best-effort basis.
- The flake hard target compares the legacy local Layer-1 shard path with the optimized local Layer-1 path; the already-monolithic direct target is tracked separately for non-regression.
- Work outside `test-rust`, `test-nix-unit`, and `test-flake` is out of scope unless a shared prerequisite must change to remove duplication in one of those targets.
- CI may continue to use split jobs when that structure serves CI isolation, required contexts, or scheduling needs; this feature targets local execution.
- Established external tooling and native ecosystem capabilities are preferred over new Bash or custom orchestration.
