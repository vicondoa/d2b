### Fixed

- Preserved a redacted diagnostic tail with an explicit dropped-byte notice when
  compiler output exceeds the safe input bound, and retained repository-relative
  context when reporting paths.
- Reported every per-test runtime threshold breach as a visible, non-failing
  advisory while retaining aggregate process-CPU budget enforcement.
- Closed Nix policy-parser gaps for structural wrappers so nested resource
  envelopes cannot silently bypass status and execution-policy checks.
- Added a deterministic runtime-ledger census regeneration target and made
  census drift diagnostics name that concrete maintainer workflow.
