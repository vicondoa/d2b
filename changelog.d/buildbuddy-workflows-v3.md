### Changed

- Make plain `make check` zero-configuration and local by default for ordinary
  contributor checkouts and BuildBuddy Workflows; developer remote execution
  remains an explicit `D2B_BAZEL_PROFILE=remote` opt-in. The native BuildBuddy
  Workflows `build / check` action runs exactly `make check` on its Ubuntu
  22.04 hosted runner, using the vendor runner marker to select the
  remote-compatible local target set without nesting the RBE profile or using
  a GitHub secret-bearing proxy.
- Retire the standalone no-bash AST walker and its Make/Bazel policy targets;
  Bazel-owned Rust CLI and contract tests remain canonical for daemon-only
  typed behavior.
- Retire the obsolete heavy-gate semaphore and its host provisioning.
  Bazel-backed lanes invoke the existing Bazel facade directly, while retained
  Layer-2 and manual scripts run directly from their public Make targets or
  explicit invocation.
