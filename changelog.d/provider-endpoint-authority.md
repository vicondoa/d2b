### Added

- Provider identity authority: every `d2b-provider-*` crate now carries one
  explicit, surface-keyed identity declaration, and product, runtime, and
  session Provider names resolve from that declaration instead of from the
  crate directory, a runtime registration row, a session catalog row, or a
  product matrix row. Product, runtime, session, fixed-bootstrap,
  shared-driver, resource-family, and no-identity crates are distinct
  surfaces. Which of the packaged artifacts ships no binary is stated by the
  same declaration rather than named by the packaging generator, so the
  catalog's one non-binary bootstrap entry resolves from the owning crate's
  own fact.

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
- A display session projected the host compositor socket as its Wayland
  endpoint, because the projection took the first `Endpoint` child the child
  list happened to yield. Every consumer resolving `waylandEndpointRef` was
  pointed at the host side of the graph and fenced on that row's generation.
  The session now names the guest frontend's own endpoint through the display
  Provider's durable derivation, which is the row the transport is produced
  on (R23).
- The session's delivery gate compared a binding's `Delivered` state and its
  own row generation but never the delivery incarnation, so replacing a worker
  rotated the endpoint's realization token and the session still reported
  `Ready` over a grant made against the realization that token replaced. Each
  published relationship is now compared against the realization its owning
  `Endpoint` row currently holds as well (R20, AE14).
- The display graph never converged: the `Process` launch gate read a source
  row's silence as a proven binding withdrawal, so a pass that landed while
  that row was between its own passes stopped a live helper, which
  un-realized the endpoint behind it, whose republication woke the row again.
  A gate that could not READ its evidence is now its own answer and defers
  without stopping; only evidence that was read and does not stand stops a
  helper (R18, R21).
- That gate also re-armed every subscription on every pass, and subscribed to
  every endpoint its owner published - including the endpoint it produces,
  whose realization is behind its own readiness - so a converged row spent
  two manager round trips per dependency per pass and woke itself in a loop.
  A registration is now released and armed again when its target's evidence
  has moved rather than when a pass runs, and a source that has published and
  named no relationship for this consumer is not evidence this row reads or
  subscribes to (R12, R21).
- The `Process` launch gate read an empty expected `EndpointBinding` set as
  "this row requires no relationship" without ever proving the scope it read
  it over, so a row whose owner was still committing its children - and every
  root row, whose owner-scoped listing answers an empty set without asking
  anyone - could launch carrying no endpoint access at all. An empty set is
  now a statement about a proven scope: an owned row waits for its owner to
  publish for its own generation, and a root row reads the `Endpoint` rows its
  whole Zone publishes (R18, R22). The retention barrier reads the same scope,
  so a relationship committed anywhere the gate could not see no longer
  retires the row it was holding. An endpoint that cannot answer the
  Zone-scoped listing refuses rather than reporting no rows.
