### Added

- Added an immutable Zone/resource identity contract that binds Zone UID,
  resource UID, desired-state generation, and committed revision.
- Added shared provider-neutral display and clipboard bridge attribution and
  transfer-frame contracts.

### Changed

- Exposed operation, protocol-token, workload-provider, launcher, lifecycle,
  posture, and capability contracts from the foundational contracts root.
- Updated unsafe-local helper consumers to use those canonical contracts
  directly instead of compatibility aliases.

### Security

- Legacy realm identity fields are rejected by the new Zone contract, and
  stale Zone or resource UID and generation tuples cannot match.
