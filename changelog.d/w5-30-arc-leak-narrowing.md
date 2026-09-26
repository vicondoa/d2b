### Changed

- Public surfaces stop handing callers a shared pointer they do not need.
  `ConsoleSession::ring()` now returns `&tokio::sync::Mutex<ConsoleRing>`
  instead of `&Arc<tokio::sync::Mutex<ConsoleRing>>`, the ring's wake handle
  is no longer reachable through `ConsoleRing::notify()` (waiters take the
  owned handle the table already gives them through
  `ConsoleSessionTable::read_output` and `ConsoleSessionTable::ring_notify`),
  `TargetBinding::new` takes the `TargetDirectory` handle by value instead of
  an `Arc<TargetDirectory>` (the directory is itself the cheap shared handle
  every binding of a Zone clones), `DaemonAuditLog.captured` is private behind
  `DaemonAuditLog::captured()` returning `&Mutex<Vec<String>>`, and
  `SkAcceptHandle.state` is private behind `SkAcceptHandle::state()` returning
  `&parking_lot::Mutex<SecurityKeyState>`. Every remaining `Arc` in these
  signatures stays because the value is genuinely shared: it is cloned across
  a `tokio::spawn` boundary, held by a factory that hands the same port to
  every driver it creates, shared between an admission offer and the session
  engine it admits, or fixed by a trait signature that already returns a
  shared handle.
