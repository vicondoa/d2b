### Changed

- The pinned Rust toolchain moves to 1.97.0.
- Rust tests execute under `cargo-nextest`. Doctests and `harness = false`
  binaries are not a nextest surface, so each workspace additionally runs
  `cargo test --doc` and a `cargo test --test` pass over any harness-free
  target, discovered from the test listing rather than pinned. The privileged
  broker workspace stays on `cargo test`, because its tests depend on the
  harness environment in a way that process-per-test execution does not
  preserve.
- `[build] rustc-wrapper` in every cargo workspace now points at a shim that
  uses sccache when it is available and plain rustc when it is not. Naming the
  binary directly made sccache mandatory for every cargo invocation, so
  environments without it had to clear `RUSTC_WRAPPER`, and that override
  spread into environments that did have sccache and silently disabled the
  compiler cache.

### Fixed

- The rustdoc and compiler caches used by the capability-seal tests are keyed
  on the toolchain. Reusing a tree produced by a different rustc version could
  fail a render or leave stale capability-surface evidence.
- The mint-surface guard discards the rendered documentation of packages whose
  library and binary targets share one output directory. Against a warm tree
  Cargo re-runs only the target it considers dirty and overwrites that
  directory, dropping exactly the private items the guard checks.
- Continuous integration caches the nix store and the guest-shell-runner cargo
  target directory. Neither was cached, so every run rebuilt them from source.

### Added

- `tests/tools/repro-rust-gate-env.sh` reconstructs the Rust gate's toolchain
  environment and runs a single command inside it, for diagnosing failures that
  only appear there without running the whole gate.
