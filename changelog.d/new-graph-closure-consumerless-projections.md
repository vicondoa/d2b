### Removed

- Two staged projections under `generated/new-graph/` are gone, together with
  the documentation that named consumers they never had. The closure committed
  a declaration-only operation catalog and a per-provider graph policy. No
  production entry point compiled either file and nothing in the product
  loaded them: the retired broker-operations merge reads the per-crate
  `operations.json` declarations itself, so it neither read the staged catalog
  nor could have held its bytes - the staged catalog carried 29 rows where the
  committed rows document carries 100 - and the graph policy was documented as
  the canonical graph policy the configuration layer reads, when the
  configuration layer is the isolated Nix test surface, which takes an
  in-memory argument of a different shape and reads no generated file. The
  closure now holds the three tables a production entry point compiles, and
  the declaration-only catalog render that fed it is removed with them.

### Changed

- The closure manifest can no longer record an artifact with no consumer. Its
  `compiledInto` field was nullable to carry the staged catalog's null; with no
  consumer-less artifact left to describe it, the field is a plain string, and
  the root `generated_new_graph` filegroup names its four members rather than
  globbing the directory. A staged file added without a production consumer now
  fails the build instead of joining the closure quietly.
- The new-graph cross-check reads the provider crates' declarations directly
  rather than the staged JSON projections it used to parse, so its two sides
  are the declarations and the crates' own compiled sources rather than one
  rendering and another. Every violation it already raised is unchanged - a
  compiled handler no declaration carries, a declared method nothing compiles,
  a Provider identity two crates declare, and a service package a crate
  declares but never spells - and it now also refuses a crate whose
  `registrations.json` and `service-catalog.json` name different Providers.