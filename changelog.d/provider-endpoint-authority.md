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

- A Process whose target binding resolves to a Guest is launched, adopted,
  observed, and stopped through the authenticated target session, with its
  endpoint bindings delivered only after the launch lease revalidates. A
  reconnected session is re-adopted rather than assumed, and a stale or
  ambiguous restart survivor is quarantined instead of resumed.
- Display endpoints now admit only their three exact shapes: a lookalike
  that differs in class, transport, producer, locality, visibility,
  lifecycle, purpose, fingerprint, consumer policy, or operation is
  refused instead of half-admitted, and a socket that is absent,
  unconnectable, or rebound invalidates the readiness it previously proved.

### Removed

- The `Provider/execution-policy` reference. Nothing enforced it, so the
  `ExecutionPolicy` resource claimed a Provider that did not exist; the
  ResourceType vocabulary is now accepted without it.