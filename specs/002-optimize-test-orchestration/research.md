# Research: Optimize Test Orchestration

## Decision 1: Use GNU Make as the Rust aggregate scheduler

**Decision**: Express the local Rust gate as a bounded GNU Make dependency
graph over d2b's existing execution leaves. Keep Bash only for leaf-specific
toolchain setup, secure diagnostics, fixtures, and test surfaces Cargo cannot
represent directly.

**Rationale**:

- Make is already d2b's stable public test interface and adds no dependency.
- GNU Make provides bounded parallel execution, keep-going behavior, grouped
  output, and a jobserver for scheduling eligible workspace lanes.
- CI already proves that the API census, main workspace, and remaining suites
  are independent enough to run on separate runners.
- A Make DAG can encode the important safety edges: broker feature passes stay
  serial, same-target-directory operations do not overlap, and independent
  workspaces may overlap under one CPU and memory budget.
- Each heavy Cargo workspace receives an explicit `--jobs` quota. The static
  lane weights are chosen so every runnable Make frontier sums to no more than
  the aggregate budget; Cargo is not expected to acquire weighted Make tokens.
- Recursive Make recipes use `+$(MAKE)` and preserve the inherited jobserver.
  Bash leaf dispatchers do not interpret or redirect jobserver descriptors.

**Alternatives considered**:

- **Keep the serial Bash `all` mode**: rejected because it leaves independent
  lanes idle and keeps scheduling in the mechanism the feature is meant to
  retire.
- **Add a Python or Rust scheduler**: rejected as unnecessary custom
  orchestration.
- **Adopt `just`**: rejected because recipe dependencies execute sequentially.
- **Adopt `cargo-make`**: rejected because parallelism still requires an
  explicitly authored task graph, adding a dependency without capability Make
  lacks.
- **Use nextest partitions locally**: rejected because partitions are intended
  for multi-machine CI sharding and cannot represent d2b's separate toolchains,
  workspaces, and non-nextest leaves.

**Sources**:

- GNU Make jobserver overview:
  https://www.gnu.org/software/make/manual/html_node/Job-Slots.html
- Rust compiler jobserver behavior:
  https://doc.rust-lang.org/stable/rustc/jobserver.html
- nextest partitioning:
  https://nexte.st/docs/ci-features/partitioning/
- just dependency behavior:
  https://just.systems/man/en/dependencies.html
- cargo-make task dependencies:
  https://sagiegurari.github.io/cargo-make/

## Decision 2: Retain nextest and explicit companion surfaces

**Decision**: Keep cargo-nextest for ordinary workspace tests. Continue to run
doctests and discovered `harness = false` binaries explicitly, and keep the
broker workspace on serial `cargo test`.

**Rationale**:

- nextest already supplies process-per-test execution, native test concurrency,
  and consolidated failure reporting.
- nextest does not run Rust doctests.
- A plain `harness = false` binary is not nextest-compatible unless it
  implements the custom harness listing protocol.
- The broker has a measured process-environment incompatibility under nextest,
  while its 528-test runtime is already negligible.

**Alternatives considered**:

- **Fold all tests into one nextest command**: rejected because it silently
  drops capability-seal doctests and the core smoke binary.
- **Rewrite the custom smoke binary using another harness**: rejected as
  unrelated work with no demonstrated critical-path benefit.
- **Convert broker tests to nextest**: rejected because this is a correctness
  regression for process-global test state, not a scheduling preference.

**Sources**:

- nextest execution model:
  https://nexte.st/docs/design/how-it-works/
- nextest custom harness requirements:
  https://nexte.st/docs/design/custom-test-harnesses/
- nextest doctest tracking issue:
  https://github.com/nextest-rs/nextest/issues/16

## Decision 3: Use native Cargo diagnostics before adding build tools

**Decision**: Use Cargo's stable timing report to identify compile and link
critical paths. Verify rust-lld is active under Rust 1.97. Do not add mold or
cargo-hakari unless measurements show a remaining bottleneck they directly
solve.

**Rationale**:

- Cargo timings expose unit concurrency, duplicate compilation, and link time.
- sccache cannot cache final links for test binaries, so linking is a plausible
  warm-cache long pole.
- rust-lld became the x86_64 GNU/Linux default before the repository's pinned
  Rust 1.97 toolchain, so adding another linker without measuring would be
  speculative.
- The gate already invokes the whole workspace with aligned development and
  test profiles. That is not the common per-member feature-thrash pattern
  cargo-hakari primarily addresses.

**Alternatives considered**:

- **Adopt mold immediately**: rejected pending a measured whole-target benefit.
- **Add cargo-hakari immediately**: rejected pending evidence of duplicate
  feature-union compilation inside the same workspace invocation.
