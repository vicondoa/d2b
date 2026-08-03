# Feature Specification: Implement ADR 0052 Bazel Rust Gate

**Feature Branch**: `adr052-bazel-rust-spec`

**Created**: 2026-08-02

**Status**: Approved

**Input**: User description: "create a spec for adr 052 on a sepwrate worktree"

## Context

ADR 0052 is accepted, was amended on 2026-08-03 after an upstream review of the
build substrate, and defines a staged migration of d2b's Rust build and test
gate from its current Cargo-based execution path to Bazel. The amended record
is settled authority for this feature: it fixes the promotion lineage, the
qualification-record definition, the binary and fixture location rules, the
nightly-channel mechanism, the vendored supply-chain materialization, and the
closed list of deliberate differences. The migration is intended to remove
duplicated compilation and scheduling work while preserving the exact coverage,
failure, test-isolation, supply-chain, and execution-evidence contracts that
protect the repository today.

The existing `make test-rust` path and required `test-rust`
continuous-integration context remain authoritative during a shadow period. A separate
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

## Clarifications

### Session 2026-08-02

- Q: Which branch owns promotion evidence, cache maintenance, and publication?
  -> A: The protected `v3` integration lineage.
- Q: Which runs supply the cold continuous-integration measurement set?
  -> A: The five most recent qualifying cold Bazel runs drawn from the
  qualification-record stream on protected `v3`.

### Session 2026-08-03

- Q: What is a qualification record, given that a pull-request merge reference
  is recomputed against a moving base and both paths must test one tree?
  -> A: A qualification record is a push event on protected `v3` produced by a
  merged pull request. Both the Cargo and the Bazel workflow runs are
  identified by the same head commit under that push event, so "both paths
  tested the same commit" is mechanically true. Pull-request runs are
  diagnostic only and never enter a streak or a measurement set.
- Q: What must each qualification record carry beyond the two compared
  verdicts?
  -> A: A passing `D2B_SKIP_FIXTURE_BUILD=1` Rust rollup equivalence result for
  both executors and a passing same-commit fixture-contract companion verdict.
  The fixture surfaces stay outside the Bazel comparison, but they cannot
  regress invisibly behind a qualifying record.
- Q: How does the streak treat a run that reaches no verdict?
  -> A: A Bazel run that reaches no verdict while its paired Cargo run reaches
  one counts as a mismatch and resets the streak. A push where neither side
  reaches a verdict is not a record and neither extends nor resets.
- Q: Are the migration's exact censuses committed literals?
  -> A: No. Every census is derived by the repository-owned generator from the
  same selector the current Cargo gate uses, committed as a generated artifact,
  and drift-checked. Literal counts in planning prose are descriptive only.
- Q: If the recorded supply-chain comparison finds no yanked-state difference,
  is the yanked carrier still built?
  -> A: Yes. The committed lock-bounded yanked snapshot and its three carriers
  under the existing `rust-deny-*` identifiers land in the shadow stage
  unconditionally. The comparison is one observation of one lock set at one
  moment; a capability conditioned on that observation is absent exactly when
  the first real finding arrives, after promotion has retired the Cargo
  executor that used to carry the outcome. Promotion still records the
  comparison and still blocks on any differing enforcing outcome, refresh stays
  an explicit reviewed networked update outside the gate, and the gate's drift
  check stays offline key-set equality run by one repository-owned validator
  that a contributor can run in a shell and get the same message from.
- Q: How is a committed dependency-resolution lock regenerated if the
  re-resolution environment controls are forbidden?
  -> A: Through one repository-owned command that names a single hub from the
  closed hub set, sets the re-resolution controls only in the environment of
  the one child process it spawns, reuses the wrapper's absolute startup values
  and output root, writes only that hub's committed lock, and fails when any
  other generated or committed derived artifact changed. It is not a build
  entry point and no workflow may reach it. The prohibition on setting those
  controls in a build entry point or continuous-integration environment is
  unchanged; a supported narrow path exists so the prohibition is not routed
  around under deadline.
- Q: The build-system module lock is a different mechanism from a hub lock. How
  is that one regenerated?
  -> A: Through its own repository-owned command, which takes no arguments,
  reuses the same absolute startup values, writes only the module lock, refuses
  when any other tracked derived file changed, and changes nothing on a tree
  that is already current. It is the exact remediation the module-lock refusal
  names, so the refusal never leaves a contributor to reconstruct an invocation
  from an upstream diagnostic that omits every startup option this repository
  requires.

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
   the coverage guard runs, **Then** every identifier maps to a nonempty set of
   existing carriers, every carrier belongs to exactly one identifier, and no
   Rust test target or hand-written build fragment is left unmapped.
