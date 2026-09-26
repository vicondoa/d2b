### Changed

- `d2bd` validates the `mode` recorded in a host activation marker
  against the public activation verbs (`switch`, `boot`, `test`,
  `rollback`) instead of carrying it as a free string, and the
  startup/status log line reports the recognized label. A mode this
  daemon does not know still parses into a catch-all variant, so a
  marker written by newer activation machinery keeps every other field
  readable.

### Fixed

- `d2bd` refuses host activation markers whose `schemaVersion` it does
  not understand. `status`/`list` no longer report degraded
  activation-pending from a future marker version, and neither startup
  pass (degraded-metric refresh, configuration staging) adopts one - a
  future version with a compatible field set can no longer parse as the
  current version. Each refusal is logged with the marker's VM and
  version.
