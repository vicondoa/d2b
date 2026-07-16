# Constellation observability

**Diataxis category:** reference.

Constellation inspection is explicit and bounded. A realm controller may query
configured providers, but the local root does not become the owner of global
telemetry history, remote registries, or realm Relay/provider credentials.

## `d2b op inspect`

`d2b op inspect` reports the current local operation/realm posture:

- local workload and configured realm counts;
- bounded provider health summaries;
- optional bounded trace identifiers when supplied by the operator;
- degraded partial results for unavailable providers or sinks.

The command returns partial results instead of falling back to host
credentials, SSH, generic tunnels, or host-owned relay sessions.

## Trace context

Constellation inspection uses the existing `TraceContext` model. Trace fields are bounded
and optional; malformed trace context is rejected at the CLI boundary rather
than propagated into daemon, controller, provider, or telemetry surfaces.

## Redaction and cardinality

Observability surfaces must never contain payload bytes, argv, stdout/stderr,
provider tokens, credential material, full endpoints, host paths, or PII.
Labels are low-cardinality and limited to bounded operation/trace identifiers,
realm/node/workload kind, state, and redacted error classifications.

Graceful VM shutdown telemetry follows the same rule. Metric and span
attributes use bounded outcome enums such as `clean_guest_shutdown`,
`clean_vmm_cleanup`, `api_unavailable`, `timeout_exceeded`, and
`force_requested`; human summary strings and raw CH/QMP errors are not label
values. The applied graceful shutdown timeout and elapsed wait are numeric
`*_seconds` values, and historical degraded shutdown outcomes are exported as
bounded counter metrics for dashboards. Current uncleared degraded markers remain
available through `d2b host doctor`.

Observer/ops realm export is opt-in. Exporters must reuse existing
observability configuration and must not acquire new fd, pidfd, cgroup,
namespace, or long-lived socket authority beyond the established observability
surfaces.
