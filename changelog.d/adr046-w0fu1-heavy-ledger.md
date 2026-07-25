### Changed

- The heavy-lane semaphore now verifies a nested invocation before reusing a
  slot: the inherited descriptor must be an open handle on the real,
  currently-locked per-uid slot file it names, proven by an independent open
  file description lock query. A forged, stale, or closed marker no longer
  skips acquisition; it acquires its own slot or fails closed.
- The heavy-lane semaphore namespace is anchored to a verified directory
  descriptor and refuses a shared parent a peer could rename entries in. An
  owned parent left group- or world-writable is locked down to `0700` instead
  of trusted, and a parent owned by another non-root user is rejected.
- Every public heavy lane (`test-integration`, `test-host-integration`,
  `test-hardware`, `perf`, and the umbrellas `test`, `check-ci`, `check-all`
  that invoke them) now acquires a heavy-lane slot itself; the raw work moved
  behind internal targets guarded against direct execution outside the gate.
- The hermetic runtime-ledger gate now warm-builds before timing, collects
  repeated execution-only samples at test and crate granularity, and enforces
  a complete, comparable census: a repetition floor, non-empty scopes,
  matching per-sample repetition counts, and detection of census ids dropped
  from a run. Its cargo invocations run from the workspace directory so
  the configured compiler wrapper is discovered.

### Fixed

- A terminating signal arriving exactly as a heavy lane's leader process exited
  could break supervision without sweeping the process group, leaving orphaned
  descendants holding the slot descriptor so the slot was never released.
  Supervision now drains pending signals before each exit check and once more
  after the child exits while signals are still blocked, then sweeps the group
  whenever an interruption was seen.
