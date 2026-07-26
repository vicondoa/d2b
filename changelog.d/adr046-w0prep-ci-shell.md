### Fixed

- The continuous-integration workflows named their environment-scrubbing
  shell by a repository-relative path. The Actions runner resolves the shell
  program against `PATH` rather than the workspace, so every job failed
  during startup before running any step. Steps now run through
  `tests/tools/ci-shell`, invoked as `sh tests/tools/ci-shell`, which keeps the
  runner's lookup on `PATH` and defers resolving the wrapper to run time. The
  dash bootstrap uses only shell builtins until the scrubber execs, so exported
  Bash functions and `BASH_ENV` are removed before any Bash process or step can
  run.
- `make check` described itself as the pull-request-equivalent Layer-1 gate
  but ran only each job's primary make target, while the continuous
  integration `tier0` job also ran the ADR index and CI coverage guards. Those
  extra targets are now declared in `tests/layer1-jobs.json` and consumed by
  both the workflow renderer and the local runner, so a job cannot run more
  in continuous integration than it runs locally.
- The runtime ledger's per-crate process-CPU budget was calibrated against the
  reference development host and was red on every GitHub-hosted runner, where
  the same suite measures roughly a third more process CPU. The budget is an
  absolute ceiling rather than a regression anchor, so it now clears the
  highest observed continuous-integration sample with headroom, and the test
  that exercises it derives its sample and expected message from the constant
  instead of restating the number.
