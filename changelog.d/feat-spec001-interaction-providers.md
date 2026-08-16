### Fixed

- Fixed interaction composition to derive display identity from committed
  WaylandSession state and authorize multi-Guest clipboard and notification
  operations against authenticated Provider routes without weakening
  fail-closed identity checks.
- Fixed picker materialization to authorize the committed Guest, Zone, and
  route before consuming its one-use receipt.
- Fixed display reconciliation to require the committed Wayland observer User
  and preserve the authenticated display identity across restart reconciliation.
