### Changed

- The activation recording manager and the endpoint dead manager in the
  two provider crates' driver test modules hold their recorder state in
  `tokio::sync::Mutex` in place of the banned `parking_lot` lock: every
  take awaits it, the activation manager's `with_row`, `log`, and `row`
  readers turn async for the same reason, and no guard is held across
  another await.

### Fixed

- The activation and endpoint test-support recorders
  (`FakeActivationEffects`, `RecordingBrokerDispatch` and
  `FakeSocketEffects`) no longer take an unsuppressed blocking lock
  inside the async effect methods their facets call. The seats the
  synchronous accessors read keep the blocking lock with the recorded
  per-site exception (`clippy::disallowed_methods`, reason `cfg(test)
  helper`), so both crates read zero unsuppressed census sites once the
  meter resolves the locks through its `lock_api` deny paths.
