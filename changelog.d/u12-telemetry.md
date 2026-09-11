### Changed

- Converted the telemetry semantic-binding family
  (`telemetry.d2bus.org.TelemetryService` / `telemetry.d2bus.org.TelemetryBinding`)
  to the v3 `ResourceDriver`/`ResourceDriverFactory` contract:
  `TelemetryDriverFactory` validates the Provider-declared spec envelope,
  adopts the manager-owned child set on recover, ensures provider-declared
  children through the manager (commit before spawn), retires obsolete owned
  children Endpoint-first and Process-last, publishes its status in memory,
  and requeues itself while not converged (the preserved 5s resync). The
  collector and forwarder processes are Process resources the Process
  controller launches.
- Removed the telemetry `d2b.d2bus.org/binding-children` finalizer
  machinery and the telemetry-only status-redaction helpers: the v3 manager
  already cascades owned children and holds the parent row until the last
  child retires, and nothing persists a telemetry status, so the finalizer
  and the store-side sanitizers had no work left to do.
- Deleted the old-plane U12 observability controller runner (provider table
  row, runner tasks/locks, readiness gate) now that both of its families,
  activation and telemetry, are served by the v3 plane.
