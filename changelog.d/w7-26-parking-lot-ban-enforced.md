### Changed

- The blocking-API deny list names the parking_lot locks through the
  paths they actually resolve to: `lock_api::Mutex::lock`,
  `lock_api::RwLock::read`, and `lock_api::RwLock::write` replace the
  former `parking_lot::*` spellings. `parking_lot::Mutex<T>` and
  `parking_lot::RwLock<T>` are type aliases into `lock_api`, and clippy
  matches a configured method path against the definition a call
  resolves to, so an alias spelling never matched a call site. Reasons
  and replacements are unchanged, the sanctioned per-site allow
  vocabulary (`synchronous path`, `cfg(test) helper`, R4 worker
  boundary) is unchanged, and `parking_lot::Condvar::wait` stays as
  configured because `Condvar` is a real type, not an alias.

### Fixed

- The parking_lot lock ban is enforced again. The blocking census read
  zero for every `parking_lot::Mutex::lock`, `parking_lot::RwLock::read`
  and `parking_lot::RwLock::write` row because the configured path did
  not resolve, so unsuppressed lock sites demanded no per-site allow and
  the census disagreed with the manifest's recorded level. The census
  now counts the locks through `lock_api`, the committed baseline
  carries the renamed rows for every crate, and each of them reads zero
  against it.
