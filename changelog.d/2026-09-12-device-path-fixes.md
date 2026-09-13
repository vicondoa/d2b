### Fixed

- A failed pre-start flush now fails the device path. The one-shot row's phase
  cannot carry its outcome (a concluded pass publishes Ready whatever happened),
  so the TPM effect reads the row's `ephemeral` status projection: `succeeded`
  proceeds, `failed` fails the effect closed, and an unreadable projection is
  refused instead of read as success.
- A Process row awaiting its relaunch publishes `Pending`, never `Ready`. The
  retry arms return `ReconcileOutcome::RetryScheduled`, the runtime maps it to
  the pending phase, and the driver's typed status stays
  `AwaitingRestart { restart_count }` until a pass actually satisfies the row, so
  a row waiting for a restart can no longer read Ready over a process that does
  not exist.
- A Device worker runs as its trusted principal inside its namespace. The
  worker's argv now names the in-namespace socket owner (the single-entry
  mapping's uid/gid 0) for namespaced postures, and the broker grants the
  launched principal the per-runner ACL on the device socket directory
  (`grant_device_worker_launch_acls`, restricted to sockets strictly inside the
  broker runtime root). Previously the argv named the host-numeric principal -
  unmapped inside the namespace - so swtpm exited with an ownership error and the
  socket directory carried no entry for the worker.
