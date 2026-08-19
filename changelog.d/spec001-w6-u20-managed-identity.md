### Changed

- Bound managed-identity Credential leases to authenticated subject, Zone,
  workload, Provider session, and bounded expiry state; restart checkpoints
  remain secret-free and finalization revokes only the owning workload's
  handles. Reacquisition replaces terminal or stale-session records without
  inflating the accepted client rotation generation, and repeated checkpoint
  restore remains occupancy-safe and idempotent.
- Restricted service admission to local authenticated transports and matching
  consumer generations, revoked superseded active handles before reacquisition,
  and kept terminal lease metadata observable without reopening closed leases.
