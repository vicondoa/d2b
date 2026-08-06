### Changed

- Strengthened the internal Bazel migration plan validator to reject malformed
  task records and aliased ownership paths, independently snapshot subprocess
  descriptor identities, refuse rebound descriptors, and verify prefix-progress
  cleanup before dependency and conflict analysis.
