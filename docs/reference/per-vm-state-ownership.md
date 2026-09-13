# Per-Guest state ownership

**Diataxis category:** reference.

Host state is owned by the current Zone, Guest, Provider, and broker
contracts. This page keeps the stable name for compatibility with existing
links; new prose should say Guest rather than VM.

## Declared posture contract

Required host permissions for the state farms and shared directories are a
**declared contract**, not an emergent property of whoever created each level:

- Declaration: [`packages/d2b-broker/src/ops/state-posture-contract.json`](../../packages/d2b-broker/src/ops/state-posture-contract.json).
  Per path level it names the creator, owner/group/mode (exact, or the
  minimum a creation-time posture guarantees), POSIX ACL entries and who
  applies them, and the rights each principal (`root`, `d2bd`, the `d2b`
  lifecycle group, runner `users`, and any principal in none of them) needs or
  is refused.
- Consumers read that one file: the broker posture code embeds and applies the
  `guest-store-view` rows
  (`packages/d2b-broker/src/ops/store_view_posture.rs`), the Nix provisioning
  derives the `shared-run-dir`/`state-root` tmpfiles lines from the rows
  (`nixos-modules/host-daemon.nix` via `nixos-modules/state-posture-contract.nix`),
  and the live validation asserts the host against every row
  (`tests/host-integration/state-posture-contract.nix`, run by
  `make test-host-integration` with `D2B_VM_CHECK=state-posture-contract`).
  Do not restate a posture value in prose: edit the declaration.

The declaration also names the **anchor-open rule**: an anchor component of a
path walk is opened `O_PATH` (the consumer needs search on the parent and no
rights on the component itself); only a leaf the consumer owns is opened
`O_RDONLY`. The daemon's anchored store-view walk implements it
(`packages/d2bd/src/resource_plane_v3.rs::open_anchored_directory`), and the
broker postures the broker-created levels above a per-Guest state dir to grant
search only (`posture_daemon_traverse_ancestors`). No consumer may need read
on a level it does not own; a walk that demanded read on a traversal-only
level fails `EACCES` against the POSIX ACL mask.

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
- [`store-sync.md`](./store-sync.md)
- [`../explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
- [`cgroup-delegation.md`](./cgroup-delegation.md)
- [`../../AGENTS.md`](../../AGENTS.md)