2. **Given** a repository-scanning or generated-output check, **When** its
   declared input or output set is empty, incomplete, unreadable, unparsable,
   or different from the committed census, **Then** the check fails before it
   can report a clean or reproducible result.
3. **Given** the main workspace or guest shell runner test suites, **When**
   their tests run, **Then** each test case executes in a fresh process, each
   case reports its own outcome in the structured per-case result the runner
   publishes, and ignored cases remain counted as ignored rather than passed.
4. **Given** any of the three privileged broker feature suites, **When** they
   run, **Then** they use the existing one-process-per-binary topology with
   bounded internal threads and never overlap another broker suite or
   unrelated test.
5. **Given** doctests or harness-free test binaries, **When** the Bazel path
   discovers companions, **Then** each required companion runs as its own
   named surface, the executed harness-free set matches the selector the Cargo
   gate uses today, and an unexpectedly empty discovery fails.
6. **Given** a test that must locate a first-party binary or fixture, **When**
   it runs under either executor, **Then** it resolves that binary or fixture
   through the declared mechanism for the executor it is actually running
   under, never falls back to the other executor's mechanism, and fails naming
   the expected location when the declared entry is missing.

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
   sources, and advisories remain enforcing, the materialized dependency tree
   contains exactly the packages the locks record, and the Bazel actions
   require no network access.
2. **Given** a known advisory ignore, **When** advisory validation runs,
   **Then** the ignore applies to the same workspace and advisory as today and
   does not create a broader waiver.
3. **Given** schema generation, **When** reproducibility is checked, **Then**
   two independent generations each produce the exact generated and committed
   nonempty schema census before their contents are compared.
4. **Given** the governed Rust source inventory, **When** the no-bash scan
   runs, **Then** its declared inputs and successfully parsed files equal the
   exact committed manifest in both directions.
5. **Given** a nightly API census or pinned test inventory, **When** the
   toolchain the census work actually used, or its observed census, differs
   from the committed pin, **Then** the corresponding surface fails with the
   difference identified.
6. **Given** the dependency policy tools, **When** any of them would need
   network access at check time, **Then** the required state is instead a
   committed, drift-checked input and the check runs offline.
7. **Given** a shadow-stage tree, **When** the supply-chain carriers are
   inventoried, **Then** the committed lock-bounded yanked-state snapshot and
   its three carriers are present under the existing dependency-policy
   identifiers whether or not the recorded comparison found a yanked
   difference, and the snapshot's offline key set equals the three locks' key
   set exactly as proved by the repository-owned offline validator the carriers
   and a contributor shell both run.
8. **Given** a committed dependency-resolution lock that no longer matches its
   Cargo lock, **When** a contributor regenerates it, **Then** the only
   supported path is the repository-owned single-hub command, that command
   changes only the named hub's lock, and the same regeneration control set in
   a build entry point or continuous-integration environment is still
   rejected.
9. **Given** a build-system module resolution the committed module lock does
   not cover, **When** any build entry point runs, **Then** the run fails
   without rewriting the lock and names the repository-owned no-argument
   refresh command as the recovery, and running that command updates only the
   module lock, leaves every other tracked derived file unchanged, and changes
   nothing when it is run a second time.
10. **Given** a repository-owned lock regeneration that also modified a tracked
    file it does not own, **When** the command completes its post-check, **Then**
    it fails, lists the unrelated changed paths repository-relative and never
    absolute, and names committing or restoring those paths followed by
    rerunning the same scoped command as the recovery.
11. **Given** an exported dependency-regeneration control in the ambient
    environment, **When** either the single-hub lock regeneration command or
    the no-argument module-lock refresh command is run, **Then** each refuses
    to start, names the three variables to unset, and ends its own recovery on
    the exact command that was refused rather than on a shared alternative that
    would have to ask for an argument the refused command does not take.
12. **Given** the reviewed networked snapshot updater, **When** its handling of
    index answers is tested, **Then** every answer is supplied through an
    injectable boundary and no test opens a socket, and the offline validator
    can reach neither that boundary nor its networked implementation.

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
7. **Given** the cleanup, result-file, and deadline paths, **When** their
   planted negatives run, **Then** every filesystem effect and every reading
   of current time is taken through an injectable boundary, so the negatives
   reproduce on any host without a full disk, a privileged mount, or a
   manipulated host clock.

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
inspect all cache and permission decisions, exercise positive and negative
workflow-policy fixtures, and confirm that only push events on protected `v3`
produced by merged pull requests yield qualification records.

