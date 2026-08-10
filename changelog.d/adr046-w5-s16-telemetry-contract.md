### Fixed

- Single-sourced the closed telemetry label and OTEL resource-attribute
  policy in `d2b-contracts`, with distinct fail-closed emitter and
  observability Provider validators consuming the same registry.
