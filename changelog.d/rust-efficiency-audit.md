### Changed

- Removed unused Rust dependencies and their lockfile edges, reducing
  unnecessary dependency resolution and compilation.
- Consolidated pinned-test inventory discovery into one main-workspace Cargo
  listing, avoiding a duplicate contract-crate resolution pass.
- Grouped compile-fail capability-seal mutations by owning type and compiled
  resource API seal fixtures in one Cargo invocation, preserving diagnostics
  while reducing repeated dependency setup.
- Combined ADR-046 measurement policy checks so the documentation corpus is
  loaded once instead of once per test.
- Documented the Rust crate-graph audit and retained the current workspace
  boundaries where change-frequency data did not support a split.
