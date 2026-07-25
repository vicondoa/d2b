### Fixed

- Added a fail-closed tier0 policy scan that rejects internal development
  markers in shipped documentation, source comments, operator-facing CLI
  contracts, workflow labels, and released changelog sections. Existing debt
  is bounded by an enumerated shrink-only path ratchet: new paths fail, and
  cleaned paths must be removed from the ratchet.
- Replaced development-wave references in CLI output fixtures with descriptions
  of the actual requirements, implementation state, and remediation.
