### Changed

- Developer Bazel commands and public Make aliases now default to BuildBuddy remote execution, while CI continues to select local execution.
- Public Make aliases now forward only an explicit profile override to the pinned Bazel command.
