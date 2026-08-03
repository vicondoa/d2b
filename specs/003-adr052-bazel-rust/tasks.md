# Tasks: ADR 0052 Bazel Rust Gate

**Input**: Approved design documents in `specs/003-adr052-bazel-rust/`, the amended `docs/adr/0052-bazel-rust-build-and-test.md`, committed code, `.specify/memory/constitution.md`, `AGENTS.md`, and `tests/AGENTS.md`.

**Prerequisites**: `specs/003-adr052-bazel-rust/spec.md`, `specs/003-adr052-bazel-rust/plan.md`, `specs/003-adr052-bazel-rust/research.md`, `specs/003-adr052-bazel-rust/data-model.md`, `specs/003-adr052-bazel-rust/quickstart.md`, and every file in `specs/003-adr052-bazel-rust/contracts/`. The ADR 0052 amendment of 2026-08-03 is a **merged prerequisite**, not a task here.

**Tests**: Tests are mandatory. Write the named positive and planted negative or mutation tests before the corresponding implementation, observe the stated failure, and retain the test in an existing Layer-1 Rust, policy, drift, or workflow-policy carrier.

**Wave rule**: Pre-W0, W0, W1, W2, W3, W4, and W5 are strictly serialized. W6 and W7 are independent children of W5 and may run in either order after their own mechanical entry gate passes. No pipelined dispatch is used.

**Panel rule**: Every plan panel and integrated-diff panel uses the ten-role roster `software`, `test`, `nixos`, `networking`, `security`, `rust`, `product`, `docs`, `observability`, and `kernel`. For this delivery run the `software` seat is filled by the Bazel and `rules_rust` expert, because every finding that forced the ADR amendment was substrate-level.

## Format: `[ID] [P?] [Story] Description`

- `[P]` means the task owns disjoint files and all of its prerequisites are already complete.
- `[US1]` through `[US6]` map tasks to the six approved user stories.
- Every task names its exact owned file or a precise generated-output directory or glob.

## Phase 1: Setup - Pre-W0 Amended ADR Verification

**Purpose**: Prove the settled authority is present before any Bazel implementation begins.

- [ ] T001 Verify from `docs/adr/0052-bazel-rust-build-and-test.md` on the W0 base commit that the record is `Status: Accepted`, carries the 2026-08-03 amendment line, names protected `v3` as the promotion, cache-maintenance, cache-publication, streak, and post-promotion lineage, defines a qualification record as a push on `refs/heads/v3` produced by a merged pull request, and sources the cold measurement set from qualifying cold qualification records; confirm the ADR index row in `docs/adr/README.md`; and record the verified commit under `.scratch/adr052-w0-adr-verification/`.
- [ ] T002 Prove the amended ADR commit is an ancestor of the W0 base and fail the gate if any of `.bazelversion`, `.bazelrc`, `.bazelignore`, `MODULE.bazel`, `MODULE.bazel.lock`, `bazel/`, or generated `packages/**/BUILD.bazel` exists in a commit that predates it.

**Checkpoint**: The amended ADR is present and verified. W0 is now allowed to begin.

---

## Phase 2: Foundational - W0 Reversible Foundation

**Purpose**: Land the shared pinned toolchain, the four dependency hubs, the generated graph and workspace boundary, the schema prerequisite, the hermeticity inventory, the coverage-map shape, and the frozen runner and locator crates while Cargo remains authoritative.

**Critical**: No user-story implementation starts until W0 is merged and sealed.

- [ ] T003 Run the W0 plan panel against `specs/003-adr052-bazel-rust/plan.md` and `specs/003-adr052-bazel-rust/contracts/` with the ten-role roster and the Bazel and `rules_rust` expert in the `software` seat, require unanimous empty recommendations, and stage only classification metadata under `.scratch/adr052-w0-plan-panel/`.
- [ ] T004 Land a W0 integrator prep commit that creates `packages/xtask/src/bazel.rs`, `packages/xtask/src/schema.rs`, and `packages/xtask/src/hermeticity.rs`, registers all three modules and their CLI parsing and routing seams in `packages/xtask/src/main.rs`, then create separate W0 worktrees from that prep commit tip: `foundation-tools` owns `.bazelversion`, `.bazelrc`, `MODULE.bazel`, `MODULE.bazel.lock`, `flake.nix`, `bazel/cargo/`, `packages/xtask/tests/policy_ci.rs`, and hand-written `bazel/`; `generator` owns only `packages/xtask/src/bazel.rs` and its tests; `schema` owns only `packages/xtask/src/schema.rs` and its tests; `hermeticity` owns only `packages/xtask/src/hermeticity.rs` and its tests; `runner` owns only `packages/d2b-bazel-runner/`; `locator` owns only `packages/d2b-test-locator/`; the integrator alone owns `Makefile`, `packages/Cargo.toml`, `packages/xtask/src/main.rs`, generated `packages/**/BUILD.bazel`, generated `bazel/generated/**`, generated `.bazelignore`, `tests/golden/bazel-rust-coverage.json`, and `changelog.d/adr052-bazel-foundation.md`.
- [ ] T005 Add failing unit tests in `packages/xtask/src/schema.rs` that prove `gen-schemas --out-dir .scratch/schema-generation` rejects invalid output layouts, that the emitted census is the manifest the writer returns rather than a hand-written literal, and that two generated trees must each contain exactly that census with nonempty valid JSON before content comparison; first observe failure against the current fixed-output and empty-snapshot behavior. (FR-015, FR-016)
- [ ] T006 Implement exact schema output support behind the T004 CLI seam in `packages/xtask/src/schema.rs`, preserving the current no-argument default `docs/reference/schemas/v2/` output and returning the emitted census as a generator artifact, without editing integrator-owned `packages/xtask/src/main.rs`. (FR-015, FR-016)
- [ ] T007 Add failing generator tests in `packages/xtask/src/bazel.rs` covering all four hub Cargo metadata roots plus `packages/Cargo.guest.lock`, `cargo xtask gen-bazel`, `cargo xtask gen-bazel --check`, deterministic generated BUILD ownership, the exact governed-source inventory, the generator-derived executed harness-free and doctest censuses with recorded out-of-census reasons, generated `.bazelignore` content, stale Cargo and Bazel-side locks, and stable and nightly toolchain mismatch; each mutation must fail before implementation. (FR-002, FR-003, FR-004, FR-014, FR-055)
- [ ] T008 Implement `gen-bazel` and `gen-bazel --check` behind the T004 CLI seam in `packages/xtask/src/bazel.rs`, with generated ownership limited to `packages/**/BUILD.bazel`, `tests/tools/no-bash-ast-walker/BUILD.bazel`, `bazel/generated/**`, `bazel/generated/governed-rust-sources.bzl`, and `.bazelignore`; derive from `packages/Cargo.toml`, `packages/Cargo.lock`, `packages/d2b-priv-broker/Cargo.toml`, `packages/d2b-priv-broker/Cargo.lock`, `packages/d2b-guest-shell-runner/Cargo.toml`, `packages/d2b-guest-shell-runner/Cargo.lock`, `tests/tools/no-bash-ast-walker/Cargo.toml`, `tests/tools/no-bash-ast-walker/Cargo.lock`, and `packages/Cargo.guest.lock`, without editing integrator-owned `packages/xtask/src/main.rs`. (FR-002, FR-003, FR-014)
- [ ] T009 Implement generated `.bazelignore` emission in `packages/xtask/src/bazel.rs` covering `.scratch/` and every Cargo output directory any workspace or tool in the worktree creates, including `packages/d2b-priv-broker/target-layer1/`, `packages/d2b-priv-broker/target-fakebackends/`, `tests/tools/no-bash-ast-walker/target/`, and the `proofs/` and `labs/` workspace output directories; prove a mutation that drops one entry and a mutation that emits an empty list both fail. (FR-055)
- [ ] T010 [P] Pin Bazel 8.6.0 in `.bazelversion` and `flake.nix`, add `bazel_8` and `bazel-buildtools` to the repository dev shell without requiring Bazelisk anywhere on the gate path, and write `.bazelrc` with only `common`, `build`, `test`, and `build:<config>` lines, including `common --lockfile_mode=error`, with no `startup` line and no `@rules_rust//rust/toolchain/channel` setting. (FR-001, FR-004, FR-055)
- [ ] T011 [P] Declare four `crate.from_cargo` hubs in `MODULE.bazel` over `packages/Cargo.lock`, `packages/d2b-priv-broker/Cargo.lock`, `packages/d2b-guest-shell-runner/Cargo.lock`, and `tests/tools/no-bash-ast-walker/Cargo.lock`, set `lockfile = ...` on every one, commit the four Bazel-side locks under `bazel/cargo/`, pin one measured Bazel-compatible `rules_rust` version with `MODULE.bazel.lock` committed, and record in `bazel/cargo/README.md` that `packages/Cargo.guest.lock` is a generator and cache-key input and deliberately not a hub. (FR-002, FR-003, FR-004)
- [ ] T012 Pin the `cargo-bazel` generator by its registry URL and sha256 in `MODULE.bazel` and `bazel/cargo/`, add a structural guard in `packages/xtask/tests/policy_ci.rs` that refuses the non-reproducible source-bootstrap fallback and refuses any `CARGO_BAZEL_REPIN`, `REPIN`, or `CARGO_BAZEL_REPIN_ONLY` setting in `Makefile` or `.github/workflows/`, and observe both negatives failing before the guard lands. (FR-004)
- [ ] T013 Add failing tests then implement the hermeticity inventory in `packages/xtask/src/hermeticity.rs`: enumerate every third-party crate per hub for which a build-script target is generated, record the annotations each requires, pin the explicit minimal action-environment allowlist, and emit all of it as a drift-checked artifact under `bazel/generated/`; prove an unenumerated build-script crate and an unlisted action-environment value each fail closed. (FR-003, FR-033)
- [ ] T014 [P] Create repository-owned Bazel support in `bazel/BUILD.bazel`, `bazel/defs.bzl`, and `bazel/toolchains.bzl` registering stable Rust 1.97.0 and `nightly-2026-02-16` together, with no network-capable action, no remote cache, and no remote execution. (FR-001, FR-002, FR-019)
- [ ] T015 Freeze and commit the runner-owned internal helper crate at `packages/d2b-bazel-runner/` by adding `packages/d2b-bazel-runner/Cargo.toml`, `packages/d2b-bazel-runner/src/lib.rs`, and the module files `coverage.rs`, `topology.rs`, `runner_env.rs`, `junit.rs`, `fsops.rs`, `manifest.rs`, `budget.rs`, `cleanup.rs`, `deadline.rs`, `process.rs`, `recovery.rs`, plus `packages/d2b-bazel-runner/src/bin/d2b-bazel-runner.rs`; leave integrator-owned `packages/Cargo.toml` unchanged and expose no public runtime or daemon surface.
- [ ] T016 Freeze and commit the locator crate at `packages/d2b-test-locator/` with `Cargo.toml`, `src/lib.rs`, and its own tests, fixing the public surface as a call-site macro for the Cargo arm and a runfiles function for the Bazel arm, and add a compile-level test proving the Cargo arm cannot be provided as a shared library function. (FR-018)
- [ ] T017 Define the deterministic eighteen-row data shape in `tests/golden/bazel-rust-coverage.json` with a nonempty carrier set per ID, one verdict-owning carrier, four slices, generated-build digest, governed-source reference, derived census and out-of-census reason fields, topology reference, binary-provider and runfiles-path identity, locator-file disposition, hand-written-fragment list, committed query-result reference, and deliberate-difference fields, without adding `rust-contract-tests` or `rust-cli-contract-tests`. (FR-007, FR-008)
- [ ] T018 Run `cargo xtask gen-bazel` only into `.scratch/adr052-w0-generator-preview/`, prove `cargo xtask gen-bazel --check` fails on one planted dependency, toolchain, governed-source, census, `.bazelignore`, Bazel-side-lock, and generated-BUILD drift mutation each, and hand the preview plus mutation results to the integrator without editing tracked `packages/**/BUILD.bazel`, `bazel/generated/**`, `.bazelignore`, or foundation-owned Bazel lock outputs. (FR-003, FR-004)
- [ ] T019 Add the W0 semantic release note to `changelog.d/adr052-bazel-foundation.md`, describing pinned Bazel tooling and generated Rust graph support without process markers.
- [ ] T020 Commit each remaining W0 scope before validation with qualified `spec003w0` tags, stage only its owned paths from `specs/003-adr052-bazel-rust/plan.md`, merge the foundation, generator, schema, hermeticity, committed T015 runner, and committed T016 locator tips into the W0 integration branch, add `d2b-bazel-runner` and `d2b-test-locator` membership only in integrator-owned `packages/Cargo.toml`, run `cargo xtask gen-bazel` once on the integrated tree, and commit only integrator-owned generated `packages/**/BUILD.bazel`, `tests/tools/no-bash-ast-walker/BUILD.bazel`, `bazel/generated/**`, `.bazelignore`, and the final coverage digest while preserving foundation-owned Bazel locks.
- [ ] T021 Validate the committed W0 tree with `make check-tier0`, `make test-lint`, `make test-rust-schema`, `make test-rust-inventory`, `make test-drift`, and `make test-policy` from `Makefile`; prove `D2B_SKIP_FIXTURE_BUILD=1 make test-rust` remains Cargo-authoritative and emits the existing eighteen IDs; and run the `quickstart.md` foundation checks for the absent `startup` line, the absent channel flag, the four `lockfile` declarations, the absent repin controls, and `.bazelignore` coverage.
- [ ] T022 Run the W0 integrated-diff panel over the committed W0 range using the ten-role roster with the Bazel and `rules_rust` expert in the `software` seat, supply T021 evidence from `.scratch/adr052-w0-validation/`, require unanimous empty recommendations, and store no panel transcript in Git.
- [ ] T023 Open the W0 PR to protected `v3`, wait for required checks on the stable head, seal `spec003w0` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge every W0 scope and verify the merged tree satisfies `cargo xtask gen-bazel --check`.
- [ ] T024 After the W0 merge, run `nix-collect-garbage`, remove finished W0 worktree `packages/target/` and `.scratch/bazel/` trees, and verify only merged branches remain in `git worktree list`.

