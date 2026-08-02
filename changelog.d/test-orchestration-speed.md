### Added

- Add a bounded GNU Make Rust test DAG with grouped keep-going output,
  dependency-ordered leaves, serial broker feature passes, and explicit
  companion coverage for doctests and harness-free binaries while retaining
  the default fixture-dependent contract and CLI surfaces when Nix is
  available.
- Add opt-in version 1 execution manifests through
  `D2B_EXECUTION_MANIFEST`, including deterministic sub-surface fragments and
  atomic partial evidence for failed and handled-interruption runs.

### Changed

- Use `D2B_RUST_BUDGET` as the supported local Rust budget control. Top-level
  Make `-j` does not cap inner Cargo concurrency; the Rust target derives
  Cargo and nextest quotas from the effective CPU and memory budget.
- Keep the separate enforcing fixture lane from duplicating the aggregate by
  honoring `D2B_SKIP_FIXTURE_BUILD=1` in the Layer-1 orchestration.
- Keep the measured parallel profile for warm local runs, restore serial
  full-budget execution and shared target trees for cold runs, and run each
  Rust leaf as a separate full-budget CI job behind the stable rollup.
