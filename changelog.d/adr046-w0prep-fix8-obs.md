### Fixed

- Preserved a redacted diagnostic tail with an explicit dropped-byte notice when
  compiler output exceeds the safe input bound, and retained repository-relative
  context when reporting paths.
- Reported every per-test runtime threshold breach as a visible, non-failing
  advisory while retaining aggregate process-CPU budget enforcement.