- **Increase Cargo jobs above logical CPUs**: rejected because Cargo already
  defaults to available parallelism and multiple simultaneous invocations
  would oversubscribe the host.

**Sources**:

- Cargo timing reports:
  https://doc.rust-lang.org/cargo/reference/timings.html
- Cargo build job configuration:
  https://doc.rust-lang.org/cargo/reference/config.html#buildjobs
- sccache Rust limitations:
  https://github.com/mozilla/sccache/blob/main/docs/Rust.md
- rust-lld default:
  https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable/
- mold:
  https://github.com/rui314/mold
- cargo-hakari:
  https://docs.rs/cargo-hakari/

## Decision 4: Select the Nix unit runner through measured experiments

**Decision**: Do not preselect the Nix unit implementation. Build and benchmark
several committed candidate branches against the same corpus, failure probes,
and warm/cold procedure. Select the fastest candidate that preserves complete
failure attribution, pin and shard integrity, `git+file://` path safety, and
bounded memory. More than one iteration is expected.

**Rationale**:

- The current corpus already converts each case into a non-throwing result with
  `builtins.tryEval`, then throws only after collecting failures within a
  shard. A whole-corpus aggregate can therefore be tested without redesigning
  case semantics, but one evaluator may leave most CPUs idle.
- Lix 2.94.2 on the representative host exposes `cores` and `max-jobs`, but no
  `eval-cores` setting. Native parallel evaluation must not be assumed.
- `lix-unit` is the Lix-compatible fork of `nix-unit`. It uses the evaluator
  C++ API, catches per-test evaluation errors, supports error type/message
  matching, and accepts flake outputs through
  `lix-unit --flake '<ref>#<attr>'`. d2b can expose an adapter attr with the
  already-injected corpus context.
- `nix-eval-jobs` evaluates attrsets in parallel, isolates per-attribute
  failures, emits JSON Lines, and provides worker and per-worker memory
  controls. Multiple workers can repeat shared dependencies, so its advantage
  must be measured rather than assumed.
- `nix-fast-build` builds on `nix-eval-jobs` and adds parallel realization and
  log rendering. It is relevant only if realization or diagnostics are a
  measured part of the critical path; its broader default check scope is not
  accepted without a narrow selector.
- `nix flake check --no-build --keep-going` on Lix continues evaluation after
  errors, but validates the whole flake output schema. It is a candidate for
  consolidating the Nix-unit and flake paths, not automatically the focused
  Nix-unit implementation.

**Required candidate matrix**:

- **N0 - tuned existing pool**: Retain the current process-per-shard runner as
  the control and tune only its worker count. It is not the preferred final
  architecture, but it prevents a slower replacement from winning by theory.
- **N1 - one pure-Nix aggregate**: Evaluate the complete existing result list
  once and throw once with all case failures. This maximizes graph sharing and
  minimizes process startup, but evaluation is expected to be single-core.
- **N2 - lix-unit adapter**: Expose the injected full corpus as a flake output
  and run `lix-unit --flake`. Measure tool build/startup separately from steady
  warm runs and verify pin/shard integrity remains an enforcing companion.
- **N3 - nix-eval-jobs**: Expose a dedicated attrset of Nix-unit jobs and run a
  bounded worker count with a measured memory cap. Verify that selected attrs
  do not broaden realization or bypass flake schema checks.
- **N4 - consolidated Lix flake check**: Measure whether one local
  `nix flake check --no-build --keep-going` can discharge both the focused
  Nix-unit and flake evaluation work without making focused iteration slower.
- **N5 - nix-fast-build**: Evaluate only if N3 wins evaluation but the required
  narrow realization and grouped diagnostics still dominate.

Each candidate is committed in an isolated experiment branch or worktree from
the same base. It receives one priming run, three warm runs, a cold observation,
peak memory and CPU sampling, an empty-discovery probe, and at least two
simultaneous failing cases in different shards. Candidates may be refined and
rerun. The winner is recorded only after it meets the contract; if none does,
the plan retains the current runner and records the unmet speed target rather
than landing a regression.

**Tools surveyed but not primary candidates**:

- **nix-unit**: Purpose-built and fast, but upstream states that modern Lix
  requires the separate `lix-unit` fork.
- **lib.debug.runTests**: Already compatible with the corpus shape, but does
  not isolate raw evaluation errors by itself.
- **Nixt and NixTest**: Pure/simple unit frameworks, but no evaluation-failure
  support according to the nix-unit/lix-unit comparison and therefore weaker
  than the current contract.
