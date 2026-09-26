### Fixed

- The ACA Guest completed-operation ledger evicts its oldest entry by
  record sequence instead of cloning the map key first. Eviction order,
  the capacity bound, and operation replay are unchanged; the eviction
  now copies no `AcaOperationId`, and a unit test pins that the first
  recorded operation is the one dropped at capacity.

### Changed

- The declaration-only metadata driver's spec fence validates the
  decoded spec without materializing it: it keeps the same decode and
  shape refusals and no longer clones the whole spec object on every
  validate pass. The lock acquisitions in the runtime's scripted test
  doubles are per-site lint allows whose recorded reason names them as
  test-only helpers; the test doubles and their lock types are
  unchanged.
