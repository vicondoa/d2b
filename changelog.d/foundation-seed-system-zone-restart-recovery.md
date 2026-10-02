### Fixed

- The foundation seed no longer wedges a Zone across a restart. The seed homes
  its rows in the reserved `system` Zone, which is not the plane's own Zone, so
  the per-Zone manager's restart adoption never covered it: a previous boot that
  died between staging a seeded row and settling it left that Zone owing an
  outcome, and because the seed published before anything adopted, every boot
  after it was refused at the seed's first publish by the
  one-outstanding-transaction-per-Zone rule. The daemon could never start again.
  `ResourcePlaneV3::prepare` now adopts the system Zone through the same
  Zone-level recovery the manager uses, before the seed stages anything, so a
  transaction a prior boot left outstanding is released or explicitly refused
  there rather than becoming a permanent refusal. A plane with no foundation seed
  is unchanged.
- The broker's accept loop now serves the authority-publication message family.
  Every durable desired-row commit the daemon makes crosses this socket, and an
  accept loop that could not decode one dropped it as malformed wire and closed
  the connection, which the daemon read as a transport failure and reported as
  its store refusing. The frame is classified ahead of the typed decode, which is
  safe because both publication shapes deny unknown fields and neither can be a
  typed request envelope: a typed request is internally tagged and so always
  carries a `kind`, while a publication envelope carries the `session` a typed
  envelope never admits. The frame is then gated on the authenticated daemon
  peer and the per-uid IPC limiter exactly as an admitted effect is, before any
  Zone is touched.
- The daemon's publication publisher speaks per Zone instead of per plane. The
  foundation plane publishes for its own Zone and for the system Zone its seed
  homes rows in, and each Zone has its own fence and its own accepted cursor, so
  a seeded row was previously fenced under the plane's Zone while the store
  reserved its sequence under the system Zone. A candidate whose Zone the
  publisher was not bound for is now refused by name rather than served under
  another Zone's fence.

### Changed

- A Zone now runs ordinary effects only once it has published authority and been
  accepted. Opening the projection on the first publication is what makes this
  the case: an unprovisioned Zone is fenced, so a Zone that has published nothing
  is refused new ordinary effects rather than admitted blind. This is the posture
  the Zone's own authority state already described, and it is the change the
  publication family was built to make.
- `d2b_resource_runtime::authority_publish::adopt_outstanding` is now the one
  recovery path for a Zone, as a function of the Zone rather than of a writer.
  The manager's restart adoption and the foundation plane's pre-publish step both
  call it, so the two can never recover one Zone two different ways.