### Added

- The daemon's display composition is now covered by a test that builds it
  the way startup does. `production_interaction_composition` - the function
  that wires one ready Zone's interaction registrar, its committed identity,
  and its system-core Resource API client into the DisplayService - had no
  test caller, and every `interaction_composition` test assembled its own
  composition over a test registrar instead. A regression that dropped the
  display driver's Resource API client, the committed identity, or the
  committed policy evidence would therefore have passed the whole suite.
  The new test drives the production builder, admits a real `d2b.display.v3`
  ComponentSession for it, reconciles, and then reads the Zone's resource
  plane back through the same client the builder bound: the committed
  `display-wayland.d2bus.org.WaylandSession` row carries the display
  finalizer, and it owns the durable `Process` the display driver launches
  against the committed Host execution reference plus the
  wayland-cross-domain `Endpoint` `d2b-wayland-proxy` serves. The test pins
  that wiring only; it makes no claim about the display reaching Ready.

### Fixed

- Durable display reads through the Resource API name no projection, and the
  Resource API refuses a read that does not: every read the display effects
  issued (`resource_get_request`, shared by the session, Process and
  Endpoint reads) returned `RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID`
  "projection is unspecified", so the production display reconcile failed
  closed before it wrote a single row. The request now asks for the full
  projection it always wanted.
- The durable display Endpoint status write carried an owner on an
  `UPDATE_STATUS` mutation, which the Resource API refuses outside Create
  and UpdateMetadata ("owner changes require Create or UpdateMetadata"). A
  status write never moves ownership, so the field is gone and the request
  is no longer rejected for that reason.

  The Endpoint status publication still does not succeed after this: the
  Resource API has no durable status write path by design, so it rejects the
  mutation once the owner is gone. The reconcile therefore still stops at
  that write, after committing the session finalizer and the durable
  Process and Endpoint rows, and never reaches Ready.