**Acceptance Scenarios**:

1. **Given** the shadow stage, **When** the Bazel workflow runs, **Then** it
   publishes no shared cache entry and does not alter the required Rust
   context.
2. **Given** a pull-request-triggered job, **When** workflow policy is
   evaluated, **Then** the job has read-only repository permission, cannot
   write Actions cache state, and cannot request `actions: write`.
3. **Given** a `v3` promotion run, **When** retired caches must be
   removed, **Then** a separate maintenance verdict deletes only authorized
   cache generations, verifies repository headroom, and remains outside the
   Rust test verdict.
4. **Given** any Rust build or test action, **When** it executes third-party
   code, **Then** cache service credentials are absent from its environment.
5. **Given** a promoted Bazel Rust job, **When** it runs, **Then** its
   enforceable deadline covers the complete measured job window and can fail
   actionably before the outer timeout backstop.
6. **Given** a push to protected `v3` produced by a merged pull request,
   **When** both the Cargo and Bazel workflows run, **Then** both are
   identified by the same head commit, both verdicts and the same-commit
   fixture-contract verdict are recorded as one qualification record, and a
   pull-request run produces no record at all.

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

1. **Given** fewer than ten consecutive matching qualification records, an
   incomplete seeded-failure matrix, a missed performance ceiling, or any
   failed census or topology proof, **When** promotion is evaluated, **Then**
   promotion is blocked.
2. **Given** all promotion criteria are satisfied, **When** the promotion
   change lands, **Then** the required context remains named `test-rust`, the
   authoritative Rust target uses Bazel for the eighteen baseline surfaces,
   and fixture-backed contract surfaces remain on their existing path.
3. **Given** a contributor invokes a pre-promotion Bazel target name after
   promotion, **When** the compatibility alias runs, **Then** it forwards to
   the authoritative target, returns the same status, and prints the named
   replacement.
4. **Given** the promotion commit has not shipped in a release, **When** alias
   removal is proposed, **Then** alias removal is blocked independently of the
   promoted-run count.
5. **Given** the promoted `v3` lineage has not completed ten consecutive green
   runs, **When** Cargo implementation retirement is proposed, **Then**
   retirement is blocked independently of release containment or alias state.
6. **Given** Cargo implementation retirement has landed, **When** a
   contributor runs the public Rust target or any documented Rust leaf name,
   **Then** the name still exists and invokes the authoritative Bazel carrier,
   and the fixture-contract mode still runs on its existing path.

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
- A binary-locating test misses its declared entry under the new executor and
  silently falls back to a stale artifact left by the other executor.
- A broker suite is accidentally allowed to overlap another test process.
- An ignored test is counted as passed or omitted from the census.
- Every test case passes but the structured per-case result cannot be
  published, so the run looks green with no per-case evidence.
- A structured per-case result carries an environment value, an absolute path,
  or raw child output that the redaction rules forbid.
- A partially written structured result is left behind after a full disk, or a
  path the runner did not create is removed during cleanup of its own
  temporary file.
- The top-level validation process exits during timeout handling while one of
  its descendants remains alive.
- A deadline is absent, expired, malformed, overflowing, or rounded later
  than the allowed ceiling.
- Cleanup encounters a tracked file, a symlink or magic link, a path escape,
  a subtree replacement race, or a server that did not shut down.
- Cache enumeration is incomplete, an entry matches more than one authorized
  prefix, or repository usage changes between headroom verification and save.
- A cache key input changes without changing the key, so a subtly stale cache
  is restored.
- Cache trimming is requested but has not finished when size is measured, so a
  compliant run refuses to publish forever.
- A pull-request workflow can reach a cache writer indirectly through a
  post-step or inherited permission.
- A qualification streak is inflated by pairing runs that tested different
  trees, or by cancelling a run that was about to fail.
- A dependency policy check runs against a materialized tree that is quietly
  short a package, so it reports fewer findings and exits zero.
- The migration comparison finds no yanked crate today, so no yanked detection
  is built, and the first crate yanked after promotion is invisible on every
  path.
