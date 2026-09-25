### Changed

- The new-plane readiness flags (`NewPlaneReadinessState`) now use the
  weakest correct memory ordering (`Relaxed`) for their independent
  published bits instead of `SeqCst` on every store and load; cross-thread
  visibility of the startup path is already ordered by the daemon's
  join/actor supervision, so the change removes needless synchronization
  without changing observable behavior.
- The daemon audit no-op sink can now be pointed at a state directory in
  tests (`no_op_with_state_dir`), so the no-op log's never-writes-files
  contract is actually asserted against the directory it was given.