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
- Run Rustc on BuildBuddy RBE with compressed cache, minimal downloads,
  and 50 jobs. Local results may seed the remote cache. Cargo build
  scripts stay local after they showed the highest remote-cache traffic.
- Route `make check` through the Bazel aggregate plus remaining local
  Layer-1 jobs. Remote-cache read and write bytes are the provider
  evidence used to qualify BuildBuddy. GitHub Layer-1 runners stay on
  the generated Cargo/Make jobs until those runners host Bazel.

### Fixed

- Fixed the Bazel graph after the v3 merge: added `d2b-host-argv`, dropped
  retired aca/relay labels, refreshed `Cargo.lock` and `MODULE.bazel.lock`,
  and synced crate source lists and library deps with the merged workspace.
- Compile `d2bd_lib_test_support` against the production first-party graph
  so it does not link two `d2b_contracts` crates.
- Keep Nix, fixture, nested-Bazel, and host-namespace tests out of the
  default Bazel aggregate with local/manual tags.
- Fixed default-config Bazel Rust tests to carry Cargo dev-dependency feature
  variants transitively, including d2bd's `test-support` dependencies.
- Fixed upstream Gazelle idempotence for hand-owned Bazel package BUILD files.
- Fixed broker guest-control signing tests to use a path-safe test scratch root
  instead of a world-writable host temp directory.
- Fixed local Bazel runfiles, Rust test-support graphs, stale coverage carriers,
  and sandbox-safe scratch paths across the complete check graph.
