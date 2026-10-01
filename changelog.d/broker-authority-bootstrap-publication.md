### Added

- The daemon now publishes its verified deployment graph to the broker before
  any provider in a Zone plane starts. The published row set is the verified
  document's own committed canonical bytes, one row per authority row, each
  carrying the digest over exactly the bytes the broker stores, and a binding
  row's relationship identity is resolved from the accepted graph rather than
  named by the publisher. Before this, nothing produced those rows: the
  broker held no projection for the Zone, its posture stayed unaccepted, every
  ordinary effect was refused as an unaccepted projection, and the graph that
  would refuse a `RoleBinding` granting its own creation was never evaluated.
- A plane the verified deployment graph does not describe publishes nothing,
  and a Zone with no verified graph refuses to start rather than publishing an
  empty projection that would read as authority with none.

### Fixed

- An authority publication over the broker socket no longer panics before it
  reaches the broker. The coordinator's blocking socket exchange runs on its
  own dedicated bounded worker, but it took the worker's reply with a blocking
  receive on the calling thread - and that calling thread is a runtime worker
  for every publication the daemon makes, so the origination link could never
  complete a round trip at all. It awaits the reply instead; only the
  dedicated worker blocks, which is where the blocking belongs. A unit test
  with a fixture link cannot find this, because the defect is in the link the
  unit tests replace.