### Added

- Added canonical Provider manifest and root configuration schema emission in
  `d2b-provider-toolkit`, plus the `manifest emit` and `manifest verify`
  authoring commands. Emission uses the exact `d2b-cjson/v1` bytes required by
  Provider artifacts, and verification reports a bounded first-divergent-byte
  offset with a direct re-emission command.
