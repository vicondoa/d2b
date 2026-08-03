# Feature Specification: Implement ADR 0052 Bazel Rust Gate

**Feature Branch**: `adr052-bazel-rust-spec`

**Created**: 2026-08-02

**Status**: Draft

**Input**: User description: "create a spec for adr 052 on a sepwrate worktree"

## Context

ADR 0052 is accepted and defines a staged migration of d2b's Rust build and
test gate from its current Cargo-based execution path to Bazel. The migration
is intended to remove duplicated compilation and scheduling work while
preserving the exact coverage, failure, test-isolation, supply-chain, and
execution-evidence contracts that protect the repository today.

The existing `make test-rust` path and required `test-rust` continuous-
integration context remain authoritative during a shadow period. A separate
Bazel path must demonstrate complete equivalence, enforce its own failure
conditions, satisfy fixed performance ceilings, and operate within the
repository's disk and cache budgets before promotion. Promotion changes the
executor beneath the Rust gate without changing the required context name or
silently removing contributor-facing entry points.

This feature is Rust-only. It does not migrate Nix evaluation, Nix packaging,
VM or image work, fixture materialization, release artifacts, static guest
binaries, cross-compilation, remote execution, or any Layer-1 job outside the
Rust rollup. The detailed mechanisms and safety invariants in
`docs/adr/0052-bazel-rust-build-and-test.md` remain binding.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Run a complete Bazel Rust gate beside Cargo (Priority: P1)

As a d2b contributor, I can run one Bazel-backed Rust validation entry point
that covers every Rust surface currently represented by the required Rust
gate, while the existing Cargo path remains available as the authoritative
comparison.

**Why this priority**: The migration has no value unless the new path can run
the complete Rust contract without weakening the existing gate. The shadow
path is the minimum independently useful slice and the basis for every later
promotion decision.

**Independent Test**: Run the Bazel aggregate and each named slice on a
committed tree, compare the published execution-manifest identifiers and
verdict with the Cargo path, and confirm the Cargo path is unchanged.

**Acceptance Scenarios**:

1. **Given** a passing committed tree, **When** a contributor runs the Bazel
   Rust aggregate, **Then** all eighteen baseline Rust surfaces complete and
   publish the same versioned surface identifiers expected from the current
   Layer-1 Rust path.
2. **Given** a failure in any one mapped Rust surface, **When** the Bazel
   aggregate runs, **Then** the command fails, names that surface, and
   preserves partial completion evidence for the other observed surfaces.
3. **Given** the shadow stage is active, **When** contributors or continuous
   integration run the existing Rust target, **Then** it continues to use the
   current Cargo path and remains the authoritative required verdict.
4. **Given** fixture-backed contract surfaces are available, **When** the
   local Rust target runs, **Then** those surfaces remain on their existing
   Cargo and Nix path rather than being silently absorbed into the Bazel
   migration.

---

### User Story 2 - Preserve exact coverage and test isolation (Priority: P1)

As a maintainer, I can prove that the Bazel path executes the same tests,
policy checks, doctests, harness-free companions, scans, and generated-output
checks with the same isolation properties as the current gate.

**Why this priority**: A faster green result is harmful if it comes from
missing tests, empty scans, stale binaries, changed process topology, or
collapsed failure reporting. Exact census and topology are release-blocking
properties.

**Independent Test**: Compare exact per-surface and per-suite censuses,
ignored-case counts, companion discovery, process topology, and planted
negative controls between the current and Bazel paths.

**Acceptance Scenarios**:

1. **Given** the committed baseline of eighteen surface identifiers, **When**
   the coverage guard runs, **Then** every identifier maps to exactly one
   existing carrier and no Rust test target or hand-written build fragment is
   left unmapped.
2. **Given** a repository-scanning or generated-output check, **When** its
   declared input or output set is empty, incomplete, unreadable, unparsable,
   or different from the committed census, **Then** the check fails before it
   can report a clean or reproducible result.
3. **Given** the main workspace or guest shell runner test suites, **When**
   their tests run, **Then** each test case executes in a fresh process and
   ignored cases remain counted as ignored rather than passed.
4. **Given** any of the three privileged broker feature suites, **When** they
   run, **Then** they use the existing one-process-per-binary topology with
   bounded internal threads and never overlap another broker suite or
   unrelated test.
5. **Given** doctests or harness-free test binaries, **When** the Bazel path
   discovers companions, **Then** each required companion runs as its own
   named surface and an unexpectedly empty discovery fails.

