### Added

- Added canonical compile-safe package scaffolds and flake outputs for the
  remaining Provider dossiers so each accepted Provider has one workspace and
  package identity before its behavior is implemented.

### Fixed

- Fixed persistent-shell integration tests failing when nested checkout paths
  made their Unix socket paths exceed the platform limit.
