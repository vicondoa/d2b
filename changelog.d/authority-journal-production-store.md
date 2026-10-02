### Changed

- The v3 spec store now has exactly one format, and it is the authority-journal
  format. The legacy desired-row schema, its migration list, the `StoreFormat`
  enum, and the store's direct desired-row writes (`ensure`, `mark_deleting`,
  `remove_after_cleanup`, `migrate`) are gone rather than left beside the
  journal: a durable authority change is staged, fenced, committed, and
  acknowledged, or it is not written.
- Opening a database written by an earlier release is refused by name instead
  of converted, and a store with no bound authority publication has no fence to
  commit against, so a durable mutation under it is refused rather than
  committed unfenced.

### Added

- `SpecStore::stage_mutation` now decides the exact committed state its
  candidate installs while the candidate is staged, so the broker's fence is
  validated against the bytes the commit writes rather than a prediction of
  them, and a replay after any failure boundary re-derives the same bytes.
- `d2b-resource-runtime` gained the broker half of the freeze / commit /
  publish / acknowledge order (`AuthorityPublisher`) and the one write path
  that drives it (`SpecStore::publish`): stage, fence, record the prepared
  identity, commit with the outbox entry, publish, acknowledge. Both the
  per-Zone manager and the foundation seed commit through it, so neither can
  write a desired row the broker never fenced.
- `d2b-resource-runtime::test_support` publishes the recording and refusing
  authority publishers a composition with no broker uses. They are test
  doubles rather than a second authority: they decide nothing and only ever
  return the facts the store already projected.
- The daemon binds that seam in production: `ResourcePlaneV3::prepare` builds a
  publication coordinator over the plane's own origination leg and store
  incarnation and hands it to the manager, and the foundation seed commits its
  seeded rows through the same path. A plane with neither a bound publisher nor
  an origination leg refuses to start, because it has no fence to commit a
  desired mutation against.
- On restart the manager adopts every transaction the Zone still owes an
  outcome for before it loads a row or spawns an actor: a candidate that
  committed nothing is released, and a candidate whose acknowledgment was lost
  is republished exactly as it committed. An outstanding transaction it cannot
  resolve keeps the Zone fenced and refuses the start rather than cleaning up
  against unaccepted authority.
