# Observability Provider

**Diataxis category:** reference.

Observability is a Provider projection in the Zone resource graph. A Zone
may own an observability Provider and attach its status to the Guests and
Processes it serves. There is no auto-declared telemetry environment or
host-global observability Guest.

The Provider owns collector processes, endpoints, storage, export policy,
credentials, and retention effects through its assigned controller. Public
status is bounded and redacted.

```bash
d2b provider list --zone work
d2b provider status observability --zone work
d2b op inspect --json
```

Keep exporter credentials and remote configuration inside the Provider-owned
execution context. A missing or degraded sink must not cause a lifecycle
fallback or expose private telemetry data.
