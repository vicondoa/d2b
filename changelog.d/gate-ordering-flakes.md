### Fixed

- The daemon's broker peer listener no longer binds its socket inside the
  spawned accept thread. `serve_broker_peer` returned as soon as the thread
  was spawned, so a caller could reach `connect()` before the socket path
  existed; under load that surfaced as a redacted
  `could not reach the privileged broker socket` transport refusal. The
  listener now binds and listens on the calling thread before the accept
  loop is spawned, which is what the three call sites already assumed and
  what the two sibling helpers in the same crate already did.
- The package suite guard sees the whole package set. One package's
  `BUILD.bazel` was the only one of the workspace sources omitted from the
  staged filegroup, and the guard read the live tree instead of the tree
  Bazel had staged for it, so its comparison was incomplete and it named a
  different package on each run while the tree never changed.