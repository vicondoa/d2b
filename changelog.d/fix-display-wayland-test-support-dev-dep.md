### Fixed

- `d2b-provider-display-wayland`'s own tests now build under
  `cargo test --features test-support`. The crate's `#[cfg(test)]` unit tests
  call `AuthenticatedSessionRouteBinding::for_test`, a helper gated behind
  `d2b-session`'s `test-support` feature, but the crate never enabled that
  feature: its `test-support` feature is empty and its `[dev-dependencies]`
  did not turn on `d2b-session/test-support`, so the helper was compiled out
  and the test build failed with `no associated function named for_test
  found`. The crate now declares `d2b-session` with
  `features = ["test-support"]` in its `[dev-dependencies]`, matching the
  pattern `d2b-provider-toolkit` already uses for the same re-exported type.
