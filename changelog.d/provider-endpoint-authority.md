### Added

- Provider identity authority: every `d2b-provider-*` crate now carries one
  explicit, surface-keyed identity declaration, and product, runtime, and
  session Provider names resolve from that declaration instead of from the
  crate directory, a runtime registration row, a session catalog row, or a
  product matrix row. Product, runtime, session, fixed-bootstrap,
  shared-driver, resource-family, and no-identity crates are distinct
  classifications, and a name may not be claimed by two crates on two
  surfaces.

### Changed

- Display Endpoint and EndpointBinding readiness is published by their own
  resource actors. A Process launches or adopts only when its expected
  canonical EndpointBindings are exactly delivered for the current
  realization incarnation, and that authority is revalidated immediately
  before the effect, so a revoked or replaced binding stops the live helper
  instead of leaving it running on stale delivery.