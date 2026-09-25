### Changed

- d2b-audit: the open-time chain-state scan now parses segment records
  straight from the read buffer with `serde_json::from_slice` instead of
  converting each line to a `String` first, dropping one allocation per
  record during sink startup.