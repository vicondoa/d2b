### Added

- The broker now resolves its private execution values from its own authority
  instead of installing an empty table. `PrivateExecutionTable::from_authority`
  joins three existing authorities and nothing else: the Zone's accepted graph,
  which says which relationships are committed and what each source admitted;
  the committed `Operation` contracts the broker serves, each naming its own
  implementation; and a new `PrivateExecutionValues` implementation the broker
  answers from the verified private bundle, which says what each of those runs
  against. A private source, its named views, and its mount point are read from
  the declared `Volume` row in the verified per-Zone resource bundle and the
  trusted storage contract that row's own source policy selects; the program,
  the arguments, and the environment are the verified bundle's own trusted
  runner intent for the template the committed contract names. Every one of
  those is a lookup keyed by an identity the broker already holds, so there is
  no path by which a caller's argv, environment, uid, gid, or mount policy
  reaches the plan. The values are resolved per admission over the live
  accepted graph rather than snapshotted at serve time, so an effect runs
  against the authority the broker holds now.

### Fixed

- A committed `Operation` no longer resolves an executable by picking the first
  trusted runner intent the verified bundle happens to hold. The admitted
  carrier names the operation and the relationship, not which execution it runs
  for, so a template two VMs each declare is exactly the case where taking the
  first one runs the wrong process; that resolves to nothing now, and so does a
  template the bundle does not declare. The previous behaviour was a guess
  dressed as a resolution, and deleting the refusal makes the case that pins it
  fail, which is how the refusal is proved rather than asserted.
- A relationship whose committed row state the broker cannot observe, or whose
  observation names another store generation, resolves no private source at
  all. Absence is a refusal rather than a default revision, so an effect is
  never planned against a dependency whose committed state the broker cannot
  show.
- A relationship the accepted graph committed but whose private host values no
  verified artifact names resolves to nothing rather than to an invented path,
  and a `Volume` the verified Zone bundle does not declare has no entry.
