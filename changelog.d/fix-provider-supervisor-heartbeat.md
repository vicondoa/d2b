### Fixed

- The ProviderSupervisor production-adapter test failed spuriously on loaded
  hosts: it asserted the single worst wall-clock heartbeat tick, which the host
  scheduler can delay while the executor cadence stays intact. It now bounds
  the heartbeat cadence distribution and additionally requires every blocking
  backend call to run off the async executor thread.
