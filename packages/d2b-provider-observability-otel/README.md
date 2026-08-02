# d2b Provider observability OTEL

## Provider identity

This crate implements the optional `Provider/observability-otel` support
surface. It is not a bootstrap dependency and does not own authoritative
audit data.

## Config schema

The installation-wide configuration accepts only `selfMetrics.enable`.
Per-binding routing, quota, redaction, and transport settings belong to the
resource contract rather than this root configuration.

## Exported resource types

The Provider consumes the provider-neutral telemetry Service and Binding
contracts. Transport Endpoints remain private implementation details.

## Controllers / services / workers / binaries

The source contains bounded emitter-socket, ingress-policy, Provider-agent,
configuration, and self-metric projections. Full process launch remains owned
by the Process Provider.

## Placement and dependencies

The Provider runs as an ordinary optional process. Its bounded support code
depends only on the shared telemetry and audit contracts.

## RBAC requirements

Resource admission and bus authorization remain core-owned. The Provider
does not mint session authority or widen a caller's resource permissions.

## Security posture

Ingress validation is structural and occurs before capacity admission. Errors
use closed classes and do not echo rejected labels, values, paths, or
identities. OTEL telemetry never reads or writes the authoritative audit sink.

## State and telemetry

Emitter and ingress state is bounded and in-memory. Export loss degrades
telemetry only; it never blocks resource mutation or audit durability.

## Build and test

Run `cargo test --manifest-path packages/d2b-provider-observability-otel/Cargo.toml`
for the standalone hermetic unit and ingress-policy tests.
