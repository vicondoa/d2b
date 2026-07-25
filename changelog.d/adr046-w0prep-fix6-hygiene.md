### Fixed

- The `d2b host check` socket-permission diagnostic now directs operators to
  restart `d2bd.service` - which recreates `/run/d2b/public.sock` and
  re-asserts its mode, owner, and group on bind - instead of a `d2bd.socket`
  unit the framework does not declare, and its documentation link now resolves.
- Every `d2b host check`, `host prepare`, and `host destroy` error code now
  resolves to a stable anchor in `docs/reference/error-codes.md`, so the
  `docs_anchor` an operator follows from a diagnostic lands on a real entry
  rather than a dangling link.
