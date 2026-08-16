### Added

- Added Bazel 9.2.0 compatibility fixtures, complete check eligibility
  inventory, prebuilt protoc enforcement, and a credential-isolated
  BuildBuddy evidence probe that remains non-qualifying without provider proof.
  The probe accepts only Bazel's credential-helper authentication mode,
  preserves sanitized partial capabilities, and rejects direct header
  authentication or credential material. Provider-accounted transfer evidence
  remains unavailable and does not qualify the integration.

### Fixed

- Fixed default-config Bazel Rust tests to carry Cargo dev-dependency feature
  variants transitively, including d2bd's `test-support` dependencies.