- A committed dependency-resolution lock is regenerated by an ad hoc
  environment override instead of the reviewed repository-owned command, so a
  lock is rewritten silently, a second build server is started, or an unrelated
  generated artifact changes in the same operation.
- The build system's own drift diagnostic names a raw refresh invocation that
  carries none of the server-selecting startup values this repository requires,
  so following the diagnostic literally starts a second build server and
  populates persistent state outside the managed scratch subtree.
- A declared direct build-system dependency disagrees with the resolved graph,
  and the resolution absorbs the difference with a warning and a zero exit, so
  the committed lock records a version nobody declared as though it were
  intended.
- A yanked-state snapshot is refreshed by the networked updater and committed
  without anyone running the offline validator, so a snapshot whose key set
  does not match the locks reaches continuous integration instead of the
  contributor's shell.
- A regeneration intended for one dependency hub rewrites another hub's lock or
  rewrites the authoritative Cargo lock the migration froze.
- A cleanup, result-file, or deadline guard depends on live host filesystem
  state or the host clock, so the planted negative it claims to reject cannot
  actually be produced on the reference host.
- The networked snapshot updater is written against a client it constructs
  itself, so no test can supply a partial, revisionless, or malformed index
  answer, and every refusal path it owns stays unproven until it first fires in
  a contributor's shell.
- The offline snapshot validator can reach the same networked client the
  updater uses, so a future change makes the gate's drift check open a socket
  without any guard noticing.
- The nightly toolchain selection fails to apply to the census, or applies to
  the whole build so every first-party crate compiles off the stable pin.
- Graph discovery traverses scratch or Cargo output directories inside the
  worktree and either fails or silently absorbs generated files.
- A performance ceiling is missed even though all coverage and correctness
  checks pass.
- A fixture-backed contract surface is accidentally counted among the
  eighteen Rust-only migration surfaces.
- Retirement of the Cargo implementation removes a public entry point name
  that contributors and documentation still use.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The feature MUST implement the accepted ADR 0052 decision for
  the Rust gate and MUST NOT infer authority over any explicitly excluded Nix,
  packaging, image, release, static-binary, cross-compilation, remote-cache,
  remote-execution, or non-Rust Layer-1 surface.
- **FR-002**: Cargo manifests, Cargo lock files, dependency policy files, and
  the committed Rust toolchain pins MUST remain the authoritative dependency,
  feature-selection, policy, and compiler inputs. Every independent Rust
  workspace whose output the Rust gate executes, including the no-bash scanner
  tool workspace, MUST keep its own lock as the authoritative resolution and
  MUST NOT be re-resolved into another workspace's dependency set.
- **FR-003**: A dependency or toolchain change MUST be initiated through the
  authoritative Cargo or toolchain files and MUST fail validation when the
  derived Bazel state has not been regenerated. The derived state MUST include
  the enumerated set of third-party packages that run build scripts, the
  configuration each of them requires, and the explicit minimal set of host
  environment values any build action is allowed to observe.
- **FR-004**: The build system, its dependency generator, its rule sets, and
  every companion tool used by the feature MUST be pinned by version and
  content, reviewable, provided by the pinned development environment, and
  unavailable through an unpinned fallback, an unpinned source bootstrap, or a
  re-resolution escape hatch in any local or continuous-integration
  environment. Transitive build-system modules MUST be pinned by a committed
  resolution lock that fails closed rather than silently updating, including
  when a declared direct dependency disagrees with the resolved graph.
  Regenerating
  a committed dependency-resolution lock MUST be possible only through a
  repository-owned command that names exactly one dependency hub from a closed
  set, applies the re-resolution control solely to the environment of the
  single child process it spawns, reuses the same absolute server-selecting
  startup values the wrapper supplies, writes only that hub's committed lock,
  and fails when any other generated or committed derived artifact changes.
  Regenerating the committed build-system module resolution lock MUST likewise
  be possible only through a separate repository-owned command that takes no
  arguments, reuses the same absolute server-selecting startup values, writes
  only that lock, fails when any other tracked derived artifact changes,
  completes with no change on an already-current tree, and is the exact
  remediation the module-lock refusal names. Neither command MAY be reachable
  from a build entry point or a workflow, and no build entry point or
  continuous-integration environment MAY set a re-resolution control.
- **FR-005**: The feature MUST add a local Bazel Rust aggregate and the
  ADR-defined slice and shutdown entry points while leaving the existing
  authoritative Rust target unchanged during the shadow stage.
