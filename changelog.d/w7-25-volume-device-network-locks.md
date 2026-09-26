### Changed

- The Volume driver's test recording manager holds its ordered log and
  its row store in awaited `tokio::sync::Mutex` locks: every endpoint
  and the tests' own `order()` snapshot await the lock instead of
  taking a `parking_lot` lock inside an async method.

### Fixed

- The Volume, Device, and network-local families carry no unsuppressed
  blocking-lock site. `d2b-provider-volume` and
  `d2b-provider-network-local` drop `parking_lot` outright: the
  network-local recorder doubles await their call log and both per-verb
  counters, the Device recorder doubles await theirs (the crate now
  declares `tokio` with the `sync` feature), and the network-local test
  requeue recorder counts ids with an atomic instead of a lock.
- The recorders a synchronous accessor reads keep a blocking lock but
  move to `std::sync::Mutex` behind a recorded per-site exception: the
  Volume runtime double's call log, read by the synchronous
  `has_layout` probe and by `call_order()`, and the network-local
  broker and reconcile fixtures, whose port and harness methods are
  synchronous. The Device driver's GPU authority-lease cache keeps the
  type the GPU port's declared construction contract locks on its own
  synchronous path. The async-gate hatch inventory is regenerated for
  the removed marker sites and the policy inputs for the dependency
  change.
