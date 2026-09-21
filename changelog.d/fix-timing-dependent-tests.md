### Fixed

- Timing-dependent tests in the CLI, bus, provider supervisor, credential
  binding and guest controller suites no longer depend on wall-clock windows or
  fixed sleeps, so a loaded machine can no longer fail them by being slow.
