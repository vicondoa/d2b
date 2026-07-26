### Changed

- Major test suites now report their wall-clock duration, including the Rust
  workspace test pass and the runtime-ledger gate, so a non-failing performance
  regression is visible without imposing a flaky time budget.
- The manifest-driven local Layer-1 gate now includes the changelog policy job,
  and manifest loading rejects any CI job with a local Make target that is
  absent from all local phases.

### Fixed

- Make recipes and shared test helpers now discard inherited Bash functions
  before resolving tool names. An exported function can no longer shadow a
  PATH stub or expected system binary and silently redirect a gate.
- Runtime-ledger and heavy-gate build failures now retain actionable compiler
  diagnostics through a shared path redactor. The filter resolves symlinks,
  treats path metacharacters literally, respects path-component boundaries,
  redacts other absolute paths, and suppresses raw output explicitly if safe
  filtering is unavailable.
- The runtime ledger now refuses a crate stream with no timed test events and
  pins the exact expected test identifiers as well as crate names. A vanished
  test can no longer turn into a zero-duration crate measurement or silently
  shrink the measured census.
- Runtime enforcement now uses aggregate process CPU time for each complete
  crate suite instead of libtest wall-clock time, without raising the existing
  crate budget. Per-test wall-clock timings remain explicitly advisory, so
  unrelated machine load cannot manufacture a regression while the exact
  non-empty test census remains mandatory.
- Delivery snapshot Git failures now classify only anchored Git diagnostic
  phrases after removing quoted caller-controlled values. A keyword in a path,
  revision, or URL can no longer misreport a healthy repository as corrupt or
  select another unrelated repair reason.
- Process-marker and dash scan abort messages no longer include the absolute
  scan root.
