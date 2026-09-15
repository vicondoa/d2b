### Added

- Bazel now gates clippy over every workspace crate: each `rust_library`,
  `rust_binary`, and `rust_test` declared through the new
  `d2b_rust_rules` wrappers (`bazel/checks/rust/d2b_rust_rules.bzl`)
  auto-emits a `rust_clippy_test` that runs with the workspace's per-crate
  `-Dwarnings` rustc flag, so a clippy regression in any crate fails
  `make check` and the CI `rust-*` targets instead of slipping past the
  Layer-1 gate. The broker crate is gated through the aggregate
  `//packages/d2b-broker:clippy` target folded into
  `portable_rust_broker`, and `d2b-provider-config-nixos` folds its
  per-target clippy tests into its explicit `all-tests` suite. All ~92
  package BUILD files migrated onto the wrappers; `check-clippy` remains as
  the standalone cargo lane.

### Changed

- `make check` is now purely the Bazel Layer-1 gate; the standalone
  `check-clippy` (cargo) lane is no longer chained into it, since the Bazel
  clippy tests enforce the same strictness (`-Dwarnings`) the crate's rustc
  builds already apply.