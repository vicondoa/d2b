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

### Fixed

- Fixed default-config Bazel Rust tests to carry Cargo dev-dependency feature
  variants transitively, including d2bd's `test-support` dependencies.
- Fixed upstream Gazelle idempotence for hand-owned Bazel package BUILD files.
- Fixed broker guest-control signing tests to use a path-safe test scratch root
  instead of a world-writable host temp directory.
