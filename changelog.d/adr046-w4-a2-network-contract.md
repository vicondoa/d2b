### Added

- Added Network resource validation and status contracts, deterministic Linux-safe interface names, opaque attachment generation fences, and a reserved Network controller User readiness contract. These are contract and validation surfaces, not proof of a live Network Provider lifecycle.

### Security

- Contract and Nix validation refuse physical-NIC bridge multiplexing across Zones before any host effect; executable external-NIC host integration remains pending.
