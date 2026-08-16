### Changed

- Pull-request validation returns faster. The gate no longer evaluates the
  Nix-unit corpus twice, and a check that has to be built rather than evaluated
  no longer queues behind two dozen short ones, so the same coverage reports in
  roughly 40% less wall time. Every check remains enforcing, and the dispatch
  fails closed rather than skipping a check it cannot classify.
