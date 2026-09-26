### Changed

- The `User` family's bounded local-account probe now runs its NSS reads
  on `d2b-core`'s bounded loader probe seat: `getpwnam`/`getgrnam` have
  no async form, so the whole blocking body is admitted with
  `loader_worker::run_probe` and a saturated or absent seat is reported
  as `SystemCoreError::DiscoveryUnavailable` - the same classification
  the probe always mapped a failed lookup to - instead of parking an
  executor worker for the lookup.

### Fixed

- The User test-support recorder doubles and the driver test harness
  hold their recorder state in async locks instead of `parking_lot`
  locks: the recorders await the lock inside the async effect methods
  and take a non-blocking `try_lock` from their synchronous accessors,
  so `d2b-provider-user` no longer depends on `parking_lot` and carries
  no unsuppressed blocking-lock site.
