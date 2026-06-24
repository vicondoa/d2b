# `sync.json` schema (`v2`)

Schema: [`sync.json`](./sync.json)

`sync.json` is the private synchronization and lock contract selected by
[ADR 0034](../../adr/0034-storage-lifecycle-restart-and-synchronization.md).
It declares framework locks, holders, fd inheritance policy, fd transfer
mechanism, acquisition order, stale-owner policy, adoption behavior, and
degraded-state handling.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `locks` — lock specs keyed by stable lock id.

## Contract notes

- New advisory file locks use Linux OFD locks (`F_OFD_SETLK`).
- Lock fds are opened with `O_CLOEXEC` and are not inherited by runner
  payloads unless the sync spec explicitly allows `SCM_RIGHTS` or explicit
  fd mapping plus a lease transfer record.
- Multi-lock operations acquire locks through the declared total order.
- Ambiguous owner state degrades/quarantines the protected scope rather than
  force-unlocking behind a possible live owner.
- Every host-mutable lock surface has one repair owner. New locks must be
  represented by a generated sync row and reconciled through that owner instead
  of through side-channel lock files or cleanup scripts.
