### Fixed

- Added a fail-closed tier0 policy scan that rejects internal development
  markers in shipped documentation, source comments, operator-facing CLI
  contracts, workflow labels, and released changelog sections. Existing debt
  is bounded by a frozen path-universe pin: only `activePaths` are exempt, new
  paths fail the digest check, and cleaning a path moves it to `retiredPaths`
  without changing the combined universe.
- Replaced development-wave references in CLI output fixtures with descriptions
  of the actual requirements, implementation state, and remediation.
