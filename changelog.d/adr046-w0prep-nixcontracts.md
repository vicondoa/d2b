### Changed

- ADR 0046 Nix configuration contract: bounded `ZoneId` and `ResourceName` to
  1 to 63 bytes at eval time (`^[a-z][a-z0-9-]{0,62}$`) everywhere the generated
  options and reference validators are specified, with 63-accept/64-reject/empty-
  reject boundary coverage, so a name that Nix accepts is always admissible to
  the resource admission layer instead of failing far from its cause at runtime.
- ADR 0046 Process contract: replaced the floating-point `backoffMultiplier`
  field with integer fixed-point `backoffMultiplierMilli` (multiplier x 1000),
  so every canonical rendering is float-free and round-trips through digest
  computation.

### Fixed

- ADR 0046 Nix configuration contract: froze a single per-Zone generation
  layout so the specs no longer disagree on whether the bundle is a monolithic
  document or a per-resource-type file set, whether the index and artifact
  catalog are global or per-Zone, or what the retention setting is called. The
  contract is now one monolithic `resource-bundle.json` per Zone, a single
  site-wide `index.json` and `artifact-catalog.json`, one retention option
  (`retainedGenerations`), and one explicitly enumerated integrity digest chain,
  documented in one canonical section every other spec defers to.
- ADR 0046 Nix configuration contract: removed a fourth root-visible
  `d2bd.socket` unit from the systemd mapping; the daemon binds its public
  socket itself and reports readiness through `Type=notify`, keeping the
  framework at exactly three root-visible units.
- ADR 0046 Provider contract: defined Provider catalog identity as the Provider
  resource's `spec.artifactId` and removed every reference to an undeclared
  `catalogEntryId`, so reference resolution and duplicate-install detection are
  computable from the frozen Provider spec without a hidden side table.
- ADR 0046 configuration activation: specified that the active pointer, prior
  pointer, and retention metadata are persisted together in one atomic durable
  write before reconciliation is notified, and specified continuation-event
  restart recovery, so an interrupted activation can no longer leave a new
  generation active with no durably recorded rollback target.
