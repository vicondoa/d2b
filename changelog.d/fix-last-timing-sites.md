### Fixed

- Remaining timing-dependent tests in the CLI, session, runtime-readiness,
  doctor and telemetry suites no longer race durations: they wait on the
  observable state, yield deterministically, or keep a ceiling generous enough
  that a loaded machine cannot trip it while still catching a magnitude
  regression.
