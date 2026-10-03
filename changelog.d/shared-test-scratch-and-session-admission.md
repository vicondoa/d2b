### Fixed

- The Guest deployment bootstrap tests no longer share one scratch directory
  between concurrent runs. Every case named a fixed `d2b-guest-bootstrap-<name>`
  root under the process-wide `TMPDIR`, and `bazel test --runs_per_test=N` runs
  N copies of the test binary at the same time against that same `TMPDIR`. The
  helper recreates the root on the way in and each case deletes it on the way
  out, so one run's teardown removed the delivered graph another run was still
  about to read, and that Guest boot path refused with "deployment-bootstrap.json
  is absent, unreadable, or over the read bound". The root now carries the
  process id, the same per-process qualifier the other scratch helpers in this
  crate already use.
- The interaction test client now waits for the server to admit the session it
  is about to drive. `establish_interaction_client` returned as soon as the
  client handshake completed, but `admit_interaction_socket` only registers the
  session afterwards, on its own task; the composition assertions that ran next
  read `route_for_service` before that registration was guaranteed to exist, and
  under load they saw no route at all.