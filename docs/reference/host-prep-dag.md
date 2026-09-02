# Host preparation graph

**Diataxis category:** reference.

The broker-owned host preparation graph establishes only d2b-owned state
needed by the current Zone and Guest Resource graph. It replaces old
per-workload service ordering and never accepts raw paths or caller-owned
steps.

## Ordered domains

The current graph checks, in dependency order:

1. Guest/Resource ownership and host-key evidence;
2. NetworkManager and firewall ownership;
3. delegated cgroup and device prerequisites;
4. typed Network/TAP/sysctl operations; and
5. broker fd handoff before a Provider runner starts.

Each step carries an opaque bundle reference. The broker resolves the path,
identity, and operation from its trusted bundle and appends a redacted audit
record.

## Failure semantics

The graph is fail-fast for one Zone/Guest and independent across unrelated
Zones. A failed step leaves the Guest Pending or Degraded; no later Process
effect is dispatched and no fallback service is started. Retrying rebuilds
the graph from the current Resource generation.

Foreign ownership markers, ancestor cgroup operations, missing identity
evidence, and uncertain cleanup fail closed. d2bd remains available for
read-only status and diagnosis.

## Inspection

```bash
d2b host check --json
d2b host prepare --dry-run
d2b guest status <name> --zone <zone>
```

See [host preparation](../how-to/host-prepare.md),
[the daemon lifecycle](../explanation/daemon-lifecycle.md), and
[the privileges reference](./privileges.md).
