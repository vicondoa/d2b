### Added

- Added typed Volume storage providers for anchored layouts, bounded quotas,
  snapshots, TPM-safe state, and private virtiofs exports.

### Security

- Added fail-closed source-policy, store-view, ACL, and virtiofs sandbox
  validation without exposing host paths or filesystem descriptors.
