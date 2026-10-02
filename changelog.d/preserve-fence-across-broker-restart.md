### Fixed

- A broker restart no longer erases the fence a Zone's store still owes a
  publication for. The open barrier wrote `reconciling` over every known Zone,
  which is what stops a restarted broker from serving the projection it
  happened to persist, but it also destroyed the one record
  `commit_change_locked` admits against: a `CommitChange` is served only under
  a live fence. A store that had committed a candidate durably and lost the
  `CommitChange` that publishes it therefore had no way to finish it after the
  broker came back - recovery replayed the exact commit and the broker refused
  it for want of a fence that had been there all along, so adoption failed
  closed and the Zone could never serve again. The barrier now keeps a prepared
  record that still names a forward commit over the accepted cursor, and writes
  `reconciling` over everything else, including the carried-forward outstanding
  marker a resynchronization leaves behind, whose two cursors are the same one
  and which names no commit to replay. A preserved fence is still exactly the
  record it was: the replay must name the same transaction, the same reserved
  revision, the same accepted predecessor, and rows that re-derive to the same
  digest, or it is refused by the same code a live fence gives, and the record
  is gone once it is discharged. Such a Zone was admitting nothing under either
  name, so the restart still serves nothing.
- A restarted manager can now replay a transaction a previous process fenced.
  The manager's publication coordinator matched every commit against a
  per-process record of the fences it had taken, and that record died with the
  process, so recovery's replay was refused as an unmatched completion before it
  ever reached the broker. `AuthorityPublisher` gained `adopt_committed` beside
  `commit`: recovery uses it, and it exists because the two have different
  preconditions, not because the commit is trusted more. The authority on
  whether a fence still exists is the broker, which re-checks the exact
  prepared identity, the exact predecessor, and the exact committed bytes before
  it installs anything. `commit` itself is unchanged.
