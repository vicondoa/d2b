### Added

- Added canonical Network resource validation and status contracts, deterministic Linux-safe interface names, opaque attachment generation fences, and the reserved Network controller User readiness contract.

### Security

- Reject physical-NIC bridge multiplexing across Zones before any host effect so separate Zones cannot share an L2 broadcast domain.