**Checkpoint**: W0 is merged and sealed. Cargo is still authoritative, and the stable foundation is available to every W1 scope.

---

## Phase 3: User Story 1 - Run a Complete Bazel Rust Gate Beside Cargo (Priority: P1)

**Goal**: Provide the Bazel aggregate, four slices, the Make compatibility surface with wrapper-supplied absolute startup options, and the execution-manifest v1 adapter while leaving Cargo authoritative.

**Independent Test**: On the integrated W1 candidate, run all six new Make targets, validate a passing and a failed v1 execution manifest, prove the startup options are byte-identical across every Bazel command the wrapper issues, and confirm `make test-rust` still invokes Cargo.

- [ ] T025 [US1] Run the W1 plan panel against the W1 section of `specs/003-adr052-bazel-rust/plan.md` and all files in `specs/003-adr052-bazel-rust/contracts/`, require unanimous ten-role signoff with the Bazel and `rules_rust` expert in the `software` seat, and stage only classification metadata under `.scratch/adr052-w1-plan-panel/`.
- [ ] T026 [US1] Land the W1 integrator prep commit in `Makefile`, `packages/d2b-bazel-runner/src/lib.rs`, `packages/d2b-bazel-runner/src/manifest.rs`, `packages/d2b-test-locator/src/lib.rs`, and `ci/rust/BUILD.bazel`, fixing the approved Make names, the absolute startup-option construction seam, the four-slice labels, the Build Event Protocol adapter boundary, the locator public macro surface the migration scope consumes, and file ownership before opening the `main`, `api`, `broker`, `aux`, `runner`, `locator`, `coverage`, and `generator` worktrees; the `api` scope additionally owns `packages/xtask/tests/policy_ci.rs` for W1, and only the integrator may reconcile `ci/rust/BUILD.bazel` or generated `packages/**/BUILD.bazel`, `tests/tools/no-bash-ast-walker/BUILD.bazel`, `bazel/generated/**`, and `.bazelignore`.
- [ ] T027 [P] [US1] Add failing Make-interface tests in `packages/d2b-bazel-runner/tests/make_interface.rs` for `test-bazel-rust`, `test-bazel-rust-main`, `test-bazel-rust-api`, `test-bazel-rust-broker`, `test-bazel-rust-aux`, and `bazel-shutdown`; require root execution, carrier-attributed failure, absolute wrapper-supplied startup options that are byte-identical across `build`, `test`, `query`, `info`, `shutdown`, and `clean`, a mutation that perturbs one command's startup options failing closed, and status propagation, while deferring bounded shutdown expiry and `D2B-BZLSERVER-STUCK` to T085 and T098. (FR-005, FR-006, FR-010, FR-055)
- [ ] T028 [P] [US1] Add failing manifest-adapter tests in `packages/d2b-bazel-runner/tests/manifest_adapter.rs` for the exact eighteen v1 IDs, prior-record invalidation, sorted partial evidence on failure and handled interruption, fixture-ID exclusion, original-status preservation, independent failed-surface attribution, and one surface verdict per ID even where the ID has several carriers. (FR-010, FR-011, SC-013)
- [ ] T029 [US1] Implement the six shadow-stage entry points and the shared absolute startup-option construction in `Makefile`, keeping all existing `test-rust` and `test-rust-*` targets Cargo-authoritative and keeping fixture mode in `tests/test-rust.sh`. (FR-005, FR-006, FR-046, FR-055)
- [ ] T030 [US1] Implement Build Event Protocol to execution-manifest v1 adaptation in `packages/d2b-bazel-runner/src/manifest.rs` and `packages/d2b-bazel-runner/src/bin/d2b-bazel-runner.rs`, using `docs/reference/schemas/test-execution-manifest-v1.json` unchanged and emitting a surface only after every carrier and companion in its coverage row passes. (FR-010, FR-011)
- [ ] T031 [P] [US1] Define the aggregate, the four slice groups, the independently named carrier aliases, and the coverage guard dependency edges in `bazel/carriers/aggregate.bzl`; T074 alone reconciles these definitions into `ci/rust/BUILD.bazel`, and workflows and Make consume them only through approved Make targets. (FR-006, FR-007, FR-010)
- [ ] T032 [US1] Add a shell-execution mutation test in `packages/d2b-bazel-runner/tests/no_shell.rs` that fails if any repository-owned wrapper, case runner, cleanup helper, timeout wrapper, or process-control path invokes `sh`, `bash`, `setsid`, or another command interpreter, and that explicitly does not fail on the `rules_rust`-generated stable-channel doctest runner, which is the recorded deliberate difference; keep all repository-owned execution on `std::process::Command` in `packages/d2b-bazel-runner/src/bin/d2b-bazel-runner.rs`. (FR-006, FR-052)
- [ ] T033 [US1] On the integrated W1 candidate, run the six targets from `Makefile`, validate the pass, fail, and interruption manifests against `docs/reference/schemas/test-execution-manifest-v1.json`, and confirm Cargo remains authoritative; W1 does not close until the US2 and US3 carrier tasks and T078 complete. (FR-005, FR-007, FR-011, FR-046)

**Checkpoint**: The US1 interface is complete on the W1 integration candidate, but the W1 PR remains blocked on US2 and US3.

---

## Phase 4: User Story 2 - Preserve Exact Coverage and Test Topology (Priority: P1)

**Goal**: Prove total and unambiguous eighteen-surface coverage, generator-derived censuses, companion discovery, dual-mode binary and fixture location, the three committed process topologies, and the runner's per-case evidence contract.

**Independent Test**: Run both halves of the coverage guard and every topology carrier against positive inputs and planted omissions, duplicates, empty discoveries, stale binaries, missing runfiles entries, redaction leaks, filesystem faults, and scheduling mutations.

