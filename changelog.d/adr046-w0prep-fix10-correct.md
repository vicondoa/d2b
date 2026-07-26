### Fixed

- Diagnostic filtering now preserves a redacted tail when truncation or
  malformed bytes split a UTF-8 sequence, instead of suppressing the full
  compiler diagnostic.
- Runtime-ledger census regeneration now fsyncs a same-directory temporary file,
  atomically renames it over the pin, and fsyncs the parent directory.
- Structural JSON, YAML, and Nix policy lookups now reject duplicate direct-child
  keys instead of silently selecting the first value.
- Contributor guidance now describes the frozen active and retired
  process-marker path-universe pin and its independent checker.
