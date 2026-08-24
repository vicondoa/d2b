### Changed

- The runtime execution-budget ledger now enforces a pinned closed census: it
  requires a census, records advisory per-test wall clock from warmed,
  crate-qualified libtest streams, records enforced aggregate process CPU for
  each complete crate invocation, reproduces the expected test and crate sets
  exactly, rejects census id loss and repetition mismatch, and runs as a
  required Layer-1 job. It holds no baseline and makes no
  historical-regression claim.

### Security

- The runtime ledger validates a short closed runner-label grammar, bounds
  printable test identifiers, row counts, and libtest input size, and rejects
  control characters both when emitting and when loading ledgers, so host
  paths, multi-line log injection, and unbounded artifact cardinality
  can no longer reach the recorded or printed output.