- **Namaka**: Snapshot testing, valuable for golden output workflows but not a
  replacement for d2b's value/error corpus.
- **NixOS VM tests and nix-vm-test**: Integration tiers, not substitutes for
  hermetic eval cases.

**Sources**:

- lix-unit:
  https://github.com/adisbladis/lix-unit
- nix-unit:
  https://github.com/nix-community/nix-unit
- nix-eval-jobs:
  https://github.com/NixOS/nix-eval-jobs
- nix-fast-build:
  https://github.com/Mic92/nix-fast-build
- Lix flake check keep-going semantics:
  https://docs.lix.systems/manual/lix/stable/command-ref/new-cli/nix3-flake-check.html
- Namaka:
  https://github.com/nix-community/namaka
- NixTest:
  https://github.com/jetify-com/nixtest
- Nix keep-going and job controls:
  https://nix.dev/manual/nix/latest/command-ref/opt-common
- nix-unit design:
  https://github.com/nix-community/nix-unit

## Decision 5: Treat shared-corpus normalization as a measured optimization

**Decision**: Prototype importing every case file once and deriving the full
corpus, shards, pin checks, and shard coverage from that shared map. Retain it
only when it improves at least one viable candidate without worsening
correctness, failure reporting, or memory.

**Rationale**:

- The current flake constructs the full corpus for pin checks and separately
  reconstructs subsets for every shard.
- A shared lazy value lets one `nix flake check` reuse parsed modules,
  fixtures, and evaluated case expressions instead of rebuilding equivalent
  graphs.
- The change preserves evaluation-time throws, no-IFD behavior, and the
  existing case format.

**Alternatives considered**:

- **Rely only on the flake evaluation cache**: rejected because any committed
  source change creates a new flake identity; the cache mainly helps exact
  retries, not the normal edit loop.
- **Introduce IFD to cache generated case data**: rejected because IFD blocks
  the evaluator on builds and weakens hermetic parallel evaluation.

**Sources**:

- Nix flake evaluation cache:
  https://nix.dev/manual/nix/latest/command-ref/conf-file#conf-eval-cache
- Nix unit testing overview:
  https://nix.dev/tutorials/nixos/testing.html

## Decision 6: Use one native flake check locally and keep CI shards

**Decision**: Local `test-flake` runs one native-system `nix flake check
--no-build --keep-going`, then realizes only the explicitly classified checks
that require execution. CI keeps its current per-check matrix.

**Rationale**:

- The default `test-flake` path is already the native single-evaluator model.
  The slower local Layer-1 path overrides it with a custom process-per-check
  scheduler that repeats flake evaluation.
- `nix flake check` validates output schemas and non-check outputs in addition
  to deriving check paths. A generic attrset evaluator is not a complete
  substitute.
- The narrow realized-check follow-up preserves the video command-surface
  contract without building the whole flake.

**Alternatives considered**:

- **Use nix-eval-jobs as the primary contract**: rejected because it evaluates
  derivation attrsets but does not replace full flake output validation and
  would require result aggregation logic.
- **Use nix-fast-build for the whole flake**: rejected because its build
  behavior is broader than the target's instantiate-only contract.
- **Use Omnix**: rejected because its default whole-flake scope is broader than
  the required checks.
- **Adopt Determinate lazy trees or parallel evaluation**: deferred because
  those capabilities are vendor-specific or not a stable stock-Nix baseline.

**Sources**:

- `nix flake check`:
  https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-flake-check
- nix-eval-jobs:
  https://github.com/NixOS/nix-eval-jobs
- nix-fast-build:
  https://github.com/Mic92/nix-fast-build
- Omnix:
  https://omnix.page/om/

## Decision 7: Benchmark warm and cold conditions separately

**Decision**: Warm acceptance uses one priming run followed by three timed runs
on the same commit and cache state. Cold results use the repository's targeted
cleaning path for Cargo and a fresh Nix evaluation cache while retaining the
host's Nix store. Cold results are reported but do not block completion.

**Rationale**:

- This reflects the contributor's normal repeat-development loop.
- Clearing the shared Nix store would be destructive, expensive, and not
  representative of ordinary local use.
- A fresh evaluator cache still exposes repeated parsing/evaluation work, which
  is the primary Nix concern in this feature.

**Alternatives considered**:

- **Clear the entire Nix store between runs**: rejected as unsafe and
  unrepresentative.
- **Use only exact-repeat eval-cache hits**: rejected because tracked source
  changes invalidate that cache in the normal workflow.

**Sources**:

- hyperfine:
  https://github.com/sharkdp/hyperfine
- Cargo clean and target directory behavior:
  https://doc.rust-lang.org/cargo/commands/cargo-clean.html
