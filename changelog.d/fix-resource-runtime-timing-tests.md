### Fixed

- Timing-dependent resource-runtime tests no longer sleep before asserting: they
  wait on the observable state change or advance a paused clock, so a busy
  machine can no longer pass them without the behaviour under test actually
  occurring.