- [ ] T034 [P] [US2] Add failing coverage-guard tests in `packages/d2b-bazel-runner/tests/coverage_map.rs` for a missing, duplicate, or added ID; an ID with an empty carrier set; a carrier claimed by more than one ID; a multiply claimed or unclaimed Rust test; a missing topology or census; an unlisted hand-written fragment including the channel transition rule, the `rustdoc_json` rule, and the vendor repository rule; a scan-manifest mismatch; an out-of-census entry with no reason; and generated-build digest drift; assert that the test itself never invokes `bazel query` and never starts a nested server. (FR-008, FR-009, SC-001)
- [ ] T035 [P] [US2] Add failing main and guest topology tests in `packages/d2b-bazel-runner/tests/topology.rs` that require one fresh process per exact libtest case, exact per-binary and ignored-case census, ignored-not-passed reporting, shell-free spawn, and budget-bounded concurrency. (FR-012, FR-023, SC-004)
- [ ] T036 [P] [US2] Add failing broker topology tests in `packages/d2b-bazel-runner/tests/broker_topology.rs` that require one process per binary, bounded `--test-threads`, `exclusive` on the default, layer1, and fake-backends carriers, no overlap with any other test, and a mutation that removes exclusivity. (FR-013, FR-052)
- [ ] T037 [P] [US2] Add failing companion-discovery tests in `packages/d2b-bazel-runner/tests/companions.rs` requiring the doctest and executed harness-free sets to equal the generator-derived census for the selector the Cargo gate uses, requiring each out-of-census manifest entry to carry a reason, and planting an empty discovery, a missing companion target, and a census taken from a manifest count rather than the executed selector. (FR-014, SC-004)
- [ ] T038 [P] [US2] Add failing locator tests in `packages/d2b-test-locator/tests/locator.rs` and `packages/d2b-bazel-runner/tests/binary_identity.rs` for absent, non-executable, stale, and wrong-identity providers; single-shot mode selection; a Bazel-mode runfiles miss failing while naming the expected runfiles path and never falling back to the Cargo arm; the absence of any absolute execution-root path under either executor; and the planted stale-binary negative fixture that places an out-of-date executable at the Cargo path, removes the runfiles entry, runs under Bazel, and must fail. (FR-018, FR-052)
- [ ] T039 [P] [US2] Add failing runner-environment tests in `packages/d2b-bazel-runner/tests/runner_env.rs` for runfiles resolution of the child test binary, forwarding only the declared test environment rather than the wrapper's incidental host environment, and one fresh directory per case beneath `TEST_TMPDIR`. (FR-012, FR-054)
- [ ] T040 [P] [US2] Add failing per-case result-content tests in `packages/d2b-bazel-runner/tests/junit_content.rs` requiring one case element per enumerated case in the document written to `XML_OUTPUT_FILE`, explicit passed, failed, and ignored outcomes, and only stable case name, outcome, bounded duration, and bounded sanitized failure text; add the committed planted fixture that first asserts every member of the canonical forbidden set is present in the unredacted fixture and then requires every one absent from the document bytes, with raw output recoverable only from the planted `test.log` path. (FR-054, SC-012)
- [ ] T041 [P] [US2] Add failing result-file filesystem tests in `packages/d2b-bazel-runner/tests/junit_fsops.rs` behind an injectable filesystem trait for anchored close-on-exec `TEST_TMPDIR` and output-parent descriptors, symlink and magic-link parent refusal, refusal of an existing case directory, refusal of an anchored `..` escape, bounded temporary-name `EEXIST` retries that unlink nothing when creation never succeeded, buffer advancement after a short write, bounded `EINTR` and `EAGAIN` retries, `ENOSPC` failure, temporary-only `unlinkat` on every terminal post-creation error, descriptor-relative `renameat`, sync before rename, close-on-exec on every opened descriptor, and no output descriptor opened before every child is reaped; each property carries a planted mutation the test must reject. (FR-054, FR-052, SC-012)
- [ ] T042 [P] [US2] Add the two failing publication-outcome tests in `packages/d2b-bazel-runner/tests/junit_outcome.rs`: one starts from an all-passing case set and forces publication failure, requiring a nonzero carrier result; the other starts from a planted test failure and forces publication failure, requiring the original test failure and exit classification to remain primary while the publication failure is reported as an additional bounded runner error. (FR-054, SC-012)
- [ ] T043 [US2] Implement exact coverage-map parsing and the split guard in `packages/d2b-bazel-runner/src/coverage.rs`, export it from `packages/d2b-bazel-runner/src/lib.rs`, and define the carrier rule in `bazel/carriers/coverage.bzl` so that mapped-label existence is proved by real `deps` and `data` edges at analysis time while census, cardinality, topology, and fragment listing are proved inside the test; T074 alone wires `//ci/rust:coverage_map_guard` in `ci/rust/BUILD.bazel`. (FR-008, FR-009, SC-001)
- [ ] T044 [US2] Populate exactly the eighteen ordered rows and no fixture-backed IDs in `tests/golden/bazel-rust-coverage.json`, including the carrier set with its verdict owner, slice, Cargo baseline, derived census and out-of-census reasons, topology, transitive tests, hand-written fragments, binary providers with runfiles paths and identities, locator-file dispositions, deliberate differences, and generated BUILD digest. (FR-007, FR-008, SC-001)
- [ ] T045 [US2] Add the committed drift-checked graph query result at `tests/golden/bazel-rust-query.json`, implement the out-of-test completeness and query-drift checks in `Makefile` and the existing `test-drift` plumbing so they consume it as a declared or committed input, and prove an unmapped Rust test target and a stale query result each fail there rather than inside any Bazel test. (FR-009, SC-001)
- [ ] T046 [US2] Implement main and guest process-per-case execution, exact `--list` parsing, faithful ignored-case handling, and `D2B_RUST_BUDGET` concurrency in `packages/d2b-bazel-runner/src/topology.rs`; do not expose a second budget control. (FR-012, FR-023, SC-004)
- [ ] T047 [US2] Implement broker process-per-binary execution and bounded threads in `packages/d2b-bazel-runner/src/topology.rs`, and define exclusive broker carrier metadata in `bazel/carriers/broker.bzl`; T074 alone marks `//ci/rust:broker_default`, `//ci/rust:broker_layer1`, and `//ci/rust:broker_fakebackends` in `ci/rust/BUILD.bazel`. (FR-013)
- [ ] T048 [US2] Implement the child environment contract in `packages/d2b-bazel-runner/src/runner_env.rs`: derive each child environment from the Bazel test environment, resolve the test binary through runfiles, forward only the declared test environment, and give each case its own directory beneath `TEST_TMPDIR`. (FR-012, FR-054)
- [ ] T049 [US2] Implement the per-case result writer in `packages/d2b-bazel-runner/src/junit.rs` and the injectable filesystem layer in `packages/d2b-bazel-runner/src/fsops.rs`, covering the canonical redaction set, raw output left only in `test.log`, anchored close-on-exec descriptors, link and escape refusal, bounded creation and write loops, ownership-limited `unlinkat`, sync before descriptor-relative `renameat`, reap-before-open ordering, enforcing publication, and test-failure precedence over publication failure. (FR-054)
- [ ] T050 [US2] Implement both locator arms and single-shot mode selection in `packages/d2b-test-locator/src/lib.rs`, with the Cargo arm expanding at the call site in the calling test crate, the Bazel arm resolving declared runfiles, no chaining between arms, and existence, executability, and identity assertions before use. (FR-018)
- [ ] T051 [US2] Migrate all 25 first-party files under `packages/` that locate binaries through `env!("CARGO_BIN_EXE_...")` to the locator, declare each located binary as `data` on its test target, and record each file's disposition in `tests/golden/bazel-rust-coverage.json`; every migrated file must stay green on the Cargo path. (FR-018, SC-004)
- [ ] T052 [US2] Migrate all 20 first-party test files under `packages/` that resolve `CARGO_MANIFEST_DIR`, including the 11 that use a `repo_root()` helper, so fixture reads become declared data through the locator and any check that needs the repository inventory consumes the generated drift-checked manifest as a declared input; record each file's disposition, and record any file needing no migration together with its reason. (FR-018, FR-015)
- [ ] T053 [P] [US2] Extend generator emission in `packages/xtask/src/bazel.rs` for main, guest, broker, doctest, and executed harness-free targets and for the walker target built from the fourth hub, rejecting an empty companion set; do not edit generated `packages/**/BUILD.bazel`, `tests/tools/no-bash-ast-walker/BUILD.bazel`, or `bazel/generated/**`, which T074 alone regenerates after scope integration. (FR-010, FR-012, FR-013, FR-014)
- [ ] T054 [US2] Implement executable existence, mode, freshness, and identity checks in `packages/d2b-bazel-runner/src/topology.rs`, and record each expected provider label, runfiles path, and identity in `tests/golden/bazel-rust-coverage.json`. (FR-018)
- [ ] T055 [US2] Run the exact Bazel and Cargo test, ignored, doctest, executed harness-free, and target censuses, confirm the four `fuzz`-gated `packages/d2b-core` entries and the `packages/d2b-zone-routing` bench entry are recorded as out of census with reasons, and store only immutable references under `.scratch/adr052-w1-census/`; every mismatch or empty set must fail the owning carrier. (FR-012, FR-014, SC-004)
- [ ] T056 [US2] Prove each of the three broker carriers stays exclusive by running the scheduler overlap mutation from `packages/d2b-bazel-runner/tests/broker_topology.rs`, and record that exclusive carriers run one at a time after the parallel phase; reserve the twenty-consecutive-pass evidence for W4. (FR-013, SC-005)
- [ ] T057 [US2] Add generated-target and coverage-digest assertions in `packages/d2b-bazel-runner/tests/generated_targets.rs`, prove they fail against stale W1 generator output, and hand the required regeneration inputs to T074 without editing integrator-owned `packages/**/BUILD.bazel`, `bazel/generated/**`, `.bazelignore`, `ci/rust/BUILD.bazel`, or the final digest in `tests/golden/bazel-rust-coverage.json`. (FR-003, FR-009)

**Checkpoint**: Total coverage, exact topology, dual-mode location, and per-case evidence are enforced on the W1 candidate.

---

## Phase 5: User Story 3 - Keep Policy and Supply-Chain Checks Enforcing (Priority: P1)

**Goal**: Preserve offline dependency, advisory, license, source, API, no-bash, schema, stub, and pinned-inventory outcomes across all three policy locks, with the nightly census reached by a per-target transition and the vendored tree materialized from pinned downloads.

**Independent Test**: Run every policy carrier with no action network, compare its enforcing outcomes with Cargo, and prove each guard rejects a planted violation.

