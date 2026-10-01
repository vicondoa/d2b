### Added

- The `EndpointBinding` serving half: a committed `Endpoint` row now derives its
  `EndpointBinding` rows, commits them through the family's child surface, and
  their delivery rides the exact-endpoint ACL wire the broker already serves.
  The endpoint's own consumer policy is the source of the deliveries, its own
  operation allowlist and attachment capacity decide how each consumer reaches
  the endpoint, and the committed slot is derived from the endpoint's own
  identity, so one endpoint is one socket name and nothing a consumer supplied
  reaches any of it. A repeat pass over an unchanged row commits nothing new
  and a narrowed policy retires the relationships it no longer derives.
- `EndpointBindingDriverFactory` serves those committed rows. Every pass
  re-derives rather than trusts: the row's derived name, the committed
  decision, the owning `Endpoint` row behind its owner fence, that row's own
  policy at its current generation, and the named consumer are all checked
  before anything privileged runs. The request is the wire's own typed form -
  the socket is the row's committed slot, the permission is the socket's own
  read/write triple for the admitted right, and the authority binding mixes the
  endpoint, the consumer, the Zone, the socket name, and the verb - so a grant
  cannot be replayed as an observation or a revoke.
- The serving pass reconciles against the broker's own answer rather than
  recomputing it locally. A standing grant is observed before it is re-applied,
  so a producer that replaced its socket reports a different pinned inode, and
  a POSIX ACL mask a later mode change nullified reports as short of the
  admitted right instead of as a prepared endpoint. A relationship short of the
  admitted right, an ancestor that no longer applies a traverse bit, and a
  parent the consumer may enumerate are all refused rather than delivered.
- `EndpointBinding` is registered in the converted resource-type catalog with
  the driver's own declaration, so the type has a real serving driver rather
  than a declaration without one.