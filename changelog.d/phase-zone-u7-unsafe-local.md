### Changed

- Moved unsafe-local launcher and persistent-shell runtime identity to the
  Zone resource contract, fencing same-name resources with Zone and resource
  UIDs plus generation while preserving same-UID user scopes, bounded streams,
  redacted diagnostics, and exact teardown.
