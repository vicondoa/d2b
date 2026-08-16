### Changed

- Reuse normalized documentation text across ADR 0046 spike-measurement policy
  checks instead of rebuilding it for every site, inventory pattern, and
  negative control.
- Run the fixture-independent Rust policy binaries through one Cargo invocation
  while retaining fail-closed evidence that every selected binary executed
  nonzero tests.
