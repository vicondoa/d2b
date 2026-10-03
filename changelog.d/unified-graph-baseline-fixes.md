### Fixed

- The `d2bd` admission-cap test no longer flakes on a saturated process-global
  semaphore. The test drove every request in the batch through one shared
  admission cap and asserted on the cap's own accounting, so a concurrent
  acquisition could absorb a permit and the request it was waiting on would
  then never be admitted. It now owns the cap it asserts against.
- The shared Rust package-suite guard no longer reports a member package as
  absent while its test runfiles are still being written. The guard walked the
  lazily-materialised execroot, whose contents are a moving snapshot: a
  package that was listed in the aggregate but whose `BUILD.bazel` had not yet
  been materialised was reported missing. It now reads the repository root
  directly and no longer depends on snapshot timing, so a package named in the
  aggregate is a package it will find.
- The source-hygiene scan no longer scans Bazel's `TEST_TMPDIR` scratch. The
  scan walks the workspace while tests execute, and the execroot scratch under
  that directory holds a proof database and profile output that are being
  written as they are read. Scratch belonging to the build is now excluded from
  the scan, which reports on repository sources only.
