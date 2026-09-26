### Changed

- The unsafe-local helper registry's mutexes are `tokio::sync::Mutex`,
  reached through one seat helper: a caller that does not drive a
  runtime - the helper accept loop, its per-connection handler threads,
  and the daemon's `d2b-conn` dispatch threads - parks on the blocking
  seat, while a runtime worker keeps the repository's bounded try-lock
  spin, because the blocking seats panic inside a runtime and the
  `--once` serve path dispatches its connection inline on the accept
  loop's runtime worker.
- The op-lock manager no longer spins on four `try_lock` loops: the
  global and per-VM op locks are taken through the same dual seat, so a
  contended production op parks its dedicated `d2b-conn` handler thread
  until the holder finishes, while the inline `--once` path keeps the
  bounded spin that works on a runtime worker. The acquisition doc
  records both seats and the single lock ordering.
- The pidfd table's locks stay blocking primitives, each locking
  function carrying a recorded per-site allow. The readiness liveness
  probe and the startup-adoption pass read the table on runtime
  workers, where tokio's blocking seats panic, while the lifecycle and
  dispatch paths lock it from dedicated threads; one lock serves both,
  so the `parking_lot` dependency is unchanged.
