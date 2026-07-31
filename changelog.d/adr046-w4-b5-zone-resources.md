### Added

- Added generic per-Zone resource authoring, canonical integrity-pinned resource bundles, and separate private artifact catalogs.
- Added schema-aware Credential declaration validation with exact lifecycle authorization requirements.

### Changed

- Configuration publication now isolates foreign-owned name conflicts, refreshes unchanged resource generations without reconciliation, and deletes only removed configuration-owned resources through finalizer-safe asynchronous cleanup.
