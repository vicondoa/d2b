### Changed

- The `Process`/`EphemeralProcess` driver's one-shot lifecycle memory
  guards its launcher clock and its terminal-outcome clock with
  `tokio::sync::Mutex` and awaits every acquisition, so a pass reaching
  either clock never parks the executor worker behind another holder.
  The clock semantics are unchanged: the launch clock is still set once,
  and a second terminal observation still keeps the first (the retention
  TTL never restarts).
- Every remaining lock site in the crate is either awaited or carries a
  per-site suppression whose reason names why it stays synchronous, so
  the crate reports no unsuppressed use of a denied lock path.
