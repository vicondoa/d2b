### Changed

- Moved semantic audio, USBIP, security-key, and telemetry Binding realization
  onto deterministic Provider-owned child intents reconciled through Core.
  Child creation, repair, and teardown preserve explicit Binding ownership,
  target placement, optimistic identity fencing, and Endpoint-before-Process
  deletion ordering without creating consumer Bindings from Services.
- AudioBinding status now persists channel grants, levels and gains,
  arbitration, enforcement posture, and the last applied host/guest path
  alongside the observed Service and realization references.

### Fixed

- Admit Binding Endpoint children by validating their reserved `providerRef`
  with the canonical full `ResourceSpec`.
