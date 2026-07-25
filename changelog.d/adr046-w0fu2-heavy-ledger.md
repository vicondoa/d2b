### Fixed

- Heavy-gate nesting verification now proves the advertised inherited
  descriptor is the one holding the slot lock. It issues a nonblocking
  `F_OFD_SETLK` through the inherited descriptor itself instead of probing a
  fresh handle, so a forged nesting marker that supplies an unlocked descriptor
  for a slot another lane happens to hold can no longer run a third concurrent
  lane, and the check-then-use race is removed.
- Heavy-gate now verifies the runtime root that holds the shared semaphore
  directory before use, accepting only a current-uid private directory or a
  mode-verified root-owned sticky directory and rejecting peer-owned roots, so a
  peer can no longer rename the verified shared directory between invocations to
  split the semaphore into a second namespace.
- Heavy-gate unconditionally terminates and reaps the supervised process group
  after the leader exits, before restoring the signal mask, closing the window
  where a signal arriving between the post-exit drain and the conditional sweep
  could kill the wrapper and orphan slot-holding survivors.

### Changed

- Every live, hardware, and performance test entrypoint now routes through the
  heavy-gate semaphore. The release smoke lanes and the aggregating and
  per-layer runners re-exec through the gate exactly once when invoked directly,
  and an inventory guard fails closed if a new live entrypoint or bare heavy
  make target is added without gating.
- The runtime execution-budget ledger now enforces a pinned closed census: it
  requires a census, measures execution-only time from warmed, crate-qualified
  libtest streams so compilation is excluded, reproduces the expected test and
  crate sets exactly, rejects census id loss and repetition mismatch, and runs
  as a required Layer-1 job. It holds no baseline and makes no
  historical-regression claim.

### Security

- The runtime ledger validates a short closed runner-label grammar, bounds
  printable test identifiers, row counts, and libtest input size, and rejects
  control characters both when emitting and when loading ledgers, so host
  paths, multi-line log injection, and unbounded artifact cardinality
  can no longer reach the recorded or printed output.
