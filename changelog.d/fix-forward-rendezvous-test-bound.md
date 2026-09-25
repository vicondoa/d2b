### Fixed

- The forward-rendezvous concurrent-calls test no longer reads the
  process-wide thread count, which parallel test load inflates while other
  tests spin up their runtimes. It now counts the threads that executed its
  own held calls, so it stays deterministic under load and still fails when a
  per-call thread-ownership regression returns.
- The forward-rendezvous test clients no longer fail when a refused call's
  write races the server's refuse-and-close: the refusal is already queued
  for the socket, so the reply is read instead of the write error.