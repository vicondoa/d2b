### Fixed

- **A Zone's own `Role` and `RoleBinding` rows are applied before its resource
  manager spawns.** The daemon opened each Zone's plane by spawning the manager
  first and ingesting the verified Nix bundle afterwards, so on a cold start
  every authority reader that runs before that ingest - the broker's per-Zone
  projection, the session layer, the manager-boundary admission - saw a Zone
  with no accepted graph at all, even though the bundle declares one. The plane
  now commits the rows an accepted graph is built from through the same fenced,
  published, and acknowledged store path the foundation seed uses, before the
  manager exists, so the manager's own initial load starts from committed rows
  and the graph a cold-start Zone's admission reads is rooted at that Zone.
  Nothing already committed is overwritten: an API-created row keeps its
  provenance, a row already retiring keeps that mark, a row that declares an
  owner keeps the manager's own ownership linkage, and every other bundle row
  still arrives through the plane's ingest after the plane opens.