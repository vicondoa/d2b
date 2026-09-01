### Added

- Join broker and resource audit records with one Zone-scoped opaque operation
  key, fail closed on durability disagreement, and require synchronized audit
  segment data and directory metadata before reporting privileged success.
- Defer broker evidence for successful Zone activation outboxes until the
  broker records it, then drain the matching outboxes through the serialized
  store writer before publishing a generation.
- Make resource outboxes replayable with deterministic mutation identities,
  ordinals, timestamps, migration of older rows, terminal deny/error records,
  retention checkpoints, and bounded exports.
- Bound telemetry frame count, bytes, age, and retries while redacting
  identity-bearing values before export; enforce the same typed frame policy
  at retained emitter and ingress boundaries.
- Retain only observability Provider foundation contracts and bounded ingress;
  production OTLP/vsock/ComponentSession adapters, collector/forwarder/exporter
  loops, journald integration, projection/share, and resource ownership remain
  a separate completion unit.
