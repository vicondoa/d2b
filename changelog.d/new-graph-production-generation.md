### Added

- `make generate` now emits the unified resource graph's projections. The
  `gen-new-graph` step renders the provider registration table, service
  catalog, converted resource-type authority, and closure manifest from the
  per-crate `operations.json`, `registrations.json`, `resource-types.json`, and
  `service-catalog.json` declarations, and commits them under
  `generated/new-graph/`.
- Generation refuses to install a graph whose declarations and compiled
  provider crates disagree: a handler a crate compiles with no declared
  method, a declared method nothing compiles, a declared Provider two crates
  declare, and a declared effect service or service package the crate never
  spells all fail `make generate` instead of committing a graph that could not
  host what it states.
