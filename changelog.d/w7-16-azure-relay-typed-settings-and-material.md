### Fixed

- `d2b-provider-transport-azure-relay`: `RelayTransportSettings` is now
  deserialized through the same admission the constructor runs, so a
  settings blob can no longer be admitted with an identifier the
  constructor refuses, and the pinned settings schema records the
  secret-shape exclusion.

### Changed

- The gateway credential loader describes the fixed
  `relayListen`/`relaySend` envelope shape with typed wire structs
  instead of walking JSON paths. The accepted values are unchanged;
  unknown keys are now refused rather than ignored.
