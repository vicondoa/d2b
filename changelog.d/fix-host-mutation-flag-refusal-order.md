### Fixed

- `d2b host prepare` and `d2b host destroy` without `--dry-run`/`--apply` are
  refused with the documented `--apply-or-dry-run-required` envelope at exit 78
  even when the public socket is unreachable. The missing-mode refusal is a
  property of the invocation, so it is now emitted before the Zone is resolved
  rather than after; previously the daemon reachability check ran first and the
  same invocation reported `zone-unavailable` at exit 1.
