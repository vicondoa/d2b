# Prepare a d2b host

Host preparation establishes the prerequisites for the current Zone daemon,
broker, and Provider effects. It does not create a per-Guest service or accept
caller-owned host paths.

## Inspect first

```bash
d2b host check --json
d2b host doctor --read-only
```

Review unified cgroup v2, KVM and device availability, NetworkManager and
nftables ownership markers, required host groups, and the three root-visible
units:

```text
d2bd.service
d2b-broker.socket
d2b-broker.service
```

## Prepare and reconcile

Use the typed host operation in the consumer's selected Zone context:

```bash
d2b host prepare --dry-run
d2b host prepare --apply
d2b host reconcile --dry-run
d2b host reconcile --apply
```

Only d2b-owned state may be changed. The broker preserves foreign nftables,
NetworkManager, systemd-networkd, cgroup, socket, and device state byte for
byte. Missing or replaced markers fail closed.

## Verify

```bash
d2b host check --json
d2b host doctor --read-only
d2b zone list
d2b guest list --zone local-root
```

Host operations return typed errors for missing prerequisites, authorization,
foreign ownership, capability gaps, and uncertain cleanup. Do not work around
one by changing permissions recursively, sweeping `/run/d2b`, or running a
direct privileged helper.

## References

- [`../reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)
- [`../reference/host-prep-dag.md`](../reference/host-prep-dag.md)
- [`../reference/privileges.md`](../reference/privileges.md)
- [`../explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
