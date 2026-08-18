### Added

- Added a Cargo-authoritative Bazel 9.2.0 check graph with fixed Rust, Nix,
  policy, fixture, proof, and companion suites.
- Added credential-helper-only BuildBuddy execution with trust partitioning,
  redacted evidence, and one typed pre-dispatch local fallback.

### Changed

- Kept public Make aliases and fixed CI jobs while moving Layer-1 scheduling,
  caching, retries, and aggregation into Bazel.
- Kept production Rust labels free of Cargo's `test-support` feature while
  retaining explicit same-crate Bazel variants for test-support graphs.
- Kept direct Cargo, nextest, doctest, feature, and manual Layer-2 workflows
  available alongside the Bazel graph.
- Declare package, deny-policy, and shell-completion sources for fixed Nix
  Bazel inputs so source changes invalidate the corresponding checks.
- Let CI use its native Bash when user namespaces are unavailable while local
  Bazel development keeps the FHS action shell.

### Removed

- Removed duplicate Cargo/Bazel authority, dynamic Layer-1 shell schedulers,
  transfer reports, and obsolete provider qualification artifacts.