---

### User Story 3 - Keep policy and supply-chain checks enforcing (Priority: P1)

As a security or release maintainer, I can rely on the Bazel path to preserve
the repository's dependency, advisory, source, license, API, no-bash, schema,
and pinned-test policies without network-dependent or empty-success behavior.

**Why this priority**: Half of the Rust execution-manifest surfaces are policy
or reproducibility checks rather than ordinary tests. Treating them as
incidental build steps would create silent security and compatibility gaps.

**Independent Test**: Run each policy surface against its expected input and
against a planted violating input, then compare the combined supply-chain
findings with the current Cargo path for all three lock files.

**Acceptance Scenarios**:

1. **Given** the three Rust dependency locks and policy configurations,
   **When** supply-chain validation runs, **Then** dependency bans, licenses,
   sources, and advisories remain enforcing and the Bazel actions require no
   network access.
2. **Given** a known advisory ignore, **When** advisory validation runs,
   **Then** the ignore applies to the same workspace and advisory as today and
   does not create a broader waiver.
3. **Given** schema generation, **When** reproducibility is checked, **Then**
   two independent generations each produce the exact committed nonempty
   schema census before their contents are compared.
4. **Given** the governed Rust source inventory, **When** the no-bash scan
   runs, **Then** its declared inputs and successfully parsed files equal the
   exact committed manifest in both directions.
5. **Given** a nightly API census or pinned test inventory, **When** its
   toolchain or observed census differs from the committed pin, **Then** the
   corresponding surface fails with the difference identified.

---

### User Story 4 - Get faster, bounded local feedback (Priority: P2)

As a contributor, I can run the Bazel Rust gate locally with bounded
concurrency, persistent incremental reuse, predictable disk consumption, and
actionable failure reporting.

**Why this priority**: The migration is justified by faster feedback, but
unbounded workers, uncontrolled caches, or unsafe cleanup would trade test
latency for workstation instability and data-loss risk.

**Independent Test**: Measure three cold and three warm local runs on the
reference host, exercise soft and hard disk limits, and run every cleanup and
shutdown refusal case against planted scratch trees and processes.

**Acceptance Scenarios**:

1. **Given** the reference development host, **When** the aggregate runs
   three times under the defined warm profile, **Then** the median completes
   within ten minutes and no run exceeds twelve minutes.
2. **Given** a fresh build state with a populated download cache, **When** the
   aggregate runs three times under the defined cold profile, **Then** the
   median completes within fifteen minutes and no run exceeds eighteen
   minutes.
3. **Given** limited CPU or memory availability, **When** the aggregate
   starts, **Then** it derives a bounded worker budget from the existing Rust
   budget control rather than creating a second independent resource policy.
4. **Given** local Bazel state reaches its soft size limit, **When** a run
   starts, **Then** the contributor receives the measured size and exact
   repository-relative reclaim command without losing the warm state.
5. **Given** local Bazel state reaches its hard size limit, **When** a run
   starts, **Then** the run refuses before build work begins and identifies
   the safe reclaim action.
6. **Given** a tracked file, symlink, magic link, escaping layout, or live
   Bazel server under the managed scratch tree, **When** cleanup is requested,
   **Then** cleanup deletes nothing, reaches nothing outside the managed
   subtree, and reports the condition with its specific redacted recovery
   steps.

---

### User Story 5 - Compare safely in continuous integration (Priority: P2)

As a maintainer, I can observe a non-required Bazel Rust workflow beside the
required Cargo workflow without evicting required caches, exposing
credentials, granting pull requests write capability, or confusing cache
maintenance failures with Rust test failures.

**Why this priority**: The shadow period must generate trustworthy evidence
without degrading the required path or widening the privilege available to
untrusted pull-request code.

**Independent Test**: Run the shadow workflow on its supported triggers,
inspect all cache and permission decisions, and exercise positive and negative
workflow-policy fixtures.

**Acceptance Scenarios**:

1. **Given** the shadow stage, **When** the Bazel workflow runs, **Then** it
   publishes no shared cache entry and does not alter the required Rust
   context.
2. **Given** a pull-request-triggered job, **When** workflow policy is
   evaluated, **Then** the job has read-only repository permission, cannot
   write Actions cache state, and cannot request `actions: write`.
