### Fixed

- A failed authority publication no longer takes every later mutation for its
  Zone with it. One Zone has at most one outstanding publication transaction,
  so a publication that failed at any step after staging held that Zone's
  single slot, and only the next daemon start released it. One refused fence
  therefore left the Zone refusing every later mutation by the leaked
  transaction's identity - the store's one-outstanding rule working exactly as
  declared, with no protocol path back - and the caller saw a bare
  `zone-unavailable`. `publish` now runs the same recovery table the restart
  path runs, over the same durable facts, before it stages anything: a
  candidate that reached no fence is released, one that did is replayed exactly
  as it committed, and one recovery cannot resolve still fences the Zone and
  refuses the mutation with that refusal. The store's own staging rule is
  unchanged.

### Changed

- The `device-worker-launch` check reports a refused `Device` teardown as
  separate short lines: the CLI's error envelope, the daemon's own account of
  the delete, both units' whole-boot account of the authority publication, and
  the Device row and its declared rows afterwards. The lane's guest console
  truncates one long line, so the single-line report this replaces lost exactly
  the tail a refusal is explained in. The publication capture is what named
  the refusal behind `d2b delete Device/tpm0`. No assertion changed.
