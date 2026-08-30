# Host preparation

Host preparation is the broker-owned prerequisite path for the current Zone
resource plane. It establishes only d2b-owned host state needed by d2bd and
the assigned Providers.

## Trust boundary

d2bd is unprivileged. It sends typed, closed-enum requests to
`d2b-broker`, which:

- resolves paths and identities from the trusted bundle;
- checks Zone, Resource, Provider, generation, and revision evidence;
- mutates only d2b-owned cgroup, network, firewall, socket, and device state;
- records a redacted audit decision; and
- fails closed on foreign markers, ambiguity, or unsafe paths.

The broker never accepts a raw host path, arbitrary command, caller-owned
credential, or parent-cgroup kill request.

## Host operations

```bash
d2b host check --json
d2b host prepare --dry-run
d2b host prepare --apply
d2b host reconcile --dry-run
d2b host reconcile --apply
d2b host doctor --read-only
```

`check`, `doctor`, and dry-run forms are read-only. Apply forms mutate only
the ownership markers and delegated leaves named by the current bundle.
Foreign nftables, NetworkManager, systemd-networkd, cgroup, TPM, socket, and
device state is preserved byte-for-byte.

## Cgroups and pidfds

The broker delegates `/sys/fs/cgroup/d2b.slice` and places each runner in a
leaf scoped to its Zone, Resource, and role. It never mutates the cgroup root,
threaded groups, partition roots, or an ancestor `cgroup.kill`.

Runner pidfds cross the private broker socket with `SCM_RIGHTS`. d2bd
observes and signals through pidfds; the broker remains the sole parent and
reaper. Restart adoption checks persisted PID/start-time evidence before
reopening a pidfd and quarantines drift.

## Recovery

If preparation fails, inspect the typed error and audit record, correct the
named host prerequisite or ownership marker, and rerun `d2b host check`.
Never use recursive chmod/chown, broad `/run/d2b` cleanup, direct privileged
helpers, or a foreign-state overwrite as a workaround.

See [host preparation](../how-to/host-prepare.md),
[the daemon lifecycle](./daemon-lifecycle.md), and
[the privileges reference](../reference/privileges.md).
