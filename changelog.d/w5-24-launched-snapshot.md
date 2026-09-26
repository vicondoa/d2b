### Changed

- The Process effect backend's launched-runner snapshot is now the named
  `LaunchedSnapshot` carrier (`vm`, `role`, `pid`, `start_time_ticks`,
  `pidfd`) instead of a five-element tuple, and the supervisor's
  `LaunchedObserver::launched` receives that carrier instead of five
  positional parameters.
