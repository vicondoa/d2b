### Added

- The v3 spec store can now open in an authority-journal format
  (`SpecStore::open_authority_journal`) that gives every authority change a
  durable identity of its own instead of borrowing the spec generation. Each
  committed desired row carries a desired revision, each Zone carries a
  monotonic desired sequence, and one durable publication transaction per
  candidate moves through staged, prepared, committed, and accepted states
  with an outbox entry and an accepted-publication cursor. This is what makes
  an ownership, view, consumer, provider-assignment, or execution-policy
  change invalidate earlier effect authority even when the rendered spec
  bytes are byte-identical, which the spec generation could never express.
- Desired revisions and Zone desired sequences are the shared
  `d2b-contracts-resource` authority counters rather than private types, and
  they fail closed at their ceiling instead of wrapping into a value that
  looks older than what it replaced.
- Every desired mutation in the new format commits through one SQLite
  transaction that carries the row, its revision, its audit record, its
  outbox entry, and the Zone sequence together, so a failure at any column
  leaves no half-published authority change and no store transaction open
  across the caller's broker I/O.
- `SpecStore::zone_recovery` answers a restart with an explicit decision per
  outstanding transaction - resume or discard, replay or cancel, or replay
  the exact committed publication - and replaying a committed transaction
  returns its recorded bytes rather than applying the mutation again.
- A database in the production store format is refused rather than converted
  when opened in the authority-journal format, and each format refuses the
  other format's operations, so a durable authority change cannot be written
  outside the journal protocol.

### Changed

- `d2b-resource-runtime` depends on `d2b-contracts-resource` for the
  authority, freshness, and identity contracts it persists. The edge points
  downward only: the crate still receives admission and broker-publication
  behavior through injected interfaces and still does not depend on
  `d2b-resource-types`, which depends on it.
- `packages/Cargo.guest.lock` records the new mirrored dependency edge for
  the Guest workspace copy of `d2b-resource-runtime`.