- **FR-006**: Every workflow entry point introduced by the feature MUST invoke
  an approved Make target rather than invoking the underlying scheduler
  directly.
- **FR-007**: The Bazel aggregate MUST represent all eighteen baseline
  execution-manifest surfaces and no fixture-backed conditional surface.
- **FR-008**: A committed coverage map MUST associate each baseline surface
  with a nonempty carrier set, exactly one continuous-integration slice, an
  exact derived census, and a declared test process topology where applicable,
  and every carrier MUST belong to exactly one baseline surface.
- **FR-009**: Coverage validation MUST fail on an unmapped identifier, missing
  carrier, carrier claimed by more than one surface, unmapped Rust test target,
  missing process topology, missing exact census, or unlisted hand-written
  build fragment. Carrier existence MUST be proven by real declared dependency
  edges at graph-analysis time rather than by a query issued from inside a
  test, and graph-completeness and query-drift checks MUST run outside the
  test over a committed drift-checked or declared query result.
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
  discovered and executed, the executed harness-free set MUST be derived from
  the same selector the current gate uses rather than from a manifest count,
  every excluded manifest entry MUST be recorded with its exclusion reason, and
  an unexpectedly empty discovery MUST fail.
- **FR-015**: Repository-scanning and generated-output checks MUST assert
  exact nonempty input and output censuses before accepting a clean or
  reproducible result.
- **FR-016**: The schema reproducibility surface MUST compare two independent
  generations, each containing the exact generated and committed schema census
  with nonempty valid content, and that census MUST be a drift-checked
  generator output rather than a hand-maintained count.
- **FR-017**: The no-bash scan MUST prove equality between the committed
  governed-source manifest, declared scan inputs, and successfully parsed
  files.
- **FR-018**: Every test that locates a first-party binary or reads a
  repository fixture MUST resolve it through the declared mechanism for the
  executor it is running under, MUST select that mechanism once with no
  fallback to the other executor's mechanism, MUST NOT resolve anything by an
  absolute build-execution-root path, MUST declare each located binary and
  fixture as a declared input, and MUST prove that the selected binary exists,
  is executable, and has the expected identity before exercising it. A check
  that needs the repository inventory rather than a specific file MUST consume
  a generated drift-checked manifest as a declared input.
- **FR-019**: Dependency bans, licenses, sources, and advisories MUST remain
  enforcing across the three Rust workspace locks they cover today, with no
  network access from any build or test action. The dependency tree the policy
  tools read MUST be materialized only from pinned, content-verified sources,
  MUST classify every lock entry or refuse by name, and MUST assert that the
  materialized package count equals the lock's before any policy tool runs.
- **FR-020**: The feature MUST compare the current combined supply-chain
  outcomes with the migrated checks for all three workspace locks and MUST
  block promotion on any differing enforcing outcome. An offline
  yanked-dependency-state carrier reporting under the existing
  dependency-policy identifiers MUST land during the shadow stage regardless of
  what that comparison finds, so the detection capability exists before the
  first difference rather than in response to it; dropping the outcome is not
  authorized, and an all-clear snapshot is a valid committed baseline rather
  than a reason to omit the carrier.
- **FR-021**: Advisory database freshness, advisory ignores, and the
  yanked-state snapshot MUST be explicit committed inputs; an ignore MUST
  retain its current workspace and advisory scope; refreshing a snapshot MUST
  be an explicit reviewed networked operation outside the gate through one
  repository-owned command; that command MUST reach the index through a single
  injectable boundary whose one networked implementation is the only site
  permitted to open a socket for it, so every refusal it can produce is
  provable from supplied responses; and the gate's own drift check MUST be a
  separate offline repository-owned command that proves exact key-set equality
  with the committed locks rather than regenerating state, MUST NOT be able to
  reach that boundary or its networked implementation at all, is the single
  implementation and the single message for that comparison, and is runnable
  unchanged both by the gate carriers and by a contributor in a shell.
- **FR-022**: The API census, pinned test inventory, scanner controls, and
  other non-compilation checks MUST each have a planted failure that proves
  the check can reject a violating input.
- **FR-023**: Local concurrency MUST use the existing memory-aware Rust budget
  control and MUST remain bounded across scheduler-level and per-suite
  concurrency.
- **FR-024**: Persistent local build state MUST live only beneath the
  worktree's ignored scratch tree, MUST have documented size and age bounds,
  and MUST be reclaimed by an explicit synchronous operation whose completion
  is observable before any size measurement or publication decision.
