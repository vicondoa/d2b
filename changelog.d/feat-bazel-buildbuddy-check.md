### Added

- Added Bazel 9.2.0 compatibility fixtures, complete check eligibility
  inventory, prebuilt protoc enforcement, and a credential-isolated
  BuildBuddy evidence probe that remains non-qualifying without provider proof.
  The probe accepts only Bazel's credential-helper authentication mode,
  preserves sanitized partial capabilities, and rejects direct header
  authentication or credential material. Provider-accounted transfer evidence
  remains unavailable and does not qualify the integration.
- Added a local Bazel cache-transfer analyzer and repeatable Make facade that
  preserve gross and digest-deduplicated input bounds, execution classes,
  fan-out, artifact exposure, and local-to-remote boundary evidence without
  enabling BuildBuddy.

### Changed

- Keep local and pull-request Rust builds unstamped and prefer the available
  GNU BFD linker over gold. rules_rust metadata pipelining stays off after a
  measured two-crate local log showed higher gross input and fan-out.
- Wire BuildBuddy's Ubuntu GCC C toolchain on remote profiles so rustc
  linking does not use the host Nix gcc path.
- Run Rustc and Cargo build scripts on BuildBuddy RBE so C objects
  match the Ubuntu worker glibc. Compressed cache, minimal downloads,
  and 50 jobs stay on. Local results may seed the remote cache.
- Route local `make check` through BuildBuddy for supported Bazel
  targets, with an automatic local fallback. GitHub Layer-1 now runs
  the same Bazel aggregate with `--config=local` and no BuildBuddy.

### Fixed

- Fixed the Bazel graph after the v3 merge: added `d2b-host-argv`, dropped
  retired aca/relay labels, refreshed `Cargo.lock` and `MODULE.bazel.lock`,
  and synced crate source lists and library deps with the merged workspace.
- Compile `d2bd_lib_test_support` against the production first-party graph
  so it does not link two `d2b_contracts` crates.
- Keep Nix, fixture, nested-Bazel, host-namespace, and Cargo-spawning
  tests out of the default Bazel aggregate with local/manual tags.
- Fixed default-config Bazel Rust tests to carry Cargo dev-dependency feature
  variants transitively, including d2bd's `test-support` dependencies.
- Fixed upstream Gazelle idempotence for hand-owned Bazel package BUILD files.
- Print the redacted Bazel log from `make bazel-check` when the
  aggregate fails, instead of only a one-line status.
- Create the broker test scratch root before opening audit logs so
  Cargo and Bazel share the same parent directory contract.
- Collapse the contract-test repo-file scan so workspace clippy stays
  warning-clean.
- Refresh checked-in package policy inputs after the workspace Cargo
  authority change.
- Keep Gas City fixture genrules out of `make bazel-check` and fetch
  the locked crate graph before the offline production-closure check.
- Run the main, broker, and guest-shell-runner make/CI rust leaves
  through per-leaf Bazel jobs instead of Cargo. The Layer-1 rust-shard
  inventory now accepts those Bazel-wrapped make invocations. Per-crate
  leaves keep exclusive and local-tagged tests.
- Fixed broker guest-control signing tests to use a path-safe test scratch root
  instead of a world-writable host temp directory.
- Fixed local Bazel runfiles, Rust test-support graphs, stale coverage carriers,
  and sandbox-safe scratch paths across the complete check graph.
