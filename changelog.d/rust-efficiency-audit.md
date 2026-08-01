### Changed

- Removed unused Rust dependencies and their lockfile edges, reducing
  unnecessary dependency resolution and compilation.
- Consolidated pinned-test inventory discovery into one main-workspace Cargo
  listing, avoiding a duplicate contract-crate resolution pass.
- Documented the Rust crate-graph audit and retained the current workspace
  boundaries where change-frequency data did not support a split.
