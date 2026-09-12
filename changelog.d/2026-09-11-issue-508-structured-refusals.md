### Changed

- Driver failures now carry a structured detail - the operation, the stage,
  the failure kind, the verdict, and the compared values (redacted where the
  value is secret) - and the operator log line and the wire failure layer are
  projections of that one value, so what an operator reads is what a test can
  assert.
- The verdict distinguishes structural outcomes: `NotYet` defers and requeues,
  `Refused` is a decision against the row, and `Error` is an operational
  failure; only `Refused` and terminal `Error` end a row's reconcile.
- A registry of failure kinds records what each kind means and its likely
  cause, and the reference document is generated from it rather than
  hand-maintained.

### Fixed

- A row that is simply not there yet reports "not yet" instead of a terminal
  refusal, so a binding whose parent row has not been observed defers rather
  than failing its reconcile permanently.
