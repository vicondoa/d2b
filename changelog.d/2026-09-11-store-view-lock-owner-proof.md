### Fixed

- The store-view Volume converges: its declared `sync.lock` entry
  (`leaseClass: file-record`) is adoptable again. The broker now records its
  live ownership into the lock while it holds the exclusive `flock(2)`
  (`d2b-broker` `StoreSync` / `StoreVerify` → `hardlink_farm::SyncLockOwnerRecord`),
  and the daemon verifies that record against the kernel before adopting the
  entry (`d2bd` anchored adapter: same boot id, recorded pid still alive at
  exactly the recorded `/proc/<pid>/stat` field-22 start time, and the
  kernel-reported `comm` equal to the recorded owner). A lock with no record,
  a foreign payload, a record from another boot, or one naming a dead process
  stays ambiguous and quarantines exactly as before, so
  `adopt-with-live-owner-proof` keeps its meaning for every other lease class
  (no blanket quarantine exemption). A `file-record` lock the daemon has to
  create itself records its creator as the first live owner, so a lock no
  broker materialized is not self-quarantined. The record is deliberately
  descriptive evidence, not an attestation: it proves liveness and shape, and
  a same-uid writer could forge it - the quarantine protects against
  stale/ambiguous ownership, not against a writer in the file owner's trust
  domain.

### Changed

- A Volume layout report that is not `Ready` (Degraded/Pending) completes the
  driver's long effect as a *retryable failure* instead of a success. `Completed`
  re-enters the pass immediately, which respawned the layout effect - and, for a
  Nix closure source, the broker `StoreSync` its source resolution performs - at
  completion rate with no bound (measured ~10 StoreSync attempts/s while the
  store-view lock stayed quarantined). The retryable class is the resource
  actor's documented ownership (R13): it schedules exactly one requeue after the
  manager's backoff, so a degraded layout runs its effect, and therefore one
  source resolution, at a fixed interval instead of spinning.
