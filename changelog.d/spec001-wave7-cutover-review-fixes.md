### Fixed

- Make cutover runners adopt and resume durable journals safely, use broker-owned
  admission and verification observations, bound broker I/O, and preserve
  retryable or restore-required failure state across crashes.
