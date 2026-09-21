### Fixed

- Broker budget, activation and reaper tests no longer measure wall time or
  sleep before asserting: they pause the clock, wait on the event the code
  actually emits, or probe real readiness, so a loaded machine cannot fail them
  or make them pass without the behaviour under test occurring. Readiness
  probing now also reports a crashed broker instead of a confusing connection
  refusal.
