### Fixed

- Restart recovery of Host-global authorities now has unit coverage: a
  tampered claim digest, a prepared-capability set that misses an active
  operation, and a duplicate operation id are each rejected as invalid
  authority requests, and a rehydrated operation is admitted only after its
  recovery resolution.
- The authority recovery coordinator's rollback is now tested: a failed
  record_close or release restores the recovery capability and quarantines
  the operation instead of silently reaching readiness, and resolving an
  observed-and-adopted operation clears the unresolved set.