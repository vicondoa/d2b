- Broker authority publication with durable fences (U7, KTD6-KTD7).

  `d2b_contracts_broker::broker_wire` gains the four publication message
  families KTD6-KTD7 need - a bounded snapshot, a change, a fence, and an
  acknowledgment - carrying the Zone, the store incarnation, the expected and
  committed desired sequence, the exact staged transaction identity, and the
  canonical digest on every message. Snapshots transfer in bounded chunks with
  transaction id, ordinal, total count, and a final digest; no partial snapshot
  becomes active. `ForwardContext` now also carries the accepted authority
  cursor, so a forwarded effect is fenced against the projection it was minted
  under.

  `d2b_broker::authority_projection` is the broker's admitted projection: one
  serialized bounded worker per broker process orders `PrepareChange`,
  `CommitChange`, cancellation and recovery, and `BeginEffect` for each Zone,
  and owns the projection cursor and digest, the prepared fences, and the
  effect and reservation journal. The broker stores the projection; only the
  manager owns desired rows. A candidate is evaluated against the prior accepted
  graph, never against the grants it introduces. A store incarnation is an
  identity, never an ordered counter, so an unexpected different one is never
  accepted as newer. A broker restart moves every known Zone to reconciliation
  and denies new effects until the manager resynchronizes. The envelope's
  admission step now consults the same worker, so a fenced Zone refuses a new
  ordinary effect by name while the bounded control lane - which can only
  reduce use or recover known state - stays serviceable.

  `d2bd::authority_publication` is the manager-side freeze / commit / publish /
  acknowledge coordinator. `PrepareChange` and `CommitChange` are callback-free
  and take owned durable facts, so no SQLite transaction, manager lock, or
  source-reservation guard crosses the transport wait; a completion applies
  only if the expected transaction and desired sequence still match. Other
  authority mutations for the Zone queue behind the pending transaction while
  observation and safe drain stay serviceable, and a control timeout keeps the
  fence rather than thawing the Zone. `AuthorityAccepted` and
  `RevocationConverged` remain separate outcomes.

  The publication path is staged beside the unchanged production entry points.
  U34 wires it into the accept loop and the resource plane's mutation path,
  moves the publication message family onto the committed broker-operation
  catalog, and removes the old graph construction in the same cutover.
