### Changed

- The daemon's composition root, the zone-plane session contract, and the
  resource contracts now compile the unified-graph declarations from the one
  committed closure under `generated/new-graph/` instead of a per-crate copy of
  the same table. The provider registration table, the service-to-provider
  catalog, and the converted resource-type authority each have exactly one
  committed byte: change a provider's `registrations.json`,
  `service-catalog.json`, or `resource-types.json` declaration, regenerate, and
  the next build links that regenerated table - there is no second file beside
  the compiled one that can drift from it. The three per-crate generated copies
  are removed rather than left as aliases.
- The closure manifest now records the production source file each staged
  artifact is compiled into instead of the path it was going to replace, and a
  gate fails if a staged artifact stops being included by the file the
  manifest names.
- The daemon's composition refuses to drift from the generated closure: every
  effect service the registration closure declares has a hosting factory built
  by the production construction, and the composition hosts nothing the closure
  does not declare. A new declared service now fails at the composition rather
  than refusing the Zone at provider startup.

### Removed

- The per-crate generated copies of the provider registration table, the
  service-to-provider catalog, and the converted resource-type authority. The
  staged closure artifacts are the compiled artifacts.