- **FR-025**: The local runner MUST warn at the configured soft output-state
  limit and MUST refuse to start build work at the configured hard limit.
- **FR-026**: Cleanup MUST operate only within the managed scratch subtree,
  MUST refuse ambiguous or unsafe layouts and live ownership, and MUST
  guarantee that no refused or successful cleanup reaches tracked content or
  any external target.
- **FR-027**: Cleanup safety MUST remain effective across every supported host
  path and across child process execution, with behavioral evidence that a
  planted unsafe variant is rejected.
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
- **FR-032**: Cache credentials MUST remain unavailable to repository and
  third-party build or test code.
- **FR-033**: Promotion caching MUST keep the action cache and download cache
  separate, MUST never cache the build system's output base, and MUST bind
  cache keys to every dependency, toolchain, policy, module-resolution,
  per-workspace generated-dependency, generator-binary, workspace-boundary,
  build-script-configuration, action-environment, and generated-build input
  the migration reads, so that changing any of them produces a different key.
- **FR-034**: A `v3`-only maintenance verdict MUST remove only
  authorized retired or superseded cache generations, paginate completely,
  verify headroom, and remain independent of the Rust test verdict.
- **FR-035**: Pull requests MUST restore caches read-only, and exactly one
  authorized protected-`v3` job MAY publish a new cache generation.
- **FR-036**: Repository cache usage plus the planned promoted snapshot MUST
  be at or below 8 GiB before publication, and publication MUST refuse if
  headroom changes before save.
- **FR-037**: The aggregate MUST satisfy wall-clock ceilings of ten minutes
  warm local, fifteen minutes cold local, and fifteen minutes cold continuous
  integration under the ADR-defined reference profiles. The cold
  continuous-integration ceiling MUST NOT become binding until a recorded
  feasibility measurement on the real runner class demonstrates it is
  attainable, and a feasibility shortfall MUST be answered only by a larger
  runner class or a further disjoint slice split.
- **FR-038**: Each performance profile MUST be evaluated using all required
  measurements, with a passing median at or below the ceiling and no
  individual measurement above 1.2 times the ceiling. A measurement taken
  under streamed test output, an altered cache state, or any other condition
  that changes scheduling MUST be invalidated and replaced rather than
  averaged in.
- **FR-039**: Promoted continuous-integration jobs MUST enforce an actionable
  in-band deadline that covers the complete measured job window and retains a
  slightly higher outer timeout only as a dead-runner backstop.
- **FR-040**: Deadline calculations MUST fail safely on invalid or
  unrepresentable input, MUST not disclose the rejected value, and MUST never
  grant more time than the applicable ceiling.
- **FR-041**: An expired deadline MUST be reported as a normal budget expiry,
  not malformed input, and a missing deadline MUST remain an unbounded local
  default while being forbidden in promoted jobs.
- **FR-042**: On deadline expiry, the runner MUST give the validation work a
  fixed graceful-stop interval, MUST terminate any surviving descendants
  after that interval, and MUST finish with no orphaned validation process.
- **FR-043**: Timeout handling MUST affect only processes owned by the current
  validation run and MUST leave the caller, unrelated processes, and detached
  server processes untouched.
- **FR-044**: A missed performance ceiling MUST block promotion or fail the
  promoted job and MUST authorize only a larger runner class or a further
  disjoint slice split; it MUST NOT authorize weaker coverage, lower
  enforcement, surface removal, or a relaxed ceiling.
- **FR-045**: Promotion MUST be blocked until all coverage, census, topology,
  supply-chain, cache, and performance requirements pass, ten consecutive
  qualification records show matching Bazel and Cargo rollup verdicts at the
  same head commit with a passing same-commit fixture-contract companion, and
  an eighteen-surface seeded-failure matrix proves each carrier fails
  independently. A qualification record MUST be a push event on protected `v3`
  produced by a merged pull request; pull-request, other-branch, scheduled, and
  manually dispatched runs are diagnostic and MUST NOT enter a streak or a
  measurement set. A differing verdict MUST reset the streak, a Bazel run that
  reaches no verdict while its paired Cargo run does MUST reset the streak, and
  a push where neither side reaches a verdict MUST NOT be a record.
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
  `v3` runs. Retirement MUST remove only Cargo implementations and unreachable
  Cargo-only plumbing; it MUST NOT remove the public Rust target name or any
  documented Rust leaf name, which MUST continue to invoke the authoritative
  Bazel carriers, and it MUST leave the fixture-contract mode intact.
