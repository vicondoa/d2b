### Fixed

- The kernel-backed Volume presentation lane builds and runs again. Retiring the
  pre-graph presentation posture made a launch state its realization directly,
  but the tests that exercise it still constructed one as optional, so the target
  stopped compiling. No gate built that target, so the break was invisible to the
  aggregate: the lane had silently stopped being evidence rather than failing.
  It now reports the declared realization the launch carries.

### Changed

- The effective-access evidence for a bound named Volume view is now real on a
  host that can establish a user namespace with a private mount tree. A worker
  reads and writes its admitted destination and cannot reach sibling source
  content, with and without a final user namespace; a read-only destination
  refuses writes and stays private to the launch; required host traversal
  survives mode and ACL reconciliation. The lane reports `SKIP` and returns when
  a host genuinely cannot establish that kernel posture, and an advisory skip is
  not accepted as passing evidence.