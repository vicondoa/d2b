# Constellation observability

**Diataxis category:** reference.

Constellation inspection is explicit and bounded. The host may fan out current
state requests through configured local and gateway entrypoints, but it does
not become the owner of global telemetry history, remote registries, or realm
relay/provider credentials.

## `nixling op inspect`

`nixling op inspect` reports the current local operation/realm posture:

- local VM and gateway counts;
- configured host-resident and gateway-backed realms;
- optional bounded trace identifiers when supplied by the operator;
- degraded partial results for unavailable gateways or sinks.

The command returns partial results instead of falling back to host
credentials, SSH, generic tunnels, or host-owned relay sessions.

## Trace context

Wave 19 uses the existing ADR032 `TraceContext` model. Trace fields are bounded
and optional; malformed trace context is rejected at the CLI boundary rather
than propagated into daemon, gateway, provider, or telemetry surfaces.

## Redaction and cardinality

Observability surfaces must never contain payload bytes, argv, stdout/stderr,
provider tokens, credential material, full endpoints, host paths, or PII.
Labels are low-cardinality and limited to bounded operation/trace identifiers,
realm/node/workload kind, state, and redacted error classifications.

Observer/ops realm export is opt-in. Exporters must reuse existing
observability configuration and must not acquire new fd, pidfd, cgroup,
namespace, or long-lived socket authority beyond the established observability
surfaces.