3. **Given** a default-branch promotion run, **When** retired caches must be
   removed, **Then** a separate maintenance verdict deletes only authorized
   cache generations, verifies repository headroom, and remains outside the
   Rust test verdict.
4. **Given** any Rust build or test action, **When** it executes third-party
   code, **Then** cache service credentials are absent from its environment.
5. **Given** a promoted Bazel Rust job, **When** it starts after checkout,
   **Then** it carries an in-band deadline that can fail the job
   actionably before the job-level timeout backstop.

---

### User Story 6 - Promote and retire without breaking contributor contracts (Priority: P3)

As a maintainer, I can promote Bazel only after complete equivalence evidence
exists, preserve the stable required context and familiar Make entry points,
and retire the Cargo implementation in separately reversible steps.

**Why this priority**: Promotion is the point of the feature, but it must
follow evidence rather than anticipation. Separating promotion, compatibility,
and retirement keeps rollback possible and prevents interface breakage from
being confused with executor failures.

**Independent Test**: Evaluate the complete promotion evidence set, perform a
promotion rehearsal, verify all aliases and context names, then confirm that
retirement conditions independently block premature deletion.

**Acceptance Scenarios**:

1. **Given** fewer than ten consecutive matching default-branch shadow
   verdicts, an incomplete seeded-failure matrix, a missed performance
   ceiling, or any failed census or topology proof, **When** promotion is
   evaluated, **Then** promotion is blocked.
2. **Given** all promotion criteria are satisfied, **When** the promotion
   change lands, **Then** the required context remains named `test-rust`, the
   authoritative Rust target uses Bazel for the eighteen baseline surfaces,
   and fixture-backed contract surfaces remain on their existing path.
3. **Given** a contributor invokes a pre-promotion Bazel target name after
   promotion, **When** the compatibility alias runs, **Then** it forwards to
   the authoritative target, returns the same status, and prints the named
   replacement.
4. **Given** the promotion commit has not shipped in a release or the promoted
   default branch has not completed ten consecutive green runs, **When**
   alias removal or Cargo implementation retirement is proposed, **Then** the
   change is blocked.

### Edge Cases

- A baseline surface is removed from the map while its aggregate still exits
  successfully.
- A new Rust test target or hand-written build fragment is present but no
  surface claims it.
- A scan sees declared files but silently skips one because it cannot be read
  or parsed.
- Two schema generations both produce empty trees and therefore have matching
  digests.
- A binary-locating test resolves to a missing, non-executable, stale, or
  wrong binary.
- A broker suite is accidentally allowed to overlap another test process.
- An ignored test is counted as passed or omitted from the census.
- The Bazel client leader exits during timeout grace while a descendant
  remains alive.
- A deadline is absent, expired, malformed, overflowing, or rounded later
  than the allowed ceiling.
- Cleanup encounters a tracked file, a symlink or magic link, a path escape,
  a subtree replacement race, or a server that did not shut down.
- Cache enumeration is incomplete, an entry matches more than one authorized
  prefix, or repository usage changes between headroom verification and save.
- A pull-request workflow can reach a cache writer indirectly through a
  post-step or inherited permission.
- A performance ceiling is missed even though all coverage and correctness
  checks pass.
- A fixture-backed contract surface is accidentally counted among the
  eighteen Rust-only migration surfaces.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The feature MUST implement the accepted ADR 0052 decision for
  the Rust gate and MUST NOT infer authority over any explicitly excluded Nix,
  packaging, image, release, static-binary, cross-compilation, remote-cache,
  remote-execution, or non-Rust Layer-1 surface.
- **FR-002**: Cargo manifests, Cargo lock files, dependency policy files, and
  the committed Rust toolchain pins MUST remain the authoritative dependency,
  feature-selection, policy, and compiler inputs.
- **FR-003**: A dependency or toolchain change MUST be initiated through the
  authoritative Cargo or toolchain files and MUST fail validation when the
  derived Bazel state has not been regenerated.
- **FR-004**: The Bazel and companion tool versions used by the feature MUST
  be pinned, reviewable, and unavailable through an unpinned fallback.
- **FR-005**: The feature MUST add a local Bazel Rust aggregate and the
  ADR-defined slice and shutdown entry points while leaving the existing
  authoritative Rust target unchanged during the shadow stage.
- **FR-006**: Every workflow entry point introduced by the feature MUST invoke
  an approved Make target rather than invoking the underlying scheduler
  directly.
