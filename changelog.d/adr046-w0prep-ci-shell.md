### Fixed

- The continuous-integration workflows named their environment-scrubbing
  shell by a repository-relative path. The Actions runner resolves the shell
  program against `PATH` rather than the workspace, so every job failed
  during startup before running any step. The shell program is now `bash`,
  which resolves on `PATH`, and it execs the scrubber relative to the
  workspace at run time. The scrub itself is unchanged: exported shell
  functions and `BASH_ENV` are still removed before any step runs.
