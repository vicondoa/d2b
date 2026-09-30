### Fixed

- The host-integration lane guest can now describe an activation stall more
  than once. The guest's own activation deadline came down to 420s while the
  repeat between stall reports stayed at 600s, which put the second report
  due at 690s, past a bound the unit had already hit. A unit that never
  activated was therefore described exactly once, at 90s, and a boot that was
  still changing was reported from a single fixed account of its first ninety
  seconds. The repeat is now 120s, so a stalled guest reports the ordering
  state it is sitting in at 90s, 210s and 330s - each of them inside the
  guest's own 420s deadline and inside the launcher's 600s bound, which the
  old comment wrongly described as something both values sat well inside of.
  The launcher reads what the guest writes as a span rather than a snapshot,
  keeping an ordering report whole from the line the guest opened it with to
  the line it closed it with, so a span it can carry is now a span the guest
  actually produces.
- The lane guest's activation unit is now covered by a test that runs it. A
  unit that never activates is driven against a faked clock, with systemd and
  the journal stubbed, and the test fails on a configuration whose stall
  report is reachable only once, which is the condition the guest shipped
  with.