- [ ] T058 [P] [US3] Add failing offline supply-chain tests in `bazel/supply_chain/BUILD.bazel` and `packages/d2b-bazel-runner/tests/supply_chain.rs` for three `cargo-deny bans licenses sources` carriers, three `cargo-audit --no-fetch` carriers, all three policy locks, exact workspace-scoped advisory ignores, zero action network, vendor-tree classification refusals for a mirror source and a checksum-less non-git entry, a materialized package count unequal to the lock's, and differing-union findings. (FR-019, FR-020, FR-021, SC-011)
- [ ] T059 [P] [US3] Add failing no-bash carrier tests in `tests/tools/no-bash-ast-walker/src/main.rs` and `packages/d2b-bazel-runner/tests/no_bash_carrier.rs` for equality among `bazel/generated/governed-rust-sources.bzl`, declared runfiles, and successfully parsed files, plus unreadable, unparsable, omitted, extra, and planted `Command::new("bash")` inputs, with the walker built from the fourth `crate_universe` hub rather than a re-resolved dependency set. (FR-002, FR-015, FR-017, FR-022)
- [ ] T060 [P] [US3] Add failing schema carrier tests in `packages/xtask/src/schema.rs` and `packages/d2b-bazel-runner/tests/schema_carrier.rs` for two sequential independent generations, the exact generated emitted and on-disk sets, nonempty valid JSON, set-difference diagnostics before digest comparison, an empty-tree mutation, and a mutation that substitutes a hand-written count for the generated census. (FR-015, FR-016, FR-022)
- [ ] T061 [P] [US3] Add failing API census tests in `packages/d2b-bazel-runner/tests/api_census_carrier.rs` and `bazel/rules/tests/` for the nightly channel applying only to the census subgraph, declared per-crate JSON outputs, the emitted toolchain version differing from `packages/d2b-api-surface/rust-toolchain.toml`, a golden diff against `tests/golden/api-surface`, and a guard that fails closed when any `.bazelrc` line or wrapper argument sets `@rules_rust//rust/toolchain/channel`. (FR-004, FR-022, SC-004)
- [ ] T062 [P] [US3] Add failing pinned-inventory and stub controls in `packages/d2b-bazel-runner/tests/inventory_stub_carriers.rs` for missing or extra pinned tests, empty listings, socket creation, a missing executable, and a wrong binary identity. (FR-018, FR-022)
- [ ] T063 [US3] Implement the repository-owned per-target channel transition in `bazel/rules/channel_transition.bzl`, setting `@rules_rust//rust/toolchain/channel` to nightly over the API census subgraph only, inside the single Bazel invocation, and list it as a hand-written fragment for `tests/golden/bazel-rust-coverage.json`. (FR-001, FR-004)
- [ ] T064 [US3] Implement the repository-owned `rustdoc_json` rule in `bazel/rules/rustdoc_json.bzl`, invoking the resolved nightly rustdoc from the registered toolchain with JSON output, declaring one JSON output per crate, and declaring the toolchain version the action actually used as an additional output that the guard compares to the committed pin; T074 alone reconciles its labels into `ci/rust/BUILD.bazel`. (FR-004, FR-022)
- [ ] T065 [US3] Implement the global-channel-flag refusal guard in `packages/xtask/tests/policy_ci.rs` over `.bazelrc` and `Makefile`, and observe it failing against a planted global flag before it lands. (FR-004, FR-052)
- [ ] T066 [US3] Implement the vendor repository rule in `bazel/vendor/defs.bzl` and `bazel/vendor/BUILD.bazel`: classify every lock entry as a first-party path dependency, a default-index registry package with a checksum, or the single pinned git source and refuse anything else by name; re-declare each registry crate with `ctx.download` using its registry URL and the lock's checksum; extract and write `.cargo-checksum.json` as `{"files":{},"package":"<sha256>"}`; fetch `wl-proxy` by pinned rev plus a committed archive sha256 cross-checked against the `outputHashes` pin in `flake.nix`, with `"package": null` and the matching source-replacement entry; and assert the materialized package count equals the lock's before any policy tool runs. Repository-rule fetch is permitted and pinned; no action opens a socket. (FR-019, SC-011)
- [ ] T067 [US3] Implement the three offline `cargo-deny` carriers in `bazel/supply_chain/defs.bzl` and `bazel/supply_chain/BUILD.bazel`, consuming the T066 vendored tree and a generated `.cargo/config.toml`, setting `CARGO_NET_OFFLINE=1`, and enforcing only bans, licenses, and sources; T074 alone reconciles their labels into `ci/rust/BUILD.bazel`. (FR-019)
- [ ] T068 [US3] Implement three pinned-RustSec `cargo-audit --no-fetch` carriers and the exact main, broker, and guest ignore scopes in `bazel/supply_chain/BUILD.bazel` and the advisory pin under `bazel/supply_chain/`; permit no network-capable action and leave `ci/rust/BUILD.bazel` reconciliation to T074. (FR-019, FR-021)
- [ ] T069 [US3] Implement no-bash source, runfiles, and parsed-set equality plus parsed-count reporting in `tests/tools/no-bash-ast-walker/src/main.rs` and `bazel/carriers/no_bash.bzl`, consuming the generator-owned `bazel/generated/governed-rust-sources.bzl` without editing it; T074 performs the sole regeneration. (FR-015, FR-017)
- [ ] T070 [US3] Implement schema reproducibility as one sequential two-generation action in `bazel/carriers/schema.bzl`, consuming `gen-schemas --out-dir .scratch/schema-generation` and the generated census from `tests/golden/bazel-rust-coverage.json`. (FR-015, FR-016)
- [ ] T071 [P] [US3] Implement the pinned-test inventory and stub-no-socket carriers in `bazel/carriers/inventory_stub.bzl`, preserving independent verdicts and binary identity through the locator; generator-owned `packages/**/BUILD.bazel` updates occur only in T074. (FR-010, FR-018, FR-022)
- [ ] T072 [US3] Record the enforcing-outcome comparison between today's `cargo deny check` with no subcommand list and the decomposed `cargo-deny` plus `cargo-audit` pair for `packages/Cargo.lock`, `packages/d2b-priv-broker/Cargo.lock`, and `packages/d2b-guest-shell-runner/Cargo.lock` under `.scratch/adr052-w1-supply-chain/`; any differing enforcing outcome blocks W1. (FR-020, SC-011)
- [ ] T073 [US3] If and only if T072 records a yanked-state difference, land the pre-authorized yanked carrier: a committed lock-bounded snapshot under `bazel/supply_chain/`, an `xtask` subcommand in `packages/xtask/src/bazel.rs` that refreshes it only during an explicit reviewed networked update outside the gate, and an offline drift check proving exact `(name, version)` key-set equality with the three committed locks, reporting under the existing `rust-deny-main`, `rust-deny-broker`, and `rust-deny-guest` identifiers and adding no nineteenth surface. (FR-020, FR-021, SC-011)
- [ ] T074 [US3] Commit every W1 scope before validation with qualified `spec003w1` tags, stage only scope-owned paths from `specs/003-adr052-bazel-rust/plan.md`, reconcile all disjoint carrier modules solely in integrator-owned `ci/rust/BUILD.bazel`, run `cargo xtask gen-bazel` once to update solely integrator-owned generated `packages/**/BUILD.bazel`, `tests/tools/no-bash-ast-walker/BUILD.bazel`, `bazel/generated/**`, and `.bazelignore`, refresh the final generated-build digest and the committed query result in `tests/golden/bazel-rust-coverage.json` and `tests/golden/bazel-rust-query.json`, and merge all W1 tips into the integration branch.
- [ ] T075 [US3] Add the W1 semantic release note to `changelog.d/adr052-bazel-rust-shadow.md`, describing the shadow aggregate and exact coverage without process markers and without claiming promotion.
- [ ] T076 [US3] Validate the committed W1 tree with all six Bazel targets in `Makefile`, a schema-valid `D2B_EXECUTION_MANIFEST=.scratch/adr052-w1-manifest.json`, `make test-rust`, `make test-policy`, `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, and `make test-drift`; observe every planted failure, including the stale-binary locator fixture, the redaction fixture, the global-channel-flag guard, and the vendor-classification refusals, before restoring the tree. (FR-051, FR-052, FR-053)
- [ ] T077 [US3] Run the W1 integrated-diff panel over the committed W1 range using all ten roles with the Bazel and `rules_rust` expert in the `software` seat, supply T076 evidence from `.scratch/adr052-w1-validation/`, require unanimous empty recommendations, and rerun the full roster after any content change.
- [ ] T078 [US3] Open the W1 PR to protected `v3`, wait for required checks on the stable head, seal `spec003w1` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge every W1 scope, verify Cargo remains the required `test-rust` path, run `nix-collect-garbage`, remove finished W1 worktree `packages/target/` and `.scratch/bazel/` trees, and verify the merged `tests/golden/bazel-rust-coverage.json` still has exactly eighteen rows.

**Checkpoint**: W1 is merged and sealed. The complete Bazel gate exists beside authoritative Cargo.

---

## Phase 6: User Story 4 - Get Faster, Bounded Local Feedback (Priority: P2)

**Goal**: Add bounded scheduling, scratch and cache limits, wrapper-supplied absolute startup options, synchronous on-demand trimming, safe cleanup, deadline and process control, exact recovery messages, and temporary cold-local preparation.

**Independent Test**: Exercise every positive case and every cleanup, descriptor, race, deadline, escalation, sibling, startup-option, trim, cache, message, and no-shell mutation on the W2 candidate.

- [ ] T079 [US4] Run the W2 plan panel against the W2 section of `specs/003-adr052-bazel-rust/plan.md`, `specs/003-adr052-bazel-rust/contracts/recovery-deadline.md`, and `specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md`, require unanimous ten-role signoff with the Bazel and `rules_rust` expert in the `software` seat, and stage only classification metadata under `.scratch/adr052-w2-plan-panel/`.
- [ ] T080 [US4] Land a W2 prep commit that fixes disjoint ownership in `packages/d2b-bazel-runner/src/{budget,cleanup,deadline,process,recovery}.rs`, `packages/d2b-bazel-runner/tests/`, `Makefile`, `.bazelrc`, `packages/xtask/src/bazel_evidence.rs`, `packages/xtask/src/main.rs`, and `packages/d2b-contract-tests/tests/policy_docs.rs` before opening separate worktrees.
- [ ] T081 [P] [US4] Add failing budget and scratch tests in `packages/d2b-bazel-runner/tests/budget.rs` for valid and invalid `D2B_RUST_BUDGET`, combined scheduler and suite concurrency, action-cache 8 GiB and 14-day bounds, the 2 GiB repository-cache bound, the 20 GiB output-root warning, and the 40 GiB pre-build refusal. (FR-023, FR-024, FR-025)
- [ ] T082 [P] [US4] Add cleanup positive and planted negative tests in `packages/d2b-bazel-runner/tests/cleanup.rs` for descriptor-relative removal, both the `openat2` and forced component-walk routes, symlink, magic link, escape, tracked file, live server, replacement race, external decoy survival, and refusal before deletion. (FR-026, FR-027, FR-051, FR-052)
- [ ] T083 [US4] Add descriptor inheritance mutations in `packages/d2b-bazel-runner/tests/cleanup.rs` after T082 for a missing `O_CLOEXEC` on the anchor, the traversal descriptor, and the enumeration reopen; each planted child must expose the leaked descriptor through `/proc/self/fd` and fail. (FR-027, FR-052)
- [ ] T084 [P] [US4] Add deadline parser and rounding tests in `packages/d2b-bazel-runner/tests/deadline.rs` for accepted integer and fractional ASCII fields; missing, empty, signed, exponent, second-separator, non-ASCII, trailing, and overflow input; capture truncation, read round-up, child round-down, the absent local default, expired `None` or zero, and no disclosure of a rejected value. (FR-039, FR-040, FR-041, FR-052)
- [ ] T085 [P] [US4] Add fake-backend escalation-order and real-process tests in `packages/d2b-bazel-runner/tests/process.rs` for dedicated group creation, SIGTERM, the full grace despite leader exit, `EXITED|NOWAIT|NOHANG`, unconditional group SIGKILL, reap last, descendant death, sibling survival, server-PID decoy survival, stuck bounded shutdown, and the five required signal and ordering mutations. (FR-042, FR-043, FR-052)
- [ ] T086 [P] [US4] Add table-driven recovery tests in `packages/d2b-bazel-runner/tests/recovery.rs` for every cleanup, server, and deadline code, the exact code-specific remedy, path, hash, user, PID, raw-deadline, and handle redaction, no recursive-removal instruction, no cross-code remedy, and every required wrong-message mutation. (FR-028, FR-029, FR-044, FR-052, SC-012)
- [ ] T087 [P] [US4] Extend source-shape tests in `packages/d2b-contract-tests/tests/policy_docs.rs` to require descriptor-relative cleanup and reject a planted path-based recursive-removal mutation, leaving `O_CLOEXEC` proof exclusively in behavioral tests. (FR-026, FR-027, FR-051)
- [ ] T088 [P] [US4] Add startup-option identity tests in `packages/d2b-bazel-runner/tests/startup_options.rs` proving `.bazelrc` carries no `startup` line, that the wrapper supplies every startup option as an absolute worktree-derived path, that `build`, `test`, `query`, `info`, `shutdown`, and `clean` receive byte-identical startup options, that `--symlink_prefix` is absolute and points beneath `.scratch/`, and that a mutation perturbing one command's options fails closed. (FR-024, FR-055, FR-052)
- [ ] T089 [P] [US4] Add synchronous trim tests in `packages/d2b-bazel-runner/tests/disk_trim.rs` proving the explicit on-demand collector is invoked as a named step, that its completion is observed before any size measurement, that a measurement taken before completion is rejected, and that a mutation relying only on idle-time collection fails closed. (FR-024, FR-033, FR-052)
- [ ] T090 [US4] Implement memory-aware budget propagation and scratch and cache high-water checks in `packages/d2b-bazel-runner/src/budget.rs`, `Makefile`, and `.bazelrc`, preserving `D2B_RUST_BUDGET` as the only budget and refusing build work at the hard limit. (FR-023, FR-024, FR-025)
- [ ] T091 [US4] Implement anchored cleanup in `packages/d2b-bazel-runner/src/cleanup.rs` with one `.scratch/` anchor, no symlink, magic-link, or escape traversal, close-on-exec descriptors, descriptor-relative enumeration and unlink, tracked and live refusal, race resistance, and no access outside `.scratch/bazel/`. (FR-026, FR-027)
- [ ] T092 [US4] Implement checked `/proc/uptime` parsing and absolute-deadline conversion in `packages/d2b-bazel-runner/src/deadline.rs`, with conservative rounding, missing-local behavior, expired-budget classification, and no raw input echo. (FR-039, FR-040, FR-041)
- [ ] T093 [US4] Implement shell-free dedicated-process-group execution and bounded shutdown in `packages/d2b-bazel-runner/src/process.rs`, preserving the exact escalation order and never signalling group zero, group -1, the caller group, an unrelated sibling, or a server PID read from a file. (FR-042, FR-043)
- [ ] T094 [US4] Implement the exact static recovery table in `packages/d2b-bazel-runner/src/recovery.rs` for `D2B-BZLCLEAN-TRACKED`, `D2B-BZLCLEAN-SYMLINK`, `D2B-BZLCLEAN-ESCAPE`, `D2B-BZLCLEAN-LIVE`, `D2B-BZLSERVER-STUCK`, expired budget, and ceiling miss. (FR-028, FR-029, FR-044)
- [ ] T095 [US4] Implement absolute startup-option construction and the absolute `--symlink_prefix` beneath `.scratch/` in `Makefile` and `packages/d2b-bazel-runner/src/process.rs`, keeping `.bazelrc` free of `startup` lines and reusing byte-identical options for every Bazel command the wrapper issues. (FR-024, FR-055)
- [ ] T096 [US4] Implement the synchronous on-demand disk-cache trim step in `Makefile` and `packages/d2b-bazel-runner/src/budget.rs`, invoking the pinned upstream collector or a pinned repository-owned equivalent and observing completion before any size measurement, and cite the plan Constraints rule that a Bazel version bump reopens this design review in the W2 PR body. (FR-024, FR-033)
- [ ] T097 [US4] Add a failing cold-local preparation test in `packages/xtask/src/bazel_evidence.rs`, then implement only `cargo xtask bazel-evidence prepare-cold-local` through `packages/xtask/src/main.rs` so it creates a fresh output user root and empty action cache while retaining the populated repository cache; add no Make target and no environment contract. (FR-024, FR-037)
- [ ] T098 [US4] Wire `make clean` and `make bazel-shutdown` in `Makefile` through `packages/d2b-bazel-runner/`, preserving `D2B_CLEAN_DRY_RUN`, `D2B_CLEAN_KEEP_SCRATCH`, and byte-identical Bazel startup options, and prove no shell is used for repository-owned process control. (FR-026, FR-028, FR-043)
- [ ] T099 [US4] Add the W2 semantic release note to `changelog.d/adr052-bazel-local-safety.md`, describing bounded scratch and safe recovery without local identifiers or process markers.
- [ ] T100 [US4] Commit every W2 scope before validation with qualified `spec003w2` tags, stage only W2-owned paths from `specs/003-adr052-bazel-rust/plan.md`, and merge the committed scope tips into the W2 integration branch.
- [ ] T101 [US4] Validate the committed W2 tree with `make test-rust-main`, `make test-policy`, `make check-tier0`, `make test-bazel-rust`, and `D2B_CLEAN_DRY_RUN=1 make clean` from `Makefile`; run every planted cleanup, descriptor, race, deadline, process, recovery, startup-option, trim, local-cache-bound, and no-shell mutation and require the expected failure. (FR-051, FR-052, FR-053, SC-012)
- [ ] T102 [US4] Run the W2 integrated-diff panel over the committed W2 range using all ten roles with the Bazel and `rules_rust` expert in the `software` seat, supply T101 evidence from `.scratch/adr052-w2-validation/`, require unanimous empty recommendations, and rerun all roles after any content change.
- [ ] T103 [US4] Open the W2 PR to protected `v3`, wait for required checks on the stable head, seal `spec003w2` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge every W2 scope, run `nix-collect-garbage`, and remove finished W2 worktree `packages/target/` and `.scratch/bazel/` trees.

**Checkpoint**: W2 is merged and sealed. Local operation is bounded and every safety invariant has positive and mutation coverage.

---

## Phase 7: User Story 5 - Compare Safely in CI and Qualify Evidence (Priority: P2)

**Goal**: Land cache-free non-required shadow CI in W3 together with qualification-record capture and the cold-CI feasibility measurement, then create the immutable W4 qualification record from exact equivalence, failure, topology, locator, performance, supply-chain, and safety evidence.

**Independent Test**: Reject all noncompliant workflow fixtures, inspect one real shadow dispatch, prove only push events on protected `v3` produce records, and validate every qualification threshold and immutable reference.

### W3 - Shadow CI and feasibility

- [ ] T104 [US5] Run the W3 plan panel against the W3 section of `specs/003-adr052-bazel-rust/plan.md`, `specs/003-adr052-bazel-rust/contracts/cache-workflow-boundaries.md`, and `specs/003-adr052-bazel-rust/contracts/shadow-promotion-evidence.md`, require unanimous ten-role signoff with the Bazel and `rules_rust` expert in the `software` seat, and stage only classification metadata under `.scratch/adr052-w3-plan-panel/`.
- [ ] T105 [US5] Create disjoint W3 worktrees from merged W2: `shadow-workflow` owns `.github/workflows/pr-bazel-rust.yml`; `workflow-policy` owns `packages/xtask/tests/policy_ci.rs` and `packages/xtask/tests/fixtures/ci/`; the integrator alone owns trigger, path-filter, and allowlist reconciliation recorded in `specs/003-adr052-bazel-rust/plan.md`.
- [ ] T106 [P] [US5] Add positive and rejecting workflow fixtures under `packages/xtask/tests/fixtures/ci/` for the cache-free shadow workflow, a restore-only pull-request job alongside a writer restricted to pushes on protected `v3`, a direct `actions/cache` post-save, `actions/cache/save`, a saving `Swatinem/rust-cache`, indirect and unknown writers, a missing promoted deadline control, pull-request job-level `actions: write`, and workflow-level `actions: write`; first observe policy-test failure. (FR-030, FR-031, FR-035, FR-039, FR-051, FR-052, SC-009, SC-012)
- [ ] T107 [US5] Implement fail-closed YAML reachability, writer recognition, permission, approved-target, deadline, and `V3_PR_GATE_WORKFLOWS` exclusion checks in `packages/xtask/tests/policy_ci.rs`, including direct, indirect, post-step, and unknown writer detection. (FR-006, FR-030, FR-031, FR-051)
- [ ] T108 [P] [US5] Create `.github/workflows/pr-bazel-rust.yml` with four slice jobs and one attributed rollup, approved Make targets only, credentialless checkout, `contents: read`, no cache restore, save, or publication, no `actions: write`, non-required status, and the amended ADR's protected-`v3` push, path-filtered pull-request, and manual triggers, with no schedule on the repository default branch. (FR-006, FR-030, FR-031, FR-032)
- [ ] T109 [US5] Add qualification-record capture to `.github/workflows/pr-bazel-rust.yml` for push events on `refs/heads/v3` produced by merged pull requests, recording the head commit shared with the required Cargo workflow run, both run identifiers, both rollup verdicts, the same-commit fixture-contract verdict reference, four slice verdicts and durations, manifest references, effective permissions, and zero cache writes; pull-request runs must record that they are diagnostic and produce no record, and no credential or attestation payload is stored. (FR-030, FR-032, FR-045)
- [ ] T110 [US5] Record the cold continuous-integration feasibility measurement from the first complete push-to-`v3` shadow run under `.scratch/adr052-w3-feasibility/`, capturing all four slice durations on the real runner class; if the 15-minute median and 18-minute maximum are not attainable, name which pre-authorized remedy is being taken, a larger runner class or a further disjoint slice split, and do not treat the ceiling as binding until this measurement exists. (FR-037, FR-044, SC-008)
- [ ] T111 [US5] Add the W3 semantic release note to `changelog.d/adr052-bazel-shadow-ci.md`, describing optional cache-free Bazel shadow CI without process markers or required-context claims.
- [ ] T112 [US5] Commit every W3 scope before validation with qualified `spec003w3` tags, stage only W3-owned paths from `specs/003-adr052-bazel-rust/plan.md`, and merge the committed scope tips into the W3 integration branch.
- [ ] T113 [US5] Validate the committed W3 tree with `make test-rust-main`, `make test-policy`, `make test-lint`, `make check-tier0`, and all four Bazel slice targets from `Makefile`; require every fixture in `packages/xtask/tests/fixtures/ci/` to produce its expected accept or reject verdict. (FR-030, FR-031, FR-051, FR-052, FR-053)
- [ ] T114 [US5] Open a draft W3 PR to protected `v3` and inspect its `.github/workflows/pr-bazel-rust.yml` `pull_request` run, proving four attributed slices, non-required status, approved Make-only execution, credentialless checkout, only `contents: read`, no cache action or writer, zero publication, and that the run is recorded as diagnostic rather than as a qualification record.
- [ ] T115 [US5] Run the W3 integrated-diff panel over the committed W3 range using all ten roles with the Bazel and `rules_rust` expert in the `software` seat, supply T110, T113, and T114 evidence from `.scratch/adr052-w3-validation/`, require unanimous empty recommendations, and rerun all roles after any content change.
- [ ] T116 [US5] Mark the draft W3 PR ready, wait for required checks on the stable head, seal `spec003w3` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge every W3 scope, run `nix-collect-garbage`, and remove finished W3 worktree artifacts.

### W4 - Immutable Qualification Evidence

- [ ] T117 [US5] Run the W4 plan panel against the W4 section of `specs/003-adr052-bazel-rust/plan.md` and `specs/003-adr052-bazel-rust/contracts/shadow-promotion-evidence.md`, require unanimous ten-role signoff with the Bazel and `rules_rust` expert in the `software` seat, and stage only classification metadata under `.scratch/adr052-w4-plan-panel/`.
- [ ] T118 [US5] Create one W4 curator worktree whose only tracked owned path is `specs/003-adr052-bazel-rust/evidence/qualification.json`; all measurement scratch, logs, manifests, and transcripts remain untracked under `.scratch/adr052-w4/`.
- [ ] T119 [P] [US5] Collect ten consecutive matching qualification records under `.scratch/adr052-w4/records/`, each a push on `refs/heads/v3` produced by a merged pull request with one head commit shared by both runs, matching Cargo and Bazel rollup verdicts, and a passing same-commit `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` verdict; apply the reset rules, treating a Bazel run that reached no verdict beside a Cargo run that did as a mismatch, and treating a push where neither reached a verdict as no record at all; reject pull-request, `main`-push, scheduled, and dispatched runs. (FR-045, SC-002)
- [ ] T120 [P] [US5] Execute eighteen single-invariant seeded-failure branches and record under `.scratch/adr052-w4/seeded-failures/` that each owning carrier alone fails and emits a schema-valid partial v1 manifest with no unrelated failed surface. (FR-022, FR-045, SC-003, SC-013)
- [ ] T121 [P] [US5] Collect exact Cargo and Bazel test, ignored, doctest, executed harness-free, API, schema, scanner, pinned-inventory, and binary-provider censuses plus main and guest topology and per-case result-publication proofs under `.scratch/adr052-w4/census-topology/`; reject any empty or differing set and confirm every out-of-census entry carries its reason. (FR-012, FR-014, FR-015, FR-016, FR-017, FR-018, FR-045, FR-054, SC-004)
- [ ] T122 [P] [US5] Collect the complete locator migration evidence under `.scratch/adr052-w4/locator/`: every one of the 25 binary-locating files and 20 manifest-resolving test files migrated or recorded as needing no migration with a reason, both arms green on the Cargo path, and the planted stale-binary fixture failing under Bazel with the expected runfiles path named. (FR-018, FR-045, SC-004)
- [ ] T123 [P] [US5] Run each broker feature carrier twenty consecutive times with exclusivity enforced and retain immutable result references under `.scratch/adr052-w4/broker-repetitions/`; any overlap or failed repetition invalidates the set. (FR-013, FR-045, SC-005)
- [ ] T124 [US5] Reserve one exclusive reference-host measurement window with no heavy lane and no other W4 build task active, hold it through T125, then measure three independently primed warm-local runs with no streamed test output and store commit, environment, comment-edit, live-server, flag, duration, and output-root-size references under `.scratch/adr052-w4/performance/warm-local/`; require a median at most 600 seconds and a maximum at most 720. (FR-037, FR-038, SC-006)
- [ ] T125 [US5] While retaining the exclusive T124 reference-host window, use `cargo xtask bazel-evidence prepare-cold-local` for three serial cold-local runs with no other W4 build task active and no streamed test output, store cache-state, environment, flag, duration, and output-root-size references under `.scratch/adr052-w4/performance/cold-local/`, require a median at most 900 seconds and a maximum at most 1080, then release the window. (FR-037, FR-038, SC-007)
- [ ] T126 [P] [US5] Select the five most recent qualifying cold qualification records, where qualifying means no Bazel cache of any kind was restored and all four slice jobs completed with a recorded duration, and store the head commit, both run identifiers, the four slice durations, the zero-cache references, and the T110 feasibility reference under `.scratch/adr052-w4/performance/cold-ci/`; require a median at most 900 seconds and no sample above 1080. (FR-030, FR-037, FR-038, SC-008, SC-009)
- [ ] T127 [P] [US5] Collect all-three-lock supply-chain equivalence, including the yanked-state outcome and the landed yanked carrier where T072 required one, plus zero-action-network references under `.scratch/adr052-w4/supply-chain/`, and collect positive plus every cleanup, deadline, process, recovery, per-case redaction, result-filesystem, workflow, and cache mutation verdict under `.scratch/adr052-w4/safety/`. (FR-019, FR-020, FR-021, FR-045, FR-051, FR-052, FR-054, SC-011, SC-012)
- [ ] T128 [US5] Create only `specs/003-adr052-bazel-rust/evidence/qualification.json` from T119 through T127, bind candidate-specific evidence to one integrated commit, include the immutable historical qualification-record references, compute every threshold, set `qualified` only when none is pending, and include no logs, transcripts, credentials, or attestation payloads. (FR-045, SC-001, SC-002, SC-003, SC-004, SC-005, SC-006, SC-007, SC-008, SC-009, SC-011, SC-012)
- [ ] T129 [US5] Commit `specs/003-adr052-bazel-rust/evidence/qualification.json` before validation with a qualified `spec003w4` tag, and treat any candidate-content change as invalidating affected evidence rather than editing around a failed threshold.
- [ ] T130 [US5] Validate the committed W4 record against its references, rerun `make test-bazel-rust`, `make test-rust`, `make test-policy`, and `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` from `Makefile`, and verify no other tracked file exists under `specs/003-adr052-bazel-rust/evidence/`.
- [ ] T131 [US5] Run the W4 integrated-diff panel over `specs/003-adr052-bazel-rust/evidence/qualification.json` using all ten roles with the Bazel and `rules_rust` expert in the `software` seat, supply T130 evidence from `.scratch/adr052-w4-validation/`, require unanimous empty recommendations, and keep the record immutable after signoff.
- [ ] T132 [US5] Open the W4 PR to protected `v3`, wait for required checks on the stable qualification record, seal `spec003w4` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge the qualification record and verify its digest on merged `v3`.
- [ ] T133 [US5] After the W4 merge, run `nix-collect-garbage`, remove W4 measurement worktrees and their `packages/target/` and `.scratch/bazel/` trees, and preserve only immutable references in `specs/003-adr052-bazel-rust/evidence/qualification.json`.

**Checkpoint**: W3 and W4 are merged and sealed. Qualification is immutable, and promotion may consume it only by digest.

---

## Phase 8: User Story 6 - Promote and Retire Without Breaking Contracts (Priority: P3)

**Goal**: Promote in W5 with rollback, the required future guards, and immutable records, then independently remove compatibility aliases in W6 after release containment and retire Cargo implementations, never names, in W7 after ten promoted green `v3` runs.

**Independent Test**: Block promotion on incomplete qualification, preserve the `test-rust` context and fixture mode, rehearse one-commit rollback, prove each post-promotion child consults only its own evidence clock, and prove every public Make name survives retirement.

### W5 - Promotion and Immutable Promotion Record

- [ ] T134 [US6] Run the W5 plan panel against the W5 section of `specs/003-adr052-bazel-rust/plan.md`, `specs/003-adr052-bazel-rust/evidence/qualification.json`, and the cache and evidence contracts, require unanimous ten-role signoff with the Bazel and `rules_rust` expert in the `software` seat, and stage only classification metadata under `.scratch/adr052-w5-plan-panel/`.
- [ ] T135 [US6] Land a W5 integrator prep commit that creates and registers `packages/xtask/src/cache_maintenance.rs` plus its CLI seam in `packages/xtask/src/main.rs`, then create disjoint W5 worktrees from that prep commit tip: `promotion-make` owns `Makefile` and `tests/test-rust.sh`; `promotion-manifest` owns `tests/layer1-jobs.json` and `tests/ci/layer1-workflow.template.yml`; `runner-tests` owns `packages/d2b-bazel-runner/tests/make_interface.rs`; `cache` owns `packages/xtask/src/cache_maintenance.rs`, `packages/xtask/src/bazel_evidence.rs`, `packages/xtask/src/main.rs`, `packages/xtask/tests/policy_ci.rs`, and `packages/xtask/tests/fixtures/ci/`; the integrator owns `.github/workflows/pr-bazel-rust.yml`, generated `.github/workflows/pr-l1-static-fast.yml`, and `specs/003-adr052-bazel-rust/evidence/`. Serialize T139 before T143 in the cache scope.
- [ ] T136 [P] [US6] Add failing promotion-interface tests in `packages/d2b-bazel-runner/tests/make_interface.rs` for the unchanged required context `test-rust`, all eight existing `test-rust-*` names, the four authoritative slices, Bazel-specific status-preserving aliases with one stderr replacement line, and unchanged fixture mode; workflow rejection of deprecated aliases belongs to T138. (FR-046, FR-047, SC-014)
- [ ] T137 [P] [US6] Add cache-maintenance positive and mutation tests in `packages/xtask/src/cache_maintenance.rs` and `packages/xtask/tests/fixtures/ci/cache/` for complete pagination, a failed query, an ambiguous prefix, unauthorized-entry preservation, retired and superseded-only deletion, an observed synchronous trim before measurement, separate 4 GiB action and 1 GiB repository snapshots, no output base, both 8 GiB headroom checks, one writer, read-only pull-request restore, a concurrent usage change, and a key-input change that fails to change the key. (FR-033, FR-034, FR-035, FR-036, FR-051, FR-052, SC-010, SC-012)
- [ ] T138 [P] [US6] Add promoted-deadline, alias-workflow, and future-guard tests in `packages/xtask/tests/policy_ci.rs` and `packages/xtask/tests/fixtures/ci/deadline/` for checkout timeout 2, the first post-checkout uptime anchor, `anchor_ms + 780000`, the mandatory in-band deadline on every promoted Bazel Rust job, outer timeout 17, cache credentials absent from `run:` and Bazel environments, maintenance verdict independence from `test-rust`, rejection of any workflow that invokes a deprecated Bazel alias, and the structural assertion that no `pull_request`-reachable job requests `actions: write`; observe each negative fixture failing first. (FR-031, FR-032, FR-034, FR-039, FR-040, FR-041, FR-048, FR-051, FR-052)
- [ ] T139 [US6] Implement ordered protected-`v3` cache enumeration, deletion, synchronous trim, usage checks, and save planning in `packages/xtask/src/cache_maintenance.rs` and route it through `packages/xtask/src/main.rs`; failures name only entry keys and headroom, never credentials. (FR-033, FR-034, FR-036)
- [ ] T140 [US6] Switch the eighteen baseline surfaces beneath `make test-rust` to Bazel in `Makefile`, retain the Cargo/Nix `fixture-contracts` mode in `tests/test-rust.sh`, preserve all eight existing leaf names as mappings, and convert Bazel-specific names into status-preserving aliases. (FR-046, FR-047, SC-014)
- [ ] T141 [US6] Replace the eight Rust CI leaves with four Bazel slices while preserving `ciJobId: test-rust` in `tests/layer1-jobs.json`, update `tests/ci/layer1-workflow.template.yml` only as required, regenerate `.github/workflows/pr-l1-static-fast.yml`, and ensure no workflow calls a deprecated alias. (FR-046, FR-048)
- [ ] T142 [US6] Add protected-`v3` restore, read-only pull-request restore, and one-writer promotion steps to generated `.github/workflows/pr-l1-static-fast.yml` through `tests/layer1-jobs.json` and `tests/ci/layer1-workflow.template.yml`, binding cache keys to `.bazelversion`, `MODULE.bazel`, `MODULE.bazel.lock`, `.bazelrc`, both toolchain pins, all four hub Cargo locks, `packages/Cargo.guest.lock`, all four per-hub Bazel-side locks, the `cargo-bazel` URL and sha256, all deny files, the advisory pin, the yanked snapshot when present, `.bazelignore`, the startup and symlink configuration, the build-script and action-environment digest, and the generated BUILD digest, keeping the maintenance verdict separate, the caches separate, the output base excluded, and cache credentials confined to cache actions. (FR-032, FR-033, FR-034, FR-035, FR-036)
- [ ] T143 [US6] Delete `.github/workflows/pr-bazel-rust.yml` and remove only the temporary `prepare-cold-local` command from `packages/xtask/src/bazel_evidence.rs` and `packages/xtask/src/main.rs`; preserve all qualification references and the remaining evidence validation support.
- [ ] T144 [US6] Add the W5 semantic release note to `changelog.d/adr052-bazel-rust-promotion.md`, including the Bazel alias deprecation and the rollback guarantee without process markers.
- [ ] T145 [US6] Commit every W5 implementation scope with qualified `spec003w5` tags, stage only owned paths, regenerate `.github/workflows/pr-l1-static-fast.yml`, then combine the committed scope tips into one exact W5 promotion candidate commit on the integration branch and record that SHA under `.scratch/adr052-w5-candidate/` before validation.
- [ ] T146 [US6] Validate the committed W5 promotion candidate with `make layer1-workflow`, `make test-drift`, `make check`, `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, every old and Bazel alias status test, the promoted deadline policy, both future guards from T138, and qualification digest verification from `specs/003-adr052-bazel-rust/evidence/qualification.json`; perform an explicit read-only cache API audit proving complete pagination, no ambiguous authorized prefix, no retired-writer run after the audit, and projected repository use plus planned snapshots at most 8 GiB after a synchronous trim. (FR-034, FR-036, FR-045, FR-046, FR-047, FR-051, SC-010, SC-014)
- [ ] T147 [US6] In a separate rollback worktree, revert without committing the exact single promotion candidate SHA recorded by T145, run Cargo-authoritative `make test-rust` and `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, retain only an immutable rehearsal reference under `.scratch/adr052-w5-rollback/`, and discard the worktree. (SC-015)
- [ ] T148 [US6] Run the W5 promotion integrated-diff panel over the committed W5 range using all ten roles with the Bazel and `rules_rust` expert in the `software` seat, supply T146 and T147 evidence from `.scratch/adr052-w5-validation/`, require unanimous empty recommendations, and rerun all roles after any content change.
- [ ] T149 [US6] Open the W5 promotion PR containing `Makefile` and generated `.github/workflows/pr-l1-static-fast.yml` to protected `v3`, wait for required checks on the stable head, recheck that the T146 cache audit is still fresh and rerun it on any changed cache or runtime state, seal the committed snapshot as `spec003w5` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge; the runtime record is a separate W5 follow-up.
- [ ] T150 [US6] On the first promoted protected-`v3` run, stop retired Cargo cache writes, enumerate all pages, reject query or prefix ambiguity, delete only authorized retired and superseded generations, run the synchronous trim and observe completion, verify repository use plus planned snapshots at most 8 GiB, recheck immediately before save, publish separate bounded action and download caches from exactly one writer, and record the independent `test-rust` verdict under `.scratch/adr052-w5-runtime/`. (FR-033, FR-034, FR-035, FR-036, SC-010)
- [ ] T151 [US6] Create `specs/003-adr052-bazel-rust/evidence/promotion-record.json` in a W5 follow-up worktree with the promotion SHA, the immutable qualification digest, the maintenance run, authorized deletions, the trim evidence, both headroom results, the one writer run, the first promoted verdict, and the T147 rollback reference; also initialize `specs/003-adr052-bazel-rust/evidence/post-promotion.json` with the promotion SHA, empty release tags and green run IDs, and both eligibility flags false; include no logs, credentials, transcripts, or attestation payloads.
- [ ] T152 [US6] Commit `specs/003-adr052-bazel-rust/evidence/promotion-record.json` and `specs/003-adr052-bazel-rust/evidence/post-promotion.json` before validation with a qualified `spec003w5fu1` tag, validate every immutable reference and digest, run the ten-role integrated-diff panel on the complete follow-up record range with the Bazel and `rules_rust` expert in the `software` seat, and require unanimous empty recommendations.
- [ ] T153 [US6] Open the W5 promotion-record follow-up PR, wait for required checks on the stable head, seal `spec003w5fu1` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge, run `nix-collect-garbage`, and remove finished W5 worktree artifacts.

### W6 - Independent Compatibility Alias Removal

- [ ] T154 [US6] Run the W6 plan panel against the W6 section of `specs/003-adr052-bazel-rust/plan.md` and `specs/003-adr052-bazel-rust/evidence/promotion-record.json`, require unanimous ten-role signoff with the Bazel and `rules_rust` expert in the `software` seat, and explicitly exclude the green-run clock from the decision.
- [ ] T155 [US6] Create one W6 worktree owning `Makefile`, `packages/xtask/src/delivery/eligibility.rs`, `packages/xtask/tests/policy_ci.rs`, `AGENTS.md`, `tests/README.md`, `docs/contributing/gates-and-lints.md`, `changelog.d/adr052-bazel-alias-removal.md`, and initialized `specs/003-adr052-bazel-rust/evidence/post-promotion.json`; Cargo leaf implementation files are read-only. If W7 is active but unmerged, choose one sibling to wait. If W7 merged first, rebase W6 onto current `v3` before validation, preserve all W7 fields, and rerun any invalidated validation and panel.
- [ ] T156 [US6] Update `specs/003-adr052-bazel-rust/evidence/post-promotion.json` with the promotion SHA and release tags, derive `alias_removal_eligible` only from `git tag --contains "$(jq -r .promotion_commit specs/003-adr052-bazel-rust/evidence/promotion-record.json)"`, and fail W6 if no containing release exists regardless of elapsed time, green runs, or W7 state. (FR-049)
- [ ] T157 [US6] Add failing tests and implement release-containment eligibility in `packages/xtask/src/delivery/eligibility.rs`, plus alias-policy tests in `packages/xtask/tests/policy_ci.rs`, for no containing tag, a containing release, removed workflows still naming an alias, and unchanged authoritative `test-rust-*` status. (FR-047, FR-048, FR-049)
- [ ] T158 [US6] Remove only `test-bazel-rust`, its four slice aliases, and their deprecated approved-target entries from `Makefile` and `packages/xtask/tests/policy_ci.rs`; retain `make bazel-shutdown`, every authoritative `test-rust-*` name, and the Cargo fallback implementation. (FR-047, FR-048, FR-049)
- [ ] T159 [US6] Add the W6 semantic release note to `changelog.d/adr052-bazel-alias-removal.md`, documenting only the shipped alias removal.
- [ ] T160 [US6] Commit W6 before validation with a qualified `spec003w6` tag, then run `make test-rust`, every authoritative Rust leaf target in `Makefile`, `make test-rust-main`, `make test-policy`, `make check-tier0`, and workflow absence checks for the removed aliases.
- [ ] T161 [US6] Run the W6 integrated-diff panel over the complete committed W6 range after every shared-path rebase, require unanimous empty recommendations from all ten roles with the Bazel and `rules_rust` expert in the `software` seat, open the W6 PR to protected `v3`, wait for required checks on the stable head, seal `spec003w6` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge, run `nix-collect-garbage`, and remove the finished W6 worktrees.

### W7 - Independent Cargo Implementation Retirement

- [ ] T162 [US6] Run the W7 plan panel against the W7 section of `specs/003-adr052-bazel-rust/plan.md` and `specs/003-adr052-bazel-rust/evidence/promotion-record.json`, require unanimous ten-role signoff with the Bazel and `rules_rust` expert in the `software` seat, and explicitly exclude release containment and W6 state from the decision.
- [ ] T163 [US6] Create one W7 worktree owning `tests/test-rust.sh`, `Makefile`, `packages/xtask/src/delivery/eligibility.rs`, `packages/d2b-contract-tests/tests/policy_source.rs`, `AGENTS.md`, `tests/AGENTS.md`, `tests/README.md`, `docs/contributing/gates-and-lints.md`, `changelog.d/adr052-cargo-runner-retirement.md`, and initialized `specs/003-adr052-bazel-rust/evidence/post-promotion.json`; fixture-contract files remain read-only. If W6 is active but unmerged, choose one sibling to wait. If W6 merged first, rebase W7 onto current `v3` before validation, preserve all W6 fields, and rerun any invalidated validation and panel.
- [ ] T164 [US6] Update `specs/003-adr052-bazel-rust/evidence/post-promotion.json` with ordered promoted protected-`v3` `test-rust` run IDs, derive `cargo_retirement_eligible` only from ten consecutive green runs, and fail W7 on any skipped, canceled, failed, incomparable, or pre-promotion run regardless of release tags or W6 state. (FR-050)
- [ ] T165 [US6] Add failing tests and implement ten-green-run eligibility in `packages/xtask/src/delivery/eligibility.rs`, and add source-retirement policy cases in `packages/d2b-contract-tests/tests/policy_source.rs` for nine green runs, an interrupted streak, ten valid green runs, attempted fixture-mode deletion, attempted removal of a non-migrated surface, attempted removal of the public `test-rust` name, attempted removal of any `test-rust-<leaf>` name, and a `test-rust` left carrying only the fixture leaf. (FR-050, FR-052, SC-014)
- [ ] T166 [US6] Remove only the Cargo implementations for the eighteen mapped surfaces from `tests/test-rust.sh` and unreachable Cargo-only gate plumbing; retain `make test-rust`, all eight `make test-rust-<leaf>` names forwarding to the authoritative Bazel carriers, `fixture-contracts`, `rust-contract-tests`, `rust-cli-contract-tests`, and the Bazel coverage guard. (FR-050, SC-014)
- [ ] T167 [US6] Add the W7 semantic release note to `changelog.d/adr052-cargo-runner-retirement.md`, documenting retirement of the migrated Cargo implementation while every public entry point and the fixture mode remain.
- [ ] T168 [US6] Commit W7 before validation with a qualified `spec003w7` tag, then run `make check`, `make test-rust`, all four authoritative slices through `Makefile`, `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, `make test-policy`, `make test-drift`, and an inventory proving exactly the eighteen Cargo implementations disappeared and that `make test-rust` plus all eight `make test-rust-<leaf>` names still exist and still invoke Bazel carriers.
- [ ] T169 [US6] Run the W7 integrated-diff panel over the complete committed W7 range after every shared-path rebase, require unanimous empty recommendations from all ten roles with the Bazel and `rules_rust` expert in the `software` seat, open the W7 PR to protected `v3`, wait for required checks on the stable head, seal `spec003w7` through `packages/xtask/src/delivery/`, capture the merge target and pass merge eligibility, then merge, run `nix-collect-garbage`, and remove the finished W7 worktrees.

