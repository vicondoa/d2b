### Added

- Guest target layer (U13): `TargetDirectory` resolves a resource's declared
  `Host/<name>`/`Guest/<name>` execution reference into a realization handle,
  records exactly one target assignment per Host-zone resource, and binds a
  guest assignment to the authenticated ComponentSession generation that
  minted it.
- `GuestTargetRuntime` and the `GuestTargetControl` port: target-local
  realizations keyed by the owning Host-zone resource identity. A realization
  is an implementation detail of the target runtime - no desired spec of its
  own, never a second API-visible resource in the guest namespace.
- `TargetBinding`: the directory-backed target handle a resource actor holds.
  Every operation re-validates against the live session, so a handle retained
  across a reconnect can never realize, observe, or delete for its successor.

### Changed

- Renamed the generic Guest target-control session purpose away from the
  literal `zone-link` to `component-session`, in the session identity, the
  enrolled endpoint policy, the generation-discovery profile, and the ZoneLink
  route admission's Gateway-Guest carriage profile (R20). ZoneLink resource
  semantics are untouched: ZoneLink traffic keeps its own purpose, roles and
  transport profiles, and a ZoneLink that runs in a guest consumes the generic
  target path.

### Fixed

- Guest disconnect marks target-dependent observed state unavailable without
  deleting or moving desired resources, and reconnect re-binds the affected
  assignments through target-local discovery and adoption instead of
  inheriting the lost session's authority (F5, R18, R21).
