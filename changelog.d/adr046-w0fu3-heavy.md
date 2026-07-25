### Added

- `cargo xtask heavy-gate verify-slot` verifies that the calling process
  genuinely holds a heavy-gate slot. It re-runs the inode, ownership, and
  atomic open-file-description lock proof through the inherited slot descriptor
  and exits non-zero (without side effects) unless a real slot is held, so a
  shell or Make guard can distinguish "already inside the gate" from "must
  acquire" without trusting an environment variable.

### Fixed

- The heavy-gate teardown no longer risks signalling an unrelated process
  group. The supervisor now observes the leader's exit without reaping it
  (`waitid` with `WNOWAIT`), sweeps the process group while the still-present
  zombie pins the pid and pgid, and only then reaps the leader. Previously the
  leader was reaped before the group sweep, so an emptied group's numeric pgid
  could be recycled onto a stranger before the sweep ran.

### Security

- Every shell and Make heavy-lane guard now proves a genuinely held slot before
  running heavy work instead of trusting the presence of the `D2B_HEAVY_GATE`
  environment variable. The live lanes, the hardware smoke, the performance
  budgets, the aggregating runner, the layer dispatcher, and the
  `heavy-lane-guard` Make target all call `heavy-gate verify-slot` (via a shared
  self-guard helper) and re-acquire a real slot when it fails. Exporting
  `D2B_HEAVY_GATE` alone no longer bypasses the sole-use semaphore - the guard
  detects the unverified marker and acquires a real slot rather than running
  raw heavy work.
- The heavy-entrypoint inventory guard is now closed-world. It walks the live,
  hardware, benchmark, and cloud directories recursively (catching nested and
  non-`.sh` executable entrypoints), requires an executable self-guard on the
  performance-budgets canary, and parses the Makefile so that every
  `heavy-lane-*` work target must both depend on the guard and be reachable only
  through a public gate-acquiring delegation. Adding a new heavy entrypoint now
  fails the guard until it is gated, and `tests/static.sh`'s direct invocation
  of the performance canary is covered because that script self-gates.
