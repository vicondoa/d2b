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
- Keep high-input Rustc compiles local-exec with remote cache only: 876 MB
  of each compile's unique input is the rustc+LLVM toolchain, not crate
  sources. Cargo build scripts stay fully local. The largest remotely
  executed set under the 80 GB working budget is
  `ExtractCargoTomlEnvVars` plus `TestRunner` (467 MB gross per two-crate
  run).
- Keep the existing Cargo and Make Layer-1 graph authoritative while exposing
  the complete Bazel graph only through the opt-in local `make bazel-check`
  facade. Remote qualification remains blocked until provider-accounted
  transfer evidence is available.

### Fixed

- Fixed the Bazel graph after the v3 merge: added `d2b-host-argv`, dropped
  retired aca/relay labels, refreshed `Cargo.lock` and `MODULE.bazel.lock`,
  and synced crate source lists and library deps with the merged workspace.
- Fixed default-config Bazel Rust tests to carry Cargo dev-dependency feature
  variants transitively, including d2bd's `test-support` dependencies.
- Fixed upstream Gazelle idempotence for hand-owned Bazel package BUILD files.
- Fixed broker guest-control signing tests to use a path-safe test scratch root
  instead of a world-writable host temp directory.