- **FR-007**: The Bazel aggregate MUST represent all eighteen baseline
  execution-manifest surfaces and no fixture-backed conditional surface.
- **FR-008**: A committed coverage map MUST associate each baseline surface
  with exactly one existing carrier, continuous-integration slice, exact
  census, and declared test process topology where applicable.
- **FR-009**: Coverage validation MUST fail on an unmapped identifier, missing
  carrier, unmapped Rust test target, missing process topology, missing exact
  census, or unlisted hand-written build fragment.
- **FR-010**: Every logical check MUST retain an independently attributable
  verdict; no aggregate wrapper may collapse several surfaces into one
  indistinguishable result.
- **FR-011**: The existing versioned execution-manifest contract MUST retain
  the same surface identifiers, completed-surface semantics, failed-surface
  semantics, and partial evidence on failure or handled interruption.
- **FR-012**: The main workspace and guest shell runner suites MUST preserve
  one fresh process per test case, exact per-binary test census, and faithful
  ignored-case reporting.
- **FR-013**: The three broker feature suites MUST preserve one process per
  test binary with bounded internal threads and MUST execute exclusively until
  a separate isolation review authorizes a change.
- **FR-014**: Doctests and harness-free companions MUST remain independently
  discovered and executed, and an unexpectedly empty discovery MUST fail.
- **FR-015**: Repository-scanning and generated-output checks MUST assert
  exact nonempty input and output censuses before accepting a clean or
  reproducible result.
- **FR-016**: The schema reproducibility surface MUST compare two independent
  generations, each containing the exact committed schema set with nonempty
  valid content.
- **FR-017**: The no-bash scan MUST prove equality between the committed
  governed-source manifest, declared scan inputs, and successfully parsed
  files.
- **FR-018**: Binary-locating tests MUST prove that the selected binary exists,
  is executable, and has the expected identity before exercising it.
- **FR-019**: Dependency bans, licenses, sources, and advisories MUST remain
  enforcing across all three Rust workspaces with no network access from a
  Bazel action.
- **FR-020**: The feature MUST compare the current combined supply-chain
  outcomes with the migrated checks for all three locks and MUST block
  promotion on any differing enforcing outcome.
- **FR-021**: Advisory database freshness and advisory ignores MUST be
  explicit committed inputs, and an ignore MUST retain its current workspace
  and advisory scope.
- **FR-022**: The API census, pinned test inventory, scanner controls, and
  other non-compilation checks MUST each have a planted failure that proves
  the check can reject a violating input.
- **FR-023**: Local concurrency MUST use the existing memory-aware Rust budget
  control and MUST remain bounded across scheduler-level and per-suite
  concurrency.
- **FR-024**: Persistent local Bazel state MUST live only beneath the
  worktree's ignored scratch tree and MUST have documented size and age
  bounds.
- **FR-025**: The local runner MUST warn at the configured soft output-state
  limit and MUST refuse to start build work at the configured hard limit.
- **FR-026**: Cleanup MUST shut down the matching Bazel server before
  deletion, anchor all traversal beneath the managed scratch tree, refuse
  symlinks, magic links, escapes, and tracked files, and never delete through
  a path that can be re-resolved after validation.
- **FR-027**: Cleanup descriptors MUST not be inherited across child process
  execution, and both supported traversal routes MUST have enforcing
  behavioral coverage.
- **FR-028**: Every cleanup or shutdown refusal MUST delete nothing and emit a
  stable static code with the exact repository-relative recovery for that
  condition.
- **FR-029**: Refusal and timeout messages MUST exclude absolute paths,
  output-state hashes, user identifiers, process identifiers, raw deadline
  values, opaque handles, and unsafe recursive-removal instructions.
- **FR-030**: The shadow continuous-integration workflow MUST remain
  non-required, MUST keep the existing required graph unchanged, and MUST
  publish no shared cache entry.
- **FR-031**: Pull-request-reachable jobs MUST be read-only, MUST NOT request
  `actions: write`, and MUST NOT save through direct, indirect, or post-step
  cache writers.
- **FR-032**: Cache credentials MUST remain confined to the cache action and
  MUST NOT enter a command step or any build, test, build-script, or macro
  environment.
- **FR-033**: Promotion caching MUST keep the action cache and download cache
  separate, MUST never cache the output base, and MUST bind cache keys to all
  dependency, toolchain, policy, module, and generated-build inputs named by
  ADR 0052.
