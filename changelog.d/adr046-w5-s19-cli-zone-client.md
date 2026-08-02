### Changed

- Route modern Zone resource, endpoint, and share commands through the
  authenticated, bounded Zone resource client.

### Fixed

- Preserve typed Zone session pins, cancellation, deadlines, and transport
  errors at the native CLI boundary without making modern calls accept realm
  target inputs.
