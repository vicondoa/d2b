### Changed

- The public `auditResponse` page end is one closed value instead of the
  `complete`/`nextCursor` boolean pair: `AuditResponse` now carries
  `AuditPageEnd` (`Complete` or `More(cursor)`), so a final page with a cursor
  and an incomplete page without one can neither be built nor decoded. The
  serialized keys, their order, and the admission error text are unchanged, so
  the published v2 wire schema and the CLI pagination contract do not move.
  The crate-local `validate_audit_page` copy is gone; the page-end constructor
  reports the shared `d2b_contracts::audit_wire::AuditPageError` classes, and
  the daemon refuses a broker page that pairs an incomplete page with no
  cursor instead of reporting it as final.
- Every mutating-verb request now selects a `MutationMode` (`dryRun` or
  `apply`) instead of carrying two independent booleans, so the request that
  selects neither mode, which the daemon refuses, is no longer representable.
  `dryRun`, `apply`, and `json` remain the serialized keys in the same order,
  and the daemon's raw-frame path still answers a request that selects neither
  mode with the documented `invalid-request` envelope; only the flags-pair
  check inside `mutating_verb_preflight` is deleted.