- **FR-034**: A default-branch-only maintenance verdict MUST remove only
  authorized retired or superseded cache generations, paginate completely,
  verify headroom, and remain independent of the Rust test verdict.
- **FR-035**: Pull requests MUST restore caches read-only, and exactly one
  authorized default-branch job MAY publish a new cache generation.
- **FR-036**: Repository cache usage plus the planned promoted snapshot MUST
  be at or below 8 GiB before publication, and publication MUST refuse if
  headroom changes before save.
- **FR-037**: The aggregate MUST satisfy wall-clock ceilings of ten minutes
  warm local, fifteen minutes cold local, and fifteen minutes cold continuous
  integration under the ADR-defined reference profiles.
- **FR-038**: Each performance profile MUST be evaluated using all required
  measurements, with a passing median at or below the ceiling and no
  individual measurement above 1.2 times the ceiling.
- **FR-039**: Promoted continuous-integration jobs MUST enforce an in-band
  deadline that includes checkout within the total ceiling and retains a
  slightly higher job timeout only as a dead-runner backstop.
- **FR-040**: Deadline conversion MUST use one checked interpretation at both
  handoff ends, reject malformed or overflowing input without echoing it, and
  apply conservative rounding that can only shorten the available time.
- **FR-041**: An expired deadline MUST be reported as a normal budget expiry,
  not malformed input, and a missing deadline MUST remain an unbounded local
  default while being forbidden in promoted jobs.
- **FR-042**: On deadline expiry, the runner MUST terminate only the dedicated
  child process group, wait the full fixed grace, terminate surviving
  descendants, and reap the direct child only after escalation completes.
- **FR-043**: The timeout path MUST never signal its own process group, group
  zero, group negative one, or a detached server process identifier read from
  a file.
- **FR-044**: A missed performance ceiling MUST block promotion or fail the
  promoted job and MUST authorize only a larger runner class or a further
  disjoint slice split; it MUST NOT authorize weaker coverage, lower
  enforcement, surface removal, or a relaxed ceiling.
- **FR-045**: Promotion MUST be blocked until all coverage, census, topology,
  supply-chain, cache, and performance requirements pass, ten consecutive
  default-branch shadow verdicts match the Cargo verdict, and an
  eighteen-surface seeded-failure matrix proves each carrier fails
  independently.
- **FR-046**: Promotion MUST preserve the required context name `test-rust`,
  route the eighteen baseline surfaces through Bazel, and leave the two
  fixture-backed contract surfaces on their existing path.
- **FR-047**: Existing Rust leaf target names MUST continue to work after
  promotion, and the Bazel-specific names MUST become status-preserving
  compatibility aliases with an actionable deprecation message.
- **FR-048**: No workflow MAY call a deprecated compatibility alias after
  promotion.
- **FR-049**: Compatibility aliases MUST NOT be removed before the promotion
  has shipped in at least one release, and their removal MUST be a separate
  documented change.
- **FR-050**: The Cargo implementation for the eighteen migrated surfaces MUST
  NOT be retired until the promoted path has completed ten consecutive green
  default-branch runs, and retirement MUST leave the fixture-contract mode
  intact.
- **FR-051**: The enforcing guards for cleanup, timeout, deadline, recovery
  messages, workflow permissions, cache writers, and required deadline
  controls MUST land with the plumbing they constrain.
- **FR-052**: Every new guard MUST include a positive case and a planted
  negative fixture or mutation that proves the guard fails when its protected
  invariant is removed.
- **FR-053**: The feature MUST use existing Rust, policy, and workflow test
  surfaces for its guards and MUST NOT add a new top-level shell gate,
  Layer-1 job, or independent required context.

### Key Entities

- **Rust Surface**: One versioned execution-manifest identifier representing a
  required compilation, test, policy, scan, or reproducibility outcome.
- **Carrier Target**: The independently reported Bazel target responsible for
  one Rust surface and its declared inputs, outputs, census, and failure.
- **Coverage Map**: The committed one-to-one relationship between baseline
  surfaces, carrier targets, continuous-integration slices, exact censuses,
  process topologies, and deliberate execution differences.
- **Execution Manifest**: The versioned evidence for completed and failed Rust
  surfaces, including partial evidence from failed or interrupted runs.
- **Test Topology**: The required process-isolation model for a suite,
  including per-case or per-binary execution, thread bounds, exclusivity, and
  ignored-case accounting.
