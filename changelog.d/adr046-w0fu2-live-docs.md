### Changed

- Reconciled the live-host testing instructions with the semaphore routing
  that now ships: a live script invoked directly re-executes itself through
  the heavy gate exactly once, so it cannot bypass the sole-use invariant,
  and any new live, hardware, or performance entrypoint must carry the same
  self-guard block or the fail-closed inventory guard rejects it.
