### Fixed

- A host-integration lane guest now reports activation from the kernel's own
  entropy estimate rather than from a line in its boot journal. The
  `d2b-lane-activation` unit used to grep the journal for the kernel's
  `random: crng init done` printk, which the kernel emits at about 0.02s -
  before journald reliably captures, and journald takes a SIGTERM and
  restarts a few seconds into the boot and then rotates on the clock jump
  that follows. Whether that one printk reached the journal was decided by a
  race in the first few seconds of boot, and a guest that lost it had a
  random pool that had come up perfectly normally and an activation that
  never finished: the unit sat on that wait until the launcher's whole 600s
  bound expired and the check failed with no stage line. The unit now reads
  `/proc/sys/kernel/random/entropy_avail` and treats the pool as ready at the
  256 bits the kernel itself calls initialised, which depends on no logging
  path at all.
- The lane's own fresh-boot-versus-restored equivalence gate asked the same
  question the same wrong way. Its marker counted `random: crng init done`
  lines in the guest's boot journal, and the two marker texts are compared
  verbatim, so a run failed whenever only one of its two boots captured that
  printk - a fresh guest reporting `crng=0 0` against a restored one
  reporting `crng=0 1` took out a restore that had restored correctly. The
  marker now reports the pool against the same 256-bit threshold rather than
  out of the journal, and reports that verdict rather than the raw estimate:
  the estimate is live and climbs for the first seconds of every boot, so
  reporting the number would have failed this gate on every run for a value
  the restore had nothing to do with.
- The guest-side activation bound is 420s against the launcher's 600s, where
  it was 1800s. The two bounds were the wrong way round: the host abandoned
  the guest twenty minutes before the guest would have reached the point
  where it explains itself, so the guest's own diagnosis was written into a
  console nobody was left to read and the failure surfaced as an absence.
  420s is about twenty-three times a measured healthy activation of 15-19s
  and leaves roughly 180s for the unit to print `the random pool was not
  initialised` and for that console tail to reach the launcher.
