### Changed

- `d2b-provider-notification-desktop` re-exports its session admission
  types (`AdmissionError`, `AdmissionPurpose`, `SessionEvidence`,
  `TransportClass`) straight from the `admission` module. The private
  three-line re-export shim is gone; the public type paths are
  unchanged, so no consumer changes.
- The clipd host-selection, niri probe failure, desktop notification
  failure, picker terminate/kill failure, and data-control connect
  events now emit through `tracing` with named fields (`quality`,
  `mimes`, `secret`, `pid`, `protocol`, `error`) instead of
  interpolated messages.

### Fixed

- `clipd_host::audit::AuditEvent` no longer derives `Deserialize`. Its
  `mime_type` field serializes through the bounded-MIME adapter with no
  matching deserializer, so a read-back would have accepted unbounded
  values the writer never emits. The record is write-only.
