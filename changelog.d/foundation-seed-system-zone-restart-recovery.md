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
- The publication session open and the publication messages it precedes now
  answer in one wire vocabulary. The broker's accept-loop arm refuses a session
  open the way it refuses any other publication message, in the family's own
  `AuthorityPublicationResponse`, while the daemon read that leg as a second and
  unrelated `{session, binding, limits}` struct: so every refused open reached
  the manager as ``unknown field `kind`, expected one of `session`, `binding`,
  `limits` ``, and a typed refusal with its code and its fence was reported as a
  transport that never arrived. That is the difference between a retry and a
  dead projection, and it is why the daemon stopped at the first refused
  session open instead of naming the refusal. `AuthorityPublicationResponse`
  gains `Opened(OpenedPublicationSessionResponse)` and the open leg is answered
  in it, so a minted session and a named refusal are the same answer type on
  both legs. `AuthorityPublicationCoordinator::open_session` propagates a
  refused open as the refusal it is and refuses to hold a session the broker
  answered some other message with. Nothing was loosened to get there: both
  publication request shapes and the refusal body still deny unknown fields,
  and the new arm is one named variant rather than a catch-all.

- The broker's authority-publication accept loop now answers with the refusal the
  projection reached, rather than replacing every failure with its own
  `unaccepted-projection`. A refusal carries the closed code, the stage, the
  reason, and the state the Zone was left in; a boundary that discards them
  reports "this broker holds no accepted projection for the Zone" about a
  decision that was about something else entirely, which is how a
  store-generation refusal was read as a lost graph. A projection that could
  not open, persist, or be read reaches no decision at all, and that case keeps
  the boundary's own fail-closed code.
- `PublicationError::Refused` now carries the `ZoneAuthorityState` the broker
  answered with. The broker decides every refusal against the Zone it durably
  holds, so that state is the only statement of the Zone's posture the manager
  ever receives; dropping it left a manager unable to tell a Zone that owes an
  outstanding transaction from one that simply holds a different store
  generation, and both read as the same code with nothing to reconcile against.
- A Zone the broker already holds can reopen its publication session. A session
  open installs no authority: it does not move the accepted cursor, accept a
  row, prepare or release a fence, or unfence anything. It mints the token every
  later message must present, bound to the Zone, to the store generation THIS
  broker already holds for it, to its own epoch, to the authenticated initiating
  subject, and to the accepted cursor it already holds - so a caller still
  cannot move a Zone onto a caller's store generation by opening a session
  against it. Refusing the open was a refusal cycle: it was the one message a
  Zone could not recover with, because every message that could recover it
  needed the session the refusal withheld, so a Zone whose store generation had
  moved or that held a fence a manager had abandoned was left with no protocol
  path back and the operator's ownership-bounded reset as its only exit - which
  no boot may run. The Zone's posture is not read by the open and does not have
  to be: a fenced, reconciling, or mid-snapshot Zone is refused by name on the
  very next message it sends, and the store-generation rule stays exactly where
  it was, on every acceptance, resynchronization, and cancellation message.

- The daemon's spec store keeps its committed state in the database file
  instead of in a write-ahead log. The store's `store_incarnation` is an
  identity: every broker authority projection for the store's Zones is bound
  to it, and a projection naming an incarnation the store no longer carries can
  never be republished against - the Zone refuses every publication until the
  ownership-bounded reset clears both halves. In WAL mode the committed rows sit
  in `<name>-wal` and the database file stays a single page until a checkpoint
  runs, so a store whose writer is killed without one - a power cut, a
  `system_reset`, a snapshot taken and restored while the daemon is running -
  reopens as an EMPTY database. `apply_authority_journal` cannot tell that from a
  first boot: both present one page and an empty schema, so it mints a fresh
  incarnation, and the split between the store and the broker's projection is
  permanent and silent. The store now opens with a rollback journal and
  `synchronous=FULL`, so an abrupt end rolls back to the last commit and the
  store reopens on exactly the incarnation it committed. The matching fsync per
  commit is the price of an identity that means something; the store writes per
  desired-row mutation, not per read. The side-file posture enforcement now
  covers `-journal` and still covers `-wal`/`-shm`, because a database an
  earlier release left in WAL mode still has them and opening it converts it.

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