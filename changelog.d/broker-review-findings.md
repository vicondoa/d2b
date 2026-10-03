### Fixed

- An accepted revision no longer moves an in-flight effect off the
  transaction it was admitted under. Only a settled record follows the
  accepted cursor forward, so a child that is still pending - or was
  cancelled but has not yet reported gone - can still be completed by the
  `EffectExit` naming the transaction its `BeginEffect` did. Previously an
  unrelated non-reducing commit landing between the two refused the
  completion by name and left the Zone permanently fenced, because the
  reducing-commit gate kept waiting for a child that could no longer report.
- The publication control lane is a real lane. Control actions reach the
  single authority writer over a second, separately bounded channel that the
  worker drains first, so a saturated ordinary queue can neither refuse a
  control action at admission nor hold it behind its own backlog. The
  documented bulkhead is now the code, rather than a constant nothing
  constructed.
- The exact-endpoint ACL surface is reachable. The broker creates the
  endpoint directory inside its own runtime root at serve time, before any
  connection is accepted, with a non-zero group class so the traverse entry a
  grant installs stays effective through a later mode assertion. Absent is
  still a refusal by name: the directory is never created from a request.