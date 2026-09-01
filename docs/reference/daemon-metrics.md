# Daemon metrics registry

**Diataxis category:** reference.

The daemon registry is an in-process, bounded metric surface. It is owned by
`d2bd` and does not imply an unauthenticated HTTP endpoint. Operators use the
configured observability Provider or `d2b op inspect` until an authenticated
scrape transport exists.

## Metric rules

Metric names use the `d2b_daemon_` prefix. Labels are closed enums or bounded
provider/component values. Never label a metric with a Zone name, Guest name,
Resource UID, operation ID, shell name, path, credential, PID, or error text.

## Current families

| Family | Meaning |
| --- | --- |
| `d2b_daemon_resource_state` | Count of observed Resource phases by closed ResourceType/state. |
| `d2b_daemon_resource_reconcile_total` | Controller reconcile outcomes by Provider, operation, and closed result. |
| `d2b_daemon_provider_request_total` | Typed Provider/controller request outcomes. |
| `d2b_daemon_broker_request_total` | Broker operation outcomes by closed op and result. |
| `d2b_daemon_broker_request_duration_seconds` | Broker round-trip latency. |
| `d2b_daemon_guest_lifecycle_total` | Guest start/stop/restart/deletion outcomes. |
| `d2b_daemon_guest_lifecycle_duration_seconds` | Guest lifecycle duration by closed operation/result. |
| `d2b_daemon_session_total` | ComponentSession and shell session outcomes. |
| `d2b_daemon_ownership_drift_total` | Count of ownership-marker drift observations. |
| `d2b_daemon_pidfd_table_size` | Number of broker runner pidfds currently observed. |
| `d2b_daemon_uptime_seconds` | Seconds since d2bd started. |

Provider, ResourceType, operation, phase, and outcome values are defined by
the corresponding Rust enums and generated contracts. Free-form diagnostics
remain in logs and redacted audit records, not metric labels.

## Inspection

```bash
d2b op inspect --json
d2b host doctor --read-only
```

If metrics are unavailable, the daemon reports that condition explicitly. It
does not claim that the public `SOCK_SEQPACKET` socket is an HTTP endpoint or
read private bundle state to synthesize labels.
