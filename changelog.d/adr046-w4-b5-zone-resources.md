### Added

- Added Nix authoring for per-Zone resources, canonical integrity-pinned bundle output, and separate private artifact catalogs.
- Added eval-time Credential declaration validation and an activation authorization contract; production application still depends on the resource compiler, store, and runtime path.

### Changed

- Added core planning for foreign-owned name conflicts, unchanged-resource refresh, and finalizer-safe cleanup of removed configuration-owned resources. No production store/watch adapter currently executes that plan.
