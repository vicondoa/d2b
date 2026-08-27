### Added

- Added the ZoneLink route-admission contract for immutable operation intent,
  authenticated session profile binding, exact Zone identities, generations,
  capability, policy, and bounded runtime expiry.

### Security

- Sealed route-admission evidence is issued only from runtime-owned identity
  and clock state, rejects forged claims, and fails closed on stale or revoked
  ZoneLink generations.
