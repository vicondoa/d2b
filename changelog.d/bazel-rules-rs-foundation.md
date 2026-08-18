### Changed

- Replaced the direct `rules_rust` and `crate_universe` foundation with the
  pinned `rules_rs` facade and root Cargo metadata authority, removing the
  Bazel-only Cargo lock and Gazelle compatibility scheduler.
