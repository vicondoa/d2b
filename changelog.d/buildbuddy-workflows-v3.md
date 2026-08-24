### Changed

- Use the native BuildBuddy Workflows `build / check` action for protected
  `v3` pull requests and trusted pushes, reusing `tests/tools/bazel-check` with
  the local profile on an Ubuntu 22.04 hosted runner. This preserves the fixed
  graph/environment contract without nesting the RBE profile or using a GitHub
  secret-bearing proxy.
- Retire the standalone no-bash AST walker and its Make/Bazel policy targets;
  Bazel-owned Rust CLI and contract tests remain canonical for daemon-only
  typed behavior.
- Retire the obsolete heavy-gate semaphore and its host provisioning.
  Bazel-backed lanes invoke the existing Bazel facade directly, while retained
  Layer-2 and manual scripts run directly from their public Make targets or
  explicit invocation.
