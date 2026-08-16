### Added

- Added authenticated, Zone- and workload-bound Secret Service Credential sessions with opaque lease ownership and disconnect/finalization revocation.

### Fixed

- Kept missing-secret, outage, timeout, status, audit, log, and telemetry outcomes stable and free of credential material or identifiers.
- Sealed session capability minting behind one provider-owned, non-Clone authority with exact consumer and generation checks; lifecycle fencing now makes admission, inspect, disconnect, and finalization race-safe and drains admitted leases.
