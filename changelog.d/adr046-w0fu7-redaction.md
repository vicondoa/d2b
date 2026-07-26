### Changed

- Major test suites now report their wall-clock duration, including the Rust
  workspace test pass and the runtime-ledger gate, so a non-failing performance
  regression is visible without imposing a flaky time budget.

### Fixed

- Make recipes and shared test helpers now discard inherited Bash functions
  before resolving tool names. An exported function can no longer shadow a
  PATH stub or expected system binary and silently redirect a gate.
- Runtime-ledger and heavy-gate build failures now retain actionable compiler
  diagnostics through a shared path redactor. The filter resolves symlinks,
  treats path metacharacters literally, respects path-component boundaries,
  redacts other absolute paths, and suppresses raw output explicitly if safe
  filtering is unavailable.
- Delivery snapshot Git failures now classify only anchored Git diagnostic
  phrases after removing quoted caller-controlled values. A keyword in a path,
  revision, or URL can no longer misreport a healthy repository as corrupt or
  select another unrelated repair reason.
- Process-marker and dash scan abort messages no longer include the absolute
  scan root.
