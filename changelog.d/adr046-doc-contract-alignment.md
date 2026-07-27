### Fixed

- Corrected the heavy-gate specification and migration map to describe the
  protected root-provisioned runtime namespace, its two provisioning paths,
  and its fail-closed no-fallback behavior.
- Corrected the runtime-ledger documentation to distinguish enforced aggregate
  per-crate process CPU from advisory per-test wall clock, and documented the
  exact closed test census without claiming a baseline or historical
  regression check.
- Updated the delivery specification for the required complete pull-request
  mapping and delivery artifact schema version 2, and documented `.scratch/`
  as the required home for throwaway probes.
