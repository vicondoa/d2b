### Changed

- The Wayland display launch boundary now reports failures as the
  closed `LaunchError` enum (`SessionInvalid`, `TicketInvalid`)
  instead of a bare string, so callers match the failure variant. The
  observable failure codes `display-grant-session-invalid` and
  `display-launch-ticket-invalid` are unchanged.

### Fixed

- `DisplayRuntime` now returns the dynamic principal leased during
  display reconciliation to the controller's bounded pool when
  finalization runs, after both workers are confirmed terminal and
  deleted. The release receipt previously had no constructor, so the
  lease was never returned and a session that launched workers held a
  pool account until the daemon restarted.
