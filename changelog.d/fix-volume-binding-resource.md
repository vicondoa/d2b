### Changed

- Changed Volume-side attachment admission and binding minting to the neutral
  durable `VolumeBinding` resource, minted by the Volume side with deterministic
  binding identity per Volume / execution-target / named-view relationship.
- Changed `volume-virtiofs` to reconcile `VolumeBinding` with sole authorship of
  the fenced binding status projection (KTD3): readiness is fenced by UID,
  generation, and revision, stale reports are never accepted as ready, and guest
  start is gated on current binding readiness.
- Changed binding deletion to drain the virtiofsd worker and private endpoint
  (and wait for a present guest mount to clear) under the
  `volume-virtiofs.d2bus.org/volume-binding` finalizer, so no serving effects
  are orphaned.

### Removed

- Removed the `virtiofs.d2bus.org.Export` durable attachment contract in the
  same clean break (KTD10), superseded by `VolumeBinding`: its schema, finalizer,
  status projection, child-mutation path, watches, and host assertions are gone;
  `volume-virtiofs` no longer mints bindings and owns only its virtiofsd worker
  process and private endpoint effects.
