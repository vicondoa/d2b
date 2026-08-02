### Added

- Added the v3 Zone resource compiler foundations, deterministic per-Zone
  bundles, private artifact catalog, and typed Process, Volume, and topology
  projections.

### Changed

- Preserved legacy Nix emitters during the migration while making Zone
  resource validation reject runtime metadata, raw paths, and secret-shaped
  values before publication.
