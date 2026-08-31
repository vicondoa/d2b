### Changed

- Developer Bazel commands and public Make aliases now default to BuildBuddy remote execution, while CI continues to select local execution.
- Public Make aliases now forward an explicit profile override and the repository-root test context to the pinned Bazel command.
