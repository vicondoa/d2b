### Changed

- The Guest family's status capture point is now an async lock
  (`Arc<tokio::sync::Mutex<Option<Value>>>` in place of the banned
  `parking_lot` lock): every write awaits it and no guard is held
  across another await. The driver's Cloud Hypervisor reconcile arm and
  the daemon's controller-status handler take it with `lock().await`,
  and `d2b-provider-guest` no longer depends on `parking_lot`.

### Fixed

- The Guest test-support recorder doubles and the guest driver's test
  harness no longer hold recorder state in `parking_lot` locks. The
  scripted effect and facet recorders use the toolkit's `SharedLog` or
  the async lock where only async methods touch them, and the blocking
  lock with a recorded per-site exception where the facet traits'
  synchronous accessors read them; the harness's order log is a
  `SharedLog` and its row/view recorders a blocking lock with the same
  recorded exception.
