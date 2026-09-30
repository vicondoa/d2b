### Added

- `d2b host reset`: the offline, ownership-bounded clean break the new
  release starts from. The verb resolves no Zone and opens no daemon
  socket, so it runs on a host with `d2bd` stopped. It invokes a one-shot
  broker ownership runner (`d2b-broker reset`) that admits the reset
  Operation from the verified new deployment graph under explicit local
  operator authority through the one evaluator, and never opens the
  previous release's SpecStore: the deletion inventory is derived from
  exactly one input, the self-hash-verified deployment graph's `owned[]`
  list, so an unreadable or old-format store changes nothing about what
  the reset removes. Nothing is migrated and no old policy is executed.
- The ownership boundary the reset acts inside
  (`d2b-host::ownership_matrix`). A foreign ownership marker, a symlink on
  any component of an owned path, an owned path outside the deployment
  root, an owned path on another filesystem, and an external Volume source
  the owned set would swallow each fail closed; a marker is never an
  authorization to overwrite. Removal is unlink-only and no-follow: a
  store-view hardlink farm is emptied one level and never descended into,
  so a `gcroots` link into the system store is unlinked as a name and no
  shared inode is ever chmod-ed or chown-ed.
- Live drain evidence is positive or it is nothing. Live cgroup members,
  managed runner records, ownership markers, non-acquirable lease locks,
  and an active host generation each refuse the reset on their own;
  fresh empty state - exactly what the new model wants to look like - is
  never the drain proof.
- Apply mode removes the verified inventory and then establishes the fresh
  deployment root and a NEW incarnation, so the next boot initializes
  rather than resynchronizing a rollback. A repeated completed reset is a
  no-op that reports every owned path as absent.