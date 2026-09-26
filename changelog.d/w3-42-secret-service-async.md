### Fixed

- Cached user-session reuse in the secret-service provider no longer inverts
  the lock order between the session map and the per-user session map, so a
  concurrent cached-key admission cannot stall behind an acquire that is
  waiting for the provider to unlock.
- Credential secret-service table-driven tests now name the failing case when
  an assertion trips, so a regression reports which placement, binding, or
  alias text was rejected instead of only the line number.
