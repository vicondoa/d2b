# Per-Guest state ownership

**Diataxis category:** reference.

Host state is owned by the current Zone, Guest, Provider, and broker
contracts. This page keeps the stable name for compatibility with existing
links; new prose should say Guest rather than VM.

## Ownership rules

- The broker owns only delegated cgroup leaves, sockets, devices, locks, and
  anchored paths named by the trusted bundle.
- The Guest controller owns Guest lifecycle state and child Resource
  ownership, not host files.
- Store synchronization owns the closure-only Guest store view.
- TPM and credential Providers own their persistent state.
- Foreign markers, wrong types, symlinks, owner drift, and uncertain state
  fail closed.

Never run recursive chmod, chown, or setfacl across a Guest store view or
private runtime tree. Do not sweep `/run/d2b` or mutate a parent cgroup.

## Inspection

```bash
d2b guest status <name> --zone <zone>
d2b host check --json
d2b host doctor --read-only
d2b audit --json
```

The public output contains bounded ownership and degraded-state metadata, not
raw host paths, credentials, PIDs, or private handles.

## Related contracts

- [`store-lifecycle.md`](./store-lifecycle.md)
- [`../explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
- [`cgroup-delegation.md`](./cgroup-delegation.md)
- [`../../AGENTS.md`](../../AGENTS.md)
