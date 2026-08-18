### Added

- Added a Cargo-authoritative Bazel 9.2.0 check graph with fixed Rust, Nix,
  policy, fixture, proof, and companion suites.
- Added credential-helper-only BuildBuddy execution with trust partitioning,
  redacted evidence, and one typed pre-dispatch local fallback.

### Changed

- Kept public Make aliases and fixed CI jobs while moving Layer-1 scheduling,
  caching, retries, and aggregation into Bazel.
- Kept direct Cargo, nextest, doctest, feature, and manual Layer-2 workflows
  available alongside the Bazel graph.

### Removed

- Removed duplicate Cargo/Bazel authority, dynamic Layer-1 shell schedulers,
  transfer reports, and obsolete provider qualification artifacts.
