### Changed

- Bound managed-identity Credential leases to authenticated subject, Zone,
  workload, Provider session, and bounded expiry state; restart checkpoints
  remain secret-free and finalization revokes only the owning workload's
  handles.
