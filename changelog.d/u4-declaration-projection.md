### Added

- The provider declaration now projects into every packaging and configuration
  surface: canonical manifest inputs, resource-and-operation graph rows,
  provider registration rows, the service catalog, the compiled consumer
  requests, and the private plan. One declaration, one build, and the typed
  binding request are the only inputs, so a provider method, a registration, a
  catalog row, and a plan row move together instead of drifting apart.
- A presentation capability now survives declaration, package manifest,
  compiler graph row, and private plan. The plan has no role, seccomp, or
  serving-worker field, so no inference fallback can supply a capability the
  declaration did not make.
- A configuration shorthand compiles into canonical consumer requests rather
  than persisting a second relationship list, and the compiler refuses a slot
  two different declarations claim.

### Changed

- Refused as their specific failure: a build output whose executable set does
  not hash to the digest the signed manifest pins, a configuration schema that
  is not the canonical schema its digest names, a duplicate consumer slot, and
  a retired contract version.

### Removed

- Nothing. The production generation path, the broker-operation merge inputs,
  and the committed generated artifacts stay exactly as they are until the
  cutover; the declaration-driven generators are staged beside them and write
  only into an isolated output directory.
