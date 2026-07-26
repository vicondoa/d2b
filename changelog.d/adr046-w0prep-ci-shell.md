### Fixed

- The continuous-integration workflows named their environment-scrubbing
  shell by a repository-relative path. The Actions runner resolves the shell
  program against `PATH` rather than the workspace, so every job failed
  during startup before running any step. Steps now run through
  `tests/tools/ci-shell`, invoked as `bash tests/tools/ci-shell`, which keeps
  the runner's lookup on `PATH` and defers resolving the wrapper to run time.
  The scrub is unchanged: exported shell functions and `BASH_ENV` are still
  removed before any step runs.
- `make check` described itself as the pull-request-equivalent Layer-1 gate
  but ran only each job's primary make target, while the continuous
  integration `tier0` job also ran the ADR index and CI coverage guards. Those
  extra targets are now declared in `tests/layer1-jobs.json` and consumed by
  both the workflow renderer and the local runner, so a job cannot run more
  in continuous integration than it runs locally.
