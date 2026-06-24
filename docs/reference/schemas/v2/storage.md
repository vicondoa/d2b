# `storage.json` schema (`v2`)

Schema: [`storage.json`](./storage.json)

`storage.json` is the private storage lifecycle contract selected by
[ADR 0034](../../adr/0034-storage-lifecycle-restart-and-synchronization.md).
It inventories nixling-managed paths, their lifecycle, owners, ACL posture,
cleanup policy, restart/adoption policy, degraded-state taxonomy, and static
remediation IDs.

The artifact is private (`root:nixlingd` `0640`) and is broker authority
only when referenced through opaque bundle ids. Daemon-owned degraded ledgers
and operator-facing status output are diagnostics, not privileged repair
authority.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `roots` — declared root directories such as `/etc/nixling`,
  `/var/lib/nixling`, and `/run/nixling`.
- `paths` — storage path specs with kind, lifecycle, owner/group/mode,
  access/default ACLs, cleanup/repair/restart/adoption policy, sensitivity,
  and invariants.
- `restartPolicies` — per-VM/per-role restart classes and adoption inputs.
- `degradedStates` — closed degraded-state reason slugs and storage class.
- `remediations` — static remediation IDs and human-facing commands.

## Contract notes

- Pidfds are never persisted. Restart policies may persist only logical
  adoption metadata such as cgroup leaf and identity checks.
- Runtime paths are not swept on daemon restart until live-owner adoption or
  dead-owner proof has completed.
- Path hashes are for structured audit/local doctor output only; they must
  never become metric labels.
- Every host-mutable path has one repair owner. New paths must either reuse an
  existing generated storage row or add a row whose repair policy routes
  reconciliation through that single owner; ad-hoc chmod/chown/cleanup paths are
  not part of the contract.
