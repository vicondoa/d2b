### Changed

- Use the native BuildBuddy Workflows `build / check` action for protected
  `v3` pull requests and trusted pushes, reusing `tests/tools/bazel-check` with
  the local profile on an Ubuntu 22.04 hosted runner. This preserves the fixed
  graph/environment contract without nesting the RBE profile or using a GitHub
  secret-bearing proxy.
