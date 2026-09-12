### Changed

- The broker resolves a launch's posture - standard, controller with
  escrow, or serving worker - once from the trusted intent and carries it as
  a value. The file-descriptor contract, bootstrap and console response
  indices, escrow custody, runner credentials and the user-namespace
  mapping all read that value instead of re-evaluating the same condition,
  so a decision cannot be omitted at one site while the others agree. The
  intent-derived wire role alias is likewise evaluated once, for both spawn
  validation and observation.

### Added

- One test per posture asserts the whole decision set at once - descriptor
  contract, escrow custody, response indices, credentials, namespace
  mapping and admission fences - so dropping any single decision fails the
  test rather than surfacing later as a decode or identity error.
