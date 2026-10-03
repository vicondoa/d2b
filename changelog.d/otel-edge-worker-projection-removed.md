### Removed

- `Provider/observability-otel` no longer projects one target-local `Process`
  row per Guest-scoped `TelemetryBinding`. The row declared the worker
  template `otel-collector-edge`, which no signed Provider artifact pinned, so
  it never received a trusted launch intent and the daemon refused its launch
  terminally with `process-template-unavailable`. Nothing in production read
  that row or its phase - the collector and forwarder a TelemetryBinding owns
  are its runtime children's own declarations (`otel-collector` and
  `otel-vsock-forwarder`) - so the Zone carried a worker the tree could not
  install and did not need. The Provider's configuration validation is
  unchanged.

- `observability-otel` leaves the closed Provider-projection owner list. The
  list is the set of projections a Zone bundle has an owner for, and a
  Provider that projects nothing has no projection to own.

### Fixed

- A declared worker template that can never launch is a false capability: a
  Zone that declared an observability edge binding reported a `Process` row
  as desired, and the refusal landed per reconcile rather than at the
  declaration that could not be honoured.
