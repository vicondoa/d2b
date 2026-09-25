### Fixed

- d2b-provider-device-security-key: the CTAPHID relay and its recording test
  doubles now carry the sanctioned per-site lint allows for their synchronous
  parking_lot lock acquisitions, matching the committed blocking-census
  baseline.
- d2b-provider-process: restart-budget counters, the ephemeral-runtime started
  flag, and the durable-runtime watching flag now use relaxed atomic
  ordering; the flags publish no other data, so the strongest ordering bought
  nothing.
- d2b-provider-toolkit: invocation identifiers use relaxed ordering for the
  monotonic counter; only uniqueness is required.
- d2b-provider-transport-unix: the transport portal lock is now a
  `std::sync::Mutex`; the portal only ever takes it with `try_lock()` on
  synchronous paths, so the tokio sync dependency is no longer needed for
  this lock.
- d2b-provider-volume-binding: the recording test doubles (serving effects,
  recording manager, and requeue scheduler) now use `std::sync::Mutex`, and
  the banned parking_lot dependency is dropped from the crate manifest.