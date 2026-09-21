### Fixed

- The facade-isolation test in the BuildBuddy configuration suite waits for
  every facade to reach its barrier instead of waiting a fixed five seconds, so
  a busy machine can no longer fail the test by being slow.
