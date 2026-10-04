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

### Fixed

- The broker collapsed its own absent endpoint-access class into a live
  handler failure, so a driver's no-grant proof could never match and an
  already-revoked relationship retried forever instead of converging. The
  dispatch now answers that class with its own closed code and keeps its
  audit entry; every other refusal class still becomes a live handler
  failure.
- The endpoint driver replaced every child-ensure, child-list, delete, and
  finalize failure with a bare drain-pending code and discarded the cause, so
  a real failure was reported as still draining and was never diagnosable.
  The underlying error now rides out as a compared value.
- A derived endpoint binding carried its consumer reference as the target
  execution reference, which the target directory's closed Host-and-Guest
  vocabulary cannot place. No actor was ever spawned and the endpoint sat in
  drain-pending indefinitely.
- A Process whose committed target binding resolves to a Guest was refused
  under a host-mode driver, which made the guest-target launch path
  unreachable in the real composition.
- The endpoint driver reported a scheduled retry without ever scheduling one,
  and its only remaining wake fired on a status projection the Process
  family does not publish. An endpoint realized behind its producer in
  roughly one run in six, so a display session could fail to start. The
  driver now honours its own outcome contract.
- A display session published readiness from its first pass regardless of
  its children, so it reported ready while its processes were still pending
  and its bindings undelivered. The session now publishes readiness only
  when every owned child is ready and both canonical bindings are delivered,
  and reports its own projection rather than an empty one.
- A committed endpoint that had published nothing was read as an endpoint
  that grants nothing. A guest process could therefore launch carrying no
  endpoint access at all. An unproven source is now treated as unproven
  everywhere the surrounding branches already treated it that way, so the
  launch defers until the source has actually spoken.