**Checkpoint**: W5 is promoted and recorded. W6 and W7 close independently when, and only when, their own evidence gate passes.

---

## Phase 9: Final Cross-Cutting Verification

**Purpose**: Prove complete requirement coverage, scope discipline, evidence hygiene, and sealed delivery after the desired post-promotion children land.

- [ ] T170 Run `/speckit-analyze` over `specs/003-adr052-bazel-rust/spec.md`, `specs/003-adr052-bazel-rust/plan.md`, and `specs/003-adr052-bazel-rust/tasks.md`, and fail if any FR-001 through FR-055 or buildable SC-001 through SC-015 lacks a completed task and a mechanical evidence path.
- [ ] T171 Audit the final diff against `docs/adr/0052-bazel-rust-build-and-test.md` and `tests/AGENTS.md`, proving no new top-level shell gate, Layer-1 job, linter, formatter, hook, remote cache or execution, Nix, package, release, or fixture migration, host, VM, live, or hardware validation, required context, or unlisted deliberate difference was added, and proving no Bazel test invokes `bazel query` and no test action runs a nested Bazel server. (FR-001, FR-009, FR-053)
- [ ] T172 Verify `specs/003-adr052-bazel-rust/evidence/` contains only immutable `qualification.json`, immutable `promotion-record.json`, and the narrowly updated eligibility summary `post-promotion.json`, with no logs, transcripts, credentials, or attestation payloads, and verify `.scratch/adr052-*` is untracked.
- [ ] T173 Verify every completed wave is merged and sealed through `packages/xtask/src/delivery/`, both independent W5 children reflect only their own gate in `specs/003-adr052-bazel-rust/evidence/post-promotion.json`, all semantic fragments under `changelog.d/` passed `make test-changelog`, and post-merge `nix-collect-garbage` completed.

---

## Dependencies and Execution Order

### Phase and Wave Graph

```text
T001-T002 Pre-W0 amended ADR verification
  -> T003-T024 W0 foundation
  -> T025-T078 W1 coverage carriers
  -> T079-T103 W2 operational safety
  -> T104-T116 W3 shadow CI and cold feasibility
  -> T117-T133 W4 immutable qualification evidence
  -> T134-T153 W5 promotion and immutable promotion record
       -> T154-T161 W6 alias removal, gated only by release containment
       -> T162-T169 W7 Cargo retirement, gated only by ten green v3 runs
  -> T170-T173 final verification after the desired W6/W7 children
```

- T001 and T002 are hard prerequisites. The ADR amendment itself is already merged; these tasks verify it and refuse to proceed on a base that predates it.
- W0 blocks every user story.
- W1 spans US1, US2, and US3. Their disjoint implementation scopes may run in parallel after T026, but T074 through T078 integrate and close W1 only after all three stories are complete.
- Within W1, T050 precedes T051 and T052, T049 precedes T055, T043 precedes T045, T063 precedes T064, and T066 precedes T067 and T068.
- W2 depends on merged W1. W3 depends on merged W2. W4 depends on merged W3 and on ten qualification records existing. W5 depends on the immutable merged W4 digest.
- T110 must exist before the cold continuous-integration ceiling is treated as binding by T126 and T128.
- T072 gates T073: the yanked carrier lands only if the recorded comparison shows a yanked-state difference.
- W6 and W7 are siblings. T156 does not inspect green-run eligibility. T164 does not inspect release containment or alias state.
- Final verification may run after either child when only that child is in delivery scope, or after both when the full migration is being closed.

