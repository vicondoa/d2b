### Added

- The common Guest target/session contract: one provider-neutral
  `GuestTargetContract` in `d2b-provider-guest`, fenced on the admitted
  graph. It carries the enrolled Guest identity, the kernel boot identity,
  the authority Zone, the declared Provider row, the per-source uid and
  desired generation, and the live reconnect generation, and it answers for
  every target-control request whether that assignment may act. The
  declared Provider is graph data the contract never matches, so the local
  VM, media, and remote-cloud Guest providers each consume it and none
  waits for another.
- `GuestParentSessionEvidence` in `d2bd-runtime`, the daemon-side evidence
  one accepted Guest parent ComponentSession presents. A reconnect can only
  advance it, so a retained value can never rewind to a generation it
  already gave up.
- Per-source ownership the contract keeps across a lost session. A session
  that goes away retains the source key, source uid, and desired generation
  it was admitted with, admits nothing, deletes nothing, and re-binds
  nothing; a strictly newer session re-adopts the retained ownership
  instead of minting a second one.
- A structural ratchet over the contract's own source. The new test reads
  the contract modules and fails if either ever names a Guest provider or
  the family kind enum, with the forbidden token list derived from the
  family's own registration table so a new Guest provider extends the
  ratchet without editing it.
- A conservative ownership fence on the Guest-local resource store. A lost
  parent session quarantines the retained rows - they stay owned, for the
  same uids - and withholds only new authority until the enrolled Guest
  re-adopts them. A different Guest never adopts them.
