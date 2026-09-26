### Fixed

- `d2b host prepare`, `d2b host destroy`, and `d2b host reconcile`
  invoked without `--dry-run` or `--apply` now exit 78 with the
  documented `--apply-or-dry-run-required` envelope, as `d2b host
  validate` already did. `host prepare` and `host destroy` previously
  exited 2 with `ref-invalid`, and `host reconcile` exited 78 under that
  same wrong code. Each envelope names the offending verb. The
  `--network` requirement on `host reconcile` keeps its own
  `ref-invalid` refusal.

### Changed

- The route-lease revocation flag in the bus route registry is an
  `AtomicBool` latch rather than a `Mutex<bool>`. Revocation is one-way,
  so revocation is a release store and every check an acquire load: the
  weakest correct ordering, and no guard is held across the caller's
  work. The lease API and its revocation semantics are unchanged.
