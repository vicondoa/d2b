### Changed

- Moved unsafe-local launcher and persistent-shell runtime identity to the
  Zone resource contract, fencing same-name resources with Zone and resource
  UIDs plus generation while preserving same-UID user scopes, bounded streams,
  redacted diagnostics, and exact teardown.
- Migrated unsafe-local helper runtime and shell consumers to the Zone resource
  identity contract instead of the d2b-core compatibility alias.
