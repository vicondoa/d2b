### Fixed

- Corrected the observability Provider specification so it no longer directs
  the `observability-otel` Provider to write authoritative audit records. The
  Provider is the subject of a `SessionConnect` record, not its author: session
  admission remains the sole writer, telemetry and authoritative audit keep
  their separate writer paths, and the Provider crate takes no audit or core
  telemetry dependency. The closed metric-label policy is recorded as
  single-sourced in the public neutral contract so telemetry ingress and the
  core emitter can never disagree about which labels are forbidden.
