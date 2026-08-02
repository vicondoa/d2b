# `d2b-provider-observability-otel`

## Provider identity

| Provider name | `observability-otel` |
| --- | --- |
| Resource reference | `Provider/observability-otel` |
| Role | Optional, non-bootstrap telemetry ingestion and export |
| Semantic services | `telemetry.d2bus.org.TelemetryService`, `telemetry.d2bus.org.TelemetryBinding` |

The Provider is optional and ordinary. Zone startup and authoritative audit
remain independent of its readiness.

## Config schema

The installation-wide configuration accepts only the bounded boolean
`selfMetrics.enable`. Per-binding routing, quota, redaction, and transport
settings belong to the provider-neutral resource contract and its strict
Provider extension rather than this root configuration. Unknown fields and
non-boolean values are rejected.

## Exported resource types

The Provider consumes the provider-neutral
`telemetry.d2bus.org.TelemetryService` and
`telemetry.d2bus.org.TelemetryBinding` contracts. Authority and projection
services carry the same semantic identity; Endpoint resources and socket
names remain private implementation details.

## Controllers / services / workers / binaries

The source contains the session-bound Provider agent, bounded emitter socket,
structural metric ingress gate, strict configuration parser, and closed
self-metric descriptors. Full collector and forwarder process launch remains
owned by the Process Provider boundary.

## Placement and dependencies

The Provider runs as an ordinary optional process in its owning Zone. Its
workspace dependencies are limited to `d2b-contracts` and
`d2b-provider-toolkit`, the admitted neutral Provider boundaries. The toolkit
supplies the diagnostic audit ring and session-facing values; authoritative
audit durability and core telemetry emission stay outside this crate.

## RBAC requirements

Resource admission, ComponentSession authority, bus authorization, and
cross-Zone projection routing remain core-owned. The Provider accepts only
already-admitted session context and never mints authority or widens a
caller's resource permissions.

## Security posture

Ingress validation is structural and occurs before capacity admission. Errors
use closed classes and do not echo rejected labels, values, paths, or
identities. The metric policy rejects identity keys, identity suffixes, and
trusted resource-identity canaries. OTEL telemetry never reads or writes the
authoritative audit sink, and journald filtering is opt-in and redacts
credential, secret, token, password, and path-shaped messages.

## State and telemetry

Emitter, ingress, quarantine, and diagnostic audit state is bounded and
in-memory. Export loss degrades telemetry only; it never blocks resource
mutation or audit durability. Resource identity belongs in the closed OTEL
resource-attribute set, never in metric dimensions.

## Build and test

From `packages/`, run:

```bash
cargo nextest run -p d2b-provider-observability-otel --lib --tests
cargo clippy -p d2b-provider-observability-otel --all-targets -- -D warnings
cargo fmt --manifest-path d2b-provider-observability-otel/Cargo.toml -- --check
```

The crate's normal tests are hermetic. The scenario declarations in
`integration/` are exercised by the existing container or host-integration
lane once the production Provider supervisor, ComponentSession stream, OTLP
exporter, and NixOS adapter are present.