### User Story Dependencies

- US1 starts after W0 and defines the W1 aggregate and interface contracts that US2 and US3 consume.
- US2 and US3 start after T026 and may execute concurrently because their owned files are disjoint except for integrator-owned regeneration and `ci/rust/BUILD.bazel` reconciliation.
- US4 starts only after W1 merges and seals.
- US5 W3 starts only after W2; US5 W4 starts only after W3 and after the required qualification records exist.
- US6 W5 starts only from the immutable W4 digest. US6 W6 and W7 fork after W5 and never wait on each other.

### Within Each Code-Changing Wave

1. The plan panel returns unanimous empty recommendations, with the `software` seat filled by the Bazel and `rules_rust` expert.
2. Integrator prep fixes shared contracts and file ownership.
3. Test tasks are written and observed failing before their implementation tasks.
4. Disjoint scope work runs in separate worktrees.
5. Every scope commits before validation.
6. The integrator merges scope commits, regenerates owned outputs once, and runs validation.
7. The ten-role work panel returns unanimous empty recommendations on one committed snapshot.
8. The stable PR head is sealed, merge eligibility passes, and only then does the wave PR merge.
9. `nix-collect-garbage` runs after merge.

## Parallel Opportunities

- W0: T010, T011, and T014 can run in parallel after T004 because root tool pins, hub declarations, and Bazel support files are disjoint. T005 through T009 and T013 own separate `xtask` modules.
- US1: T027 and T028 can run in parallel after T026; T031 can run alongside both because it owns `bazel/carriers/aggregate.bzl`.
- US2: T034 through T042 can run in parallel because each owns a separate test file; T053 can run while T054 proceeds because `packages/xtask/src/bazel.rs` and runner source are disjoint. T051 and T052 must not run in parallel with each other where they touch the same crate's test files; assign them by crate.
- US3: T058 through T062 can run in parallel; T071 can run while T069 and T070 proceed because their carrier files are disjoint until integrator reconciliation.
- US4: T081, T082, T084 through T089 can run in parallel after T080 because they own distinct test or policy files; T083 runs only after T082 and shares `packages/d2b-bazel-runner/tests/cleanup.rs`.
- US5: T106 and T108 can run in parallel in W3; T119 through T123, T126, and T127 may run in parallel only on separate worktrees or agents that do not use the reference measurement host. T124 and T125 are serial and hold an exclusive reference-host measurement window after all other W4 build-heavy evidence work is idle.
- US6: T136 through T138 can run in parallel in W5. After T153, W6 T154 through T161 and W7 T162 through T169 are dependency-independent and may start in either order, but their shared `Makefile`, `packages/xtask/src/delivery/eligibility.rs`, documentation, and `specs/003-adr052-bazel-rust/evidence/post-promotion.json` paths require one branch to integrate first and the other to rebase before validation and panel.

