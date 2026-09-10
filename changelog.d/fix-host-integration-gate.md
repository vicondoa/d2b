### Fixed

- Fixed the daemon resource plane wedging at startup under the
  `runtime-cloud-hypervisor-guest-preflight` host-integration lane: controllers
  now reach `Ready`, the VMM process is created, and all runner-death classes
  are eliminated. Runner processes respawn on transient source failures
  (including session-establish and controller-identity races) instead of dying,
  and bootstrap re-arms on transient establish/setup failures.
- Fixed effect-claim lifecycle to survive requeue and writer stalls: the claim
  digest is revision-bound and the post-commit record is hardened against
  stale-owner adoption.
- Fixed store error classification so watch-open and writer-stall closures are
  treated as transient rather than integrity failures, and lagging self-status
  writes are requeued (convergence-gated) instead of suppressed.
- Fixed startup publication races by retrying process-resource start (60 s
  budget) and publishing `Ready` status for alive controllers during observe.

### Changed

- Store durability and throughput: concurrent writes coalesce before fsync
  (drain-only coalesce window), status-class commit groups use relaxed
  durability, and the read lifetime under commit load was raised from 250 ms
  to 2 s with an extended startup source-retry budget.
- Fences no longer carry assignment epochs; succession is determined by
  generations and revisions alone, and pure-stale predecessors are adopted
  instead of wedging runners.
- Control-plane silent failures now log: a telemetry sweep across 31 crates
  (~2450 sites) replaces dropped errors with tracing; slow reads/writes log
  queue depth and bring-up phases log permanently.

### Removed

- Removed assignment epochs from the fence contract in the same change that
  moved succession to generations/revisions (no external consumers).