- **FR-051**: The enforcing guards for cleanup, timeout, deadline, recovery
  messages, workflow permissions, cache writers, and required deadline
  controls MUST land with the plumbing they constrain.
- **FR-052**: Every new guard MUST include a positive case and a planted
  negative fixture or mutation that proves the guard fails when its protected
  invariant is removed. Guards over filesystem effects, over elapsed or
  absolute time, and over networked registry-index responses MUST be exercised
  through injectable boundaries, so every planted negative is reproducible
  without depending on live host filesystem state, a full disk, a privileged
  mount, the host clock, or a reachable network. The one networked
  implementation of an index boundary MUST be exercised only by the explicit
  contributor-run operation that owns it, outside the gate, and that run MUST
  be recorded as a measured observation rather than repeated as a gate
  assertion.
- **FR-053**: The feature MUST use existing Rust, policy, and workflow test
  surfaces for its guards and MUST NOT add a new top-level shell gate,
  Layer-1 job, or independent required context.
- **FR-054**: Every migrated test suite MUST publish a structured per-case
  result to the location the executor designates, containing one entry per
  enumerated case with explicit passed, failed, and ignored outcomes and only
  the stable case name, outcome, bounded duration, and bounded sanitized
  failure text. Environment values, command-line arguments, absolute paths,
  store paths, socket paths, runfiles or worktree locations, unit names,
  process identifiers, user identifiers, opaque handles, terminal bytes, shell
  names, and raw child output MUST be absent from it, while raw child output
  remains available in the executor's ordinary per-target log artifact. Each
  case MUST receive its own temporary directory beneath the executor-supplied
  temporary root, its binary MUST be resolved through declared runfiles, and
  only the declared test environment MUST be forwarded. Result publication is
  enforcing: an otherwise passing suite whose result cannot be published MUST
  fail, and where a test already failed the test failure MUST remain the
  primary diagnosis with the publication failure reported additionally.
- **FR-055**: The repository MUST declare its build-graph boundary and its
  persistent state locations explicitly: scratch state and every Cargo output
  directory in the worktree MUST be excluded from graph discovery by a
  generated, drift-checked exclusion list; convenience links MUST be placed
  beneath the scratch tree by an absolute prefix the wrapper supplies; and all
  server-selecting startup paths MUST be supplied as absolute values by the
  wrapper, byte-identical across build, test, query, information, shutdown, and
  clean invocations, rather than written into the checked-in configuration
  file.

### Key Entities

- **Rust Surface**: One versioned execution-manifest identifier representing a
  required compilation, test, policy, scan, or reproducibility outcome.
- **Carrier Target**: The independently reported build target responsible for
  one Rust surface and its declared inputs, outputs, census, and failure.
- **Coverage Map**: The committed total and unambiguous relationship between
  baseline surfaces, carrier sets, continuous-integration slices, derived
  censuses, process topologies, hand-written build fragments, and deliberate
  execution differences.
- **Execution Manifest**: The versioned evidence for completed and failed Rust
  surfaces, including partial evidence from failed or interrupted runs.
- **Per-Case Result Document**: The structured, redacted, per-test-case record
  the runner publishes for each carrier, carrying case name, outcome, bounded
  duration, and bounded sanitized failure text and nothing else.
- **Test Topology**: The required process-isolation model for a suite,
  including per-case or per-binary execution, thread bounds, exclusivity, and
  ignored-case accounting.
- **Test Locator Migration**: The enumerated set of first-party test files that
  stop resolving binaries and repository paths through compile-time Cargo
  environment expansion, each recorded either as migrated or as needing no
  migration with the reason.
- **Qualification Record**: One push event on protected `v3` produced by a
  merged pull request, carrying the head commit, both workflow run
  identifiers, both rollup verdicts, the same-commit fixture-contract verdict,
  and, for a cold sample, the four slice durations.
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

- **SC-001**: All 18 baseline Rust surface identifiers have a nonempty valid
  carrier set, every carrier belongs to exactly one identifier, and the
  coverage guard reports zero unmapped identifiers, targets, test targets,
  process topologies, exact censuses, or hand-written fragments.
- **SC-002**: Ten consecutive qualification records produce the same pass or
  fail verdict from the Bazel and Cargo Rust rollups at the same head commit,
  each with a passing same-commit fixture-contract companion verdict.
