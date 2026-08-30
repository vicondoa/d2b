# Recover a broker user-namespace failure

The broker creates approved runner namespaces before launching Provider
effects. A failure is fail-closed: the Guest remains degraded and no
caller-owned fallback is attempted.

## Inspect

```bash
d2b host doctor --read-only
d2b guest status <name> --zone <zone>
journalctl -u d2b-broker.service -u d2bd.service --since today
```

Look for the typed failure, Provider, Zone, Guest generation, and operation
identity. Do not infer ownership from a PID or Guest name alone.

## Common causes

- unified cgroup v2 or required controllers are unavailable;
- the broker cannot create the approved user namespace;
- a delegated cgroup leaf or device ownership marker is foreign;
- a signed Provider template or artifact commitment is stale; or
- persistent TPM, socket, or lock state is missing or replaced.

Correct the host prerequisite or restore the named owner's state, then rerun
the read-only checks. Never use broad chmod/chown, parent-cgroup kill,
`/run/d2b` sweeps, or a direct privileged helper.

## Retry

```bash
d2b guest start <name> --zone <zone> --dry-run
d2b guest start <name> --zone <zone> --apply
```

If the Guest is finalization-blocked, keep the finalizer until the controller
can prove that all owned descendants and session state are drained. The
broker's typed audit record is the authoritative evidence for host mutation.

See [host preparation](./host-prepare.md),
[the daemon lifecycle](../explanation/daemon-lifecycle.md), and
[the privileges reference](../reference/privileges.md).
