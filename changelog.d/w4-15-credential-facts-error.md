### Fixed

- A transient manager read failure during a credential dependency-facts probe
  is no longer silently reported as missing dependency facts; the daemon now
  logs the failure with the zone, provider, and execution refs instead of
  degrading credential readiness and revocation decisions without a trace.