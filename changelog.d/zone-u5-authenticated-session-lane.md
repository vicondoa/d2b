### Added

- Added authenticated ZoneLink and Guest ComponentSession driver admission
  that consumes the sealed route profile and its owning authenticated session,
  retaining the single-owner session authority in a non-cloneable driver lane.

### Security

- Route admission revalidation fences stale or revoked lanes, while
  purpose, role, service, target, Zone, peer, and generation substitutions
  fail closed before driver traffic is admitted.