- **Shadow Run**: A non-authoritative Bazel execution compared with the Cargo
  verdict and retained as promotion evidence.
- **Performance Profile**: A reproducible warm local, cold local, or cold
  continuous-integration measurement with a defined host, cache state, start,
  stop, and ceiling.
- **Cache Generation**: One bounded, keyed action-cache or download-cache
  snapshot with an authorized writer and retention policy.
- **Promotion Evidence Set**: The complete coverage, equivalence, seeded
  failure, census, topology, performance, supply-chain, and cache evidence
  required to change the authoritative executor.
- **Recovery Condition**: A stable refusal or timeout classification with
  redacted output and condition-specific repository-relative remediation.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All 18 baseline Rust surface identifiers have exactly one valid
  carrier, and the coverage guard reports zero unmapped identifiers, targets,
  test targets, process topologies, exact censuses, or hand-written fragments.
- **SC-002**: Ten consecutive default-branch shadow runs produce the same
  pass or fail verdict from the Bazel and Cargo Rust rollups.
- **SC-003**: An 18-case seeded-failure matrix causes the intended carrier to
  fail in all 18 cases and causes zero unrelated Rust surfaces to fail.
- **SC-004**: The Bazel and Cargo paths report identical test-case,
  ignored-case, doctest, harness-free companion, API, schema, scanner, and
  pinned-inventory censuses for every migrated suite.
- **SC-005**: The three broker suites pass 20 consecutive executions under
  the required exclusive topology.
- **SC-006**: Three warm local measurements have a median of at most 10
  minutes and a maximum of at most 12 minutes.
- **SC-007**: Three cold local measurements have a median of at most 15
  minutes and a maximum of at most 18 minutes.
- **SC-008**: The last five scheduled cold continuous-integration measurements
  have a median of at most 15 minutes and no measurement above 18 minutes.
- **SC-009**: The shadow stage creates zero shared Bazel cache entries, and
  pull-request-reachable jobs create zero cache writes and request zero
  `actions: write` permissions.
- **SC-010**: Before the first promoted cache save, measured repository cache
  usage plus the planned snapshot is at most 8 GiB, with one authorized
  default-branch writer and zero cached output-base trees.
- **SC-011**: All three supply-chain workspaces produce identical enforcing
  findings before and after decomposition, with zero network-dependent Bazel
  actions and zero broadened advisory ignores.
- **SC-012**: Every cleanup, timeout, deadline, message-redaction, cache-policy,
  and workflow-policy guard rejects all of its planted negative variants and
  accepts its compliant positive case.
- **SC-013**: In every observed Bazel failure, contributors can identify the
  failing surface from the same invocation without rerunning the complete
  aggregate.
- **SC-014**: Promotion changes zero required context names and leaves all
  documented Rust leaf entry points callable with status equivalent to their
  authoritative replacement.
- **SC-015**: The migration can be rolled back before Cargo retirement by
  reverting the promotion change without reconstructing deleted Rust gate
  behavior.

## Assumptions

- ADR 0052 is accepted and is the binding architectural decision for this
  feature.
- Committed, passing code and the current execution-manifest reference define
  the authoritative baseline when prose and implementation differ.
- The baseline migration set is eighteen Rust surfaces under
  `D2B_SKIP_FIXTURE_BUILD=1`; the two fixture-backed contract surfaces remain
  outside this Bazel-only set.
- The reference local host and continuous-integration runner are the profiles
  defined by ADR 0052 unless a separately reviewed change records a new basis.
- The current Make target names and required `test-rust` context are public
  contributor contracts that must survive promotion.
- The shadow period intentionally carries two working Rust execution paths and
  accepts the temporary maintenance and disk cost.
- Detailed cleanup, timeout, deadline, cache, and process-control mechanics are
  implemented exactly as constrained by ADR 0052 and are not redesigned by
  this feature specification.

### Dependencies

- The accepted ADR 0052 document and its referenced execution-manifest,
  toolchain, supply-chain, no-bash, workflow, and test-inventory contracts.
- The current Cargo Rust gate must remain runnable throughout shadow evidence
  collection.
- The pinned development environment must provide the accepted Bazel and
  companion tool versions.
- Promotion depends on default-branch shadow history and repository cache
  maintenance capabilities that cannot be demonstrated by a source diff
  alone.
- Alias removal depends on a release containing the promotion commit, and
  Cargo retirement depends on post-promotion default-branch history.