- **SC-003**: An 18-case seeded-failure matrix causes the intended carrier to
  fail in all 18 cases and causes zero unrelated Rust surfaces to fail.
- **SC-004**: The Bazel and Cargo paths report identical test-case,
  ignored-case, doctest, harness-free companion, API, schema, scanner, and
  pinned-inventory censuses for every migrated suite, and every one of those
  censuses is a generated drift-checked artifact rather than a hand-maintained
  count.
- **SC-005**: The three broker suites pass 20 consecutive executions under
  the required exclusive topology.
- **SC-006**: Three warm local measurements have a median of at most 10
  minutes and a maximum of at most 12 minutes.
- **SC-007**: Three cold local measurements have a median of at most 15
  minutes and a maximum of at most 18 minutes.
- **SC-008**: A recorded feasibility measurement on the real runner class
  exists, and the five most recent qualifying cold Bazel qualification records
  have a median of at most 15 minutes and no measurement above 18 minutes.
- **SC-009**: The shadow stage creates zero shared Bazel cache entries, and
  pull-request-reachable jobs create zero cache writes and request zero
  `actions: write` permissions.
- **SC-010**: Before the first promoted cache save, measured repository cache
  usage plus the planned snapshot is at most 8 GiB after an explicit
  synchronous trim has completed, with one authorized protected-`v3` writer and
  zero cached output-base trees.
- **SC-011**: All three supply-chain workspaces produce identical enforcing
  findings before and after decomposition, with zero network-dependent build or
  test actions, zero broadened advisory ignores, a materialized package count
  equal to each lock's, and a committed lock-bounded yanked-state snapshot
  whose offline key-set drift check passes for all three locks.
- **SC-012**: Every cleanup, timeout, deadline, message-redaction,
  per-case-result redaction, result-file filesystem, binary-locator,
  cache-policy, and workflow-policy guard rejects all of its planted negative
  variants and accepts its compliant positive case.
- **SC-013**: In every observed Bazel failure, contributors can identify the
  failing surface and the failing test case from the same invocation without
  rerunning the complete aggregate.
- **SC-014**: Promotion changes zero required context names and leaves all
  documented Rust leaf entry points callable with status equivalent to their
  authoritative replacement, and Cargo implementation retirement removes zero
  public entry point names.
- **SC-015**: The migration can be rolled back before Cargo retirement by
  reverting the promotion change without reconstructing deleted Rust gate
  behavior.

## Assumptions

- ADR 0052 is accepted, is amended as of 2026-08-03, and that amended record is
  the binding architectural decision for this feature. The amendment is a
  merged prerequisite rather than work this feature performs.
- Committed, passing code and the current execution-manifest reference define
  the authoritative baseline when prose and implementation differ.
- The baseline migration set is eighteen Rust surfaces under
  `D2B_SKIP_FIXTURE_BUILD=1`; the two fixture-backed contract surfaces remain
  outside this Bazel-only set and are carried as a required same-commit
  companion verdict rather than being compared between executors.
- The reference local host and continuous-integration runner are the profiles
  defined by ADR 0052 unless a separately reviewed change records a new basis.
- The current Make target names and required `test-rust` context are public
  contributor contracts that must survive both promotion and Cargo
  implementation retirement.
- The shadow period intentionally carries two working Rust execution paths and
  accepts the temporary maintenance and disk cost, including that both binary
  location arms stay green on the Cargo path for its whole duration.
- Promotion, cache maintenance, publication, the equivalence streak, and the
  post-promotion observation window all run on protected `v3`.
- Detailed cleanup, timeout, deadline, cache, locator, per-case result, vendor
  materialization, and process-control mechanics are implemented exactly as
  constrained by the amended ADR 0052 and are not redesigned by this feature
  specification.

### Dependencies

- The amended ADR 0052 document and its referenced execution-manifest,
  toolchain, supply-chain, no-bash, workflow, and test-inventory contracts.
- The current Cargo Rust gate must remain runnable throughout shadow evidence
  collection.
- The pinned development environment must provide the accepted build system and
  companion tool versions, including the build-file formatting tools, without
  requiring an unpinned version launcher anywhere on the gate path.
- Promotion depends on `v3` qualification-record history and repository cache
  maintenance capabilities that cannot be demonstrated by a source diff
  alone.
- Alias removal depends only on a release containing the promotion commit, and
  Cargo implementation retirement depends only on post-promotion `v3` history.
