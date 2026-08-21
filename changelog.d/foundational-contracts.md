### Changed

- Inverted foundational contract ownership so `d2b-contracts` no longer
  depends on `d2b-core` or `d2b-realm-core`, while preserving wire
  serialization, validation, and redacted diagnostics.
- Made `d2b-contracts::v3::IfName` the sole interface-name owner and migrated
  core, host, daemon, and broker consumers to it.
