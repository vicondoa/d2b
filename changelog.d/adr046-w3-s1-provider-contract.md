### Added

- The `Provider` ResourceType contract. A Provider resource authors exactly
  two fields, `artifactId` and `config`; every other Provider property is
  read-only data resolved from the signed manifest and catalog entry that
  identifier selects. The contract covers exact package, executable, manifest,
  config, schema, and service digests, publisher and trust identity, exported
  ResourceTypes with their base schema bindings, controller, service, and
  worker component descriptors, the closed dependency alias set, the signed
  standard capability matrix, registered `spec.provider` and `status.provider`
  extension schemas, export and import projection factories, and the upgrade,
  drain, and restart policy.
- Provider installation admission. Package presence alone is not installation:
  a `providerRef` resolves only a Ready Provider resource whose row selects the
  supplied manifest, whose artifact passes production trust admission, whose
  Provider API major is exact with only additive minors and no handshake
  downgrade, and whose published method surface is a subset of the surface its
  signed component graph exports.
- Provider toolkit fakes and fault injection. Hermetic fake core, resource
  store, dependency bus, supervisor, and effect clients, plus a per-call fault
  schedule, so a Provider crate can prove its controller's behaviour without a
  socket, a filesystem, a process, or a wall-clock wait. The fake bus resolves
  one declared dependency alias and never returns its binding table, the fake
  supervisor records a launch intent without spawning, and the fake effect port
  records an intent without mutating anything.
- Contract and integration coverage for a well-behaved Provider and for a
  malicious one: a self-attested or revoked artifact, an artifact negotiating
  past the required API, a Provider claiming a ResourceType twice or binding
  one it does not own, an extension schema registered for a foreign
  ResourceType, a worker granting itself a dependency portal or method
  surface, a backing Device or Binding smuggled across a Zone boundary, an
  impostor bootstrap binding, alias probing, a status write outside the owned
  set, and a capability refused without a signed declaration.

### Changed

- Provider diagnostics are redacted throughout. A publisher, artifact
  identifier, digest, component identifier, configuration object, and
  installation decision now render as a discriminant or a count rather than as
  the value they were handed, so a third-party artifact cannot author text that
  reaches a log line.
