### Changed

- Use the native BuildBuddy Workflows `build / check` action for protected
  `v3` pull requests and trusted pushes, running the fixed remote Bazel
  Layer-1 graph without placing a BuildBuddy credential in GitHub Actions.