## Concrete Parallel Examples

### User Story 1

```text
Agent A: T027 in packages/d2b-bazel-runner/tests/make_interface.rs
Agent B: T028 in packages/d2b-bazel-runner/tests/manifest_adapter.rs
Agent C: T031 in bazel/carriers/aggregate.bzl
```

### User Story 2

```text
Agent A: T034 in packages/d2b-bazel-runner/tests/coverage_map.rs
Agent B: T035 in packages/d2b-bazel-runner/tests/topology.rs
Agent C: T036 in packages/d2b-bazel-runner/tests/broker_topology.rs
Agent D: T037 in packages/d2b-bazel-runner/tests/companions.rs
Agent E: T038 in packages/d2b-test-locator/tests/locator.rs and packages/d2b-bazel-runner/tests/binary_identity.rs
Agent F: T039 in packages/d2b-bazel-runner/tests/runner_env.rs
Agent G: T040 in packages/d2b-bazel-runner/tests/junit_content.rs
Agent H: T041 in packages/d2b-bazel-runner/tests/junit_fsops.rs
Agent I: T042 in packages/d2b-bazel-runner/tests/junit_outcome.rs
```

### User Story 3

```text
Agent A: T058 in bazel/supply_chain/ and packages/d2b-bazel-runner/tests/supply_chain.rs
Agent B: T059 in tests/tools/no-bash-ast-walker/src/main.rs
Agent C: T060 in packages/xtask/src/schema.rs
Agent D: T061 in packages/d2b-bazel-runner/tests/api_census_carrier.rs and bazel/rules/tests/
Agent E: T062 in packages/d2b-bazel-runner/tests/inventory_stub_carriers.rs
```

### User Story 4

```text
Agent A: T081 in packages/d2b-bazel-runner/tests/budget.rs
Agent B: T082-T083 in packages/d2b-bazel-runner/tests/cleanup.rs
Agent C: T084 in packages/d2b-bazel-runner/tests/deadline.rs
Agent D: T085 in packages/d2b-bazel-runner/tests/process.rs
Agent E: T086 in packages/d2b-bazel-runner/tests/recovery.rs
Agent F: T087 in packages/d2b-contract-tests/tests/policy_docs.rs
Agent G: T088 in packages/d2b-bazel-runner/tests/startup_options.rs
Agent H: T089 in packages/d2b-bazel-runner/tests/disk_trim.rs
```

### User Story 5

```text
Agent A: T106 in packages/xtask/tests/fixtures/ci/
Agent B: T108 in .github/workflows/pr-bazel-rust.yml
After W3 merge, evidence agents run T119-T123, T126, and T127 away from the reference measurement host; T124 and T125 then run serially in an exclusive window.
```

### User Story 6

```text
Agent A: T136 in packages/d2b-bazel-runner/tests/make_interface.rs
Agent B: T137 in packages/xtask/src/cache_maintenance.rs and packages/xtask/tests/fixtures/ci/cache/
Agent C: T138 in packages/xtask/tests/policy_ci.rs and packages/xtask/tests/fixtures/ci/deadline/
```

## Implementation Strategy

### MVP: Setup, Foundation, and Complete W1

1. Complete T001 and T002 so the settled amended ADR is proven present in the base.
2. Complete and merge T003 through T024 so the pinned generated foundation, the four hubs, the workspace boundary, the hermeticity inventory, and the runner and locator crates exist.
3. Complete US1, US2, and US3 tasks T025 through T078, because the complete eighteen-surface aggregate cannot be independently accepted without exact topology, per-case evidence, dual-mode location, and enforcing policy carriers.
4. Stop and validate the complete W1 Bazel gate beside unchanged authoritative Cargo. Do not promote, publish cache, or alter required CI.

The honest MVP is therefore complete W1, not any single user story: US1 delivers an interface that proves nothing without US2 and US3.

### Incremental Delivery

1. Add total coverage, exact topology, dual-mode location, and per-case evidence through US2.
2. Add enforcing offline policy carriers, the nightly transition, and the vendored supply-chain tree through US3, then merge and seal W1.
3. Add local operational safety, startup-option identity, and synchronous trimming through US4, then merge and seal W2.
4. Run cache-free shadow CI, record the cold feasibility measurement, and collect immutable qualification through US5 W3 and W4.
5. Promote only from the merged qualification digest through US6 W5.
6. Deliver W6 and W7 independently when their separate mechanical conditions become true.

## Notes

- Commit before validation. New generated files and crates are invisible to Nix-backed validation until tracked in the scope commit.
- Every code-changing wave carries one semantic `changelog.d/` fragment, one plan panel, one integrated-diff panel, one PR, seal, and merge sequence, and post-merge garbage collection.
- Panel signoff is true only when recommendations are empty. All ten roles review the same committed snapshot, with the `software` seat filled by the Bazel and `rules_rust` expert for this delivery run. Reviewers inspect supplied evidence and do not rerun gates.
- Evidence summaries contain immutable references and computed outcomes only. Logs, transcripts, credentials, and attestation payloads stay out of Git.
- `[P]` never permits shared-file editing. The integrator owns shared contracts, generated reconciliation, workflow generation, and the W6 and W7 rebases for the shared `packages/xtask/src/delivery/eligibility.rs` and `specs/003-adr052-bazel-rust/evidence/post-promotion.json` paths.
- No literal census belongs in an implementation. Every census is a generator output that `test-drift` ties to the repository, and a hand-written count is a defect wherever a derivation exists.
- Existing committed passing code wins over prose. Record any discovered drift in the implementation PR body or the plan's Spec Corrections table; never silently realign code to stale prose.






