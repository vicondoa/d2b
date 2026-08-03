### Fixed

- Kept the controller reaction benchmark on its hermetic in-memory path until
  an authenticated Resource-API write route is available, so it cannot bypass
  store-owned mutation admission.
