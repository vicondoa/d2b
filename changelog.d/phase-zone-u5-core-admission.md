### Added

- Added durable ZoneLink route identity and post-commit admission handoff.

### Security

- Route admissions now require current ready-session and cursor ownership,
  bind the committed ZoneLink identity and policy, and reject stale,
  revoked, conflicting, aborted, or restarted state before evidence issuance.
- Committed route operation IDs are durably retained without eviction and
  recover only for their exact ZoneLink identity and controller generation.
