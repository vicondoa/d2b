### Changed

- ZoneLink routing and Provider forwarding now consume runtime-issued sealed route admissions instead of caller-populated authorization, connectivity, capability, and time claims.

### Security

- Route admission verifies exact ZoneLink identity, topology edge, controller and reconnect generations, Zone identities, immutable operation, capability, policy revision, session profile, and bounded expiry at current route use, with single-use invalidation after consumption.
