### Fixed

- Storage and sync contract validation now reports a typed reason and the
  offending id instead of an opaque message string, so lifecycle reports can
  classify duplicate ids, restart policies, degraded reasons, and lock-order
  violations without string-matching.