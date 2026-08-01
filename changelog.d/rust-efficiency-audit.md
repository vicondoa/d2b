### Changed

- Added rustdoc compile-fail examples documenting downstream fabrication
  barriers for authenticated capability types.
- Removed redundant external seal fixtures whose export and private-member
  checks are covered by the compiler-derived rustdoc JSON API census while
  retaining the forced-`cfg(test)` downstream trust-boundary probe.
- Removed unused Rust dependencies and their lockfile edges, reducing
  unnecessary dependency resolution and compilation.
- Consolidated pinned-test inventory discovery into one main-workspace Cargo
  listing, avoiding a duplicate contract-crate resolution pass.
- Kept semantic compile-fail mutations independently attributable while
  retaining cached fixture dependencies, and reduced the resource API seal to
  its narrow forced-`cfg(test)` probe.
- Added an absolute 60-second per-test wall-clock ceiling to the runtime
  ledger; shorter timing thresholds remain advisory while aggregate crate CPU
  budgets remain the regression gate.
- Combined ADR-046 measurement policy checks so the documentation corpus is
  loaded once instead of once per test.
- Cached workspace Rust source contents during the tracing-contract scan,
  avoiding repeated reads for each forbidden-attribute pattern.
- Reused the enumerated and loaded source set across the CLI consumer policy's
  multiple pattern scans.
- Removed unused direct `ttrpc` and `serde` dependencies from the bus and
  relay-bridge crates, respectively.
- Documented the Rust crate-graph audit and retained the current workspace
  boundaries where change-frequency data did not support a split.
