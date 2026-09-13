### Fixed

- `d2bd-runtime`'s startup store-retry test no longer races the transient
  retry backoff against the wall clock.
  `startup_store_retry_uses_delayed_async_backoff` now asserts the 5 ms and
  20 ms retry delays against a paused Tokio clock that only the test advances,
  so `make check` stops failing on this test while the host is under load. The
  assertions are unchanged. The `tokio` `test-util` feature the paused clock
  needs is declared as a dev-dependency, so production builds resolve the same
  feature set as before.
