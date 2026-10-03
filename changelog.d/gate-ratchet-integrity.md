### Fixed

- The async-discipline gate scans the whole control plane instead of a
  hand-kept root list. The scan set is now every workspace crate the daemon
  and the broker-linking composition crate reach through their
  `[dependencies]`, plus every provider crate, so the crate that actually
  links the broker binary is covered and a substrate crate is covered the
  moment a binary links it. Root resolution fails closed: a missing entry
  point or an unreadable member manifest fails the gate and names the crate,
  where a renamed directory used to be skipped and leave a smaller scan set
  and a green gate. The control-plane crates that are deliberately out of the
  set are listed with their reasons and printed on every run.
- The blocking-API census baseline is a ratchet a change cannot walk down. It
  now carries a second per-crate axis, the number of
  `#[allow(clippy::disallowed_methods)]` / `#[expect(...)]` sites, and fails
  when either axis grows. Previously one allow attribute outside a provider
  crate deleted the diagnostic the count came from and lowered the committed
  baseline by one while every gate stayed green. A count can now only fall by
  removing the call.
- A census re-baseline is judged against the committed baseline before it
  overwrites it, so `--json <committed> --check <committed>` can no longer
  compare the tree against the file it just measured. A refused re-baseline
  also leaves the committed baseline untouched, and the failure message
  names every item that is over the line.
