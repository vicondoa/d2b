# U96 d2b-telemetry

Lean already. Ship.

The bounded-emitter / audit-hash-chain / metric-policy core: every item here is
either a contract DTO the workspace's telemetry, provider, and broker surfaces
admit, or a bounded primitive the ringer and emitter paths drive. Full
in-crate read + workspace caller trace; zero-caller count = 0.

## Checked
- emitter.rs: `BoundedEmitter::new/new_with_limits`, `emit/emit_raw_frame`,
  `drain`, `buffered_*`, `drops`, `EmitOutcome`, `EmitterError` - all live
  production paths in d2bd (emitter.rs ring + bounded unix-datagram sink),
  d2bd-runtime (bounded emitter framer), d2b-provider-telemetry service, and
  the provider-toolkit base emitter; bounded queue/prune/redact run in the
  toolchain's own conformance + this crate's bounded-ring tests
- audit_hash.rs: `AuditHash::parse/from_bytes/as_str`, `AuditChainLink`
  `new/verify/verify_at`, `genesis_hash`, `record_hash`, `payload_hash`,
  `is_canonical_digest` - consumed by d2b-audit's canonical audit writer +
  d2bd's closed audit chain; no hand-rolls
- `Signal::{Metric, Trace, Log}` + `as_str`, `TelemetryFrame` codec -
  encode/decode live in d2bd emitter + telemetry service envelope path
- metric_label_policy.rs + meter_registry.rs + session_metrics_sink.rs -
  validated data-point policy + bounded families consumed by d2bd-runtime +
  d2b-provider-telemetry; no unread table
- redaction_guard.rs - field/key redaction + forbidden-table closed policy,
  exercised by d2bd redaction tests + telemetry service emission

No net lines to cut; no hand-rolled stdlib surface beyond the shared
`ResourceUid`/digest vocabulary already migrated by the cross-cutting
findings this crate shares. Ship.
