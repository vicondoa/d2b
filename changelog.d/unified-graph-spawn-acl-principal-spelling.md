### Fixed

- The `state-posture-contract` host check no longer reports a Zone serving
  worker's own traversal grant on the host state root as an undeclared ACL
  entry. The broker's spawn preflight grants the runner principal search
  (`u:<principal>:--x`) on every ancestor above the served view root that no
  unprivileged principal can already search, and `/var/lib/d2b` is such an
  ancestor: the grant is the intended, minimal one and it is the runner's own
  principal, not an inherited, default, or group-resolved entry. The check
  already derives those spawn-time grants structurally instead of reading them
  from the declaration, because the account name carries the Zone and no
  host-global declared level can name one. What it derived them by was the
  numeric `ps -eo uid=`, while the ACL entries it compares are read without
  `--numeric` and therefore carry the account NAME. That worked only while
  runner principals were uids minted per Guest with no passwd entry; host
  account provisioning gave every runner a named account, so from then on the
  numeric prefix matched nothing and each named runner's grant read as drift.
  The check now resolves each live worker uid to the spelling `getfacl`
  actually renders it as, and the shape rules it allows are unchanged: search
  on a level with a deeper grant of the same principal, or that principal's
  own topmost grant. An entry for a principal no live worker runs as, and a
  wider grant above a worker's own leaf, are still refused, and unit tests pin
  both directions.