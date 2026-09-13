### Fixed

- The serving worker no longer runs with the daemon's uid/gid. A binding-owned
  serving launch runs as the trusted intent's principal with per-runner ACLs on
  its socket directory and view root (`grant_serving_worker_launch_acls`), so a
  compromised worker can no longer borrow the daemon's identity for the broker's
  peer-identity check; a ticket that names no broker socket, or one outside the
  broker runtime root, is refused instead of granted.
- A credential Provider session is admitted against the policy it was built
  with: the session's transport evidence is derived from that policy's channel
  binding digest instead of a hardcoded constant, which had made the credential
  lane unadmittable while its check passed vacuously. Evidence compiled from
  another policy is refused as a channel-binding mismatch.
- Manager-plane LIST honours its filters and its cursor: owner filters
  (`owner.resourceUid`, `owner.resourceRef`) match the row's real ownership, the
  page is deterministic with an opaque keyset cursor and an honest `truncated`,
  and a malformed or selector-mismatched cursor is refused with a typed reason
  instead of being ignored.
- Manager-plane WATCH refuses with `UnsupportedCapability` (`watch-not-wired`)
  rather than returning a receipt for a stream nothing fills; the consumer-less
  producer machinery is deleted. LIST is the enumeration surface until a pump is
  wired.
- An ensure whose spec is unchanged but whose metadata or owner differs now
  writes those columns and notifies the actor instead of reporting `Unchanged`;
  a trigger that coalesced during a terminally failing pass or effect is
  reconciled instead of being lost with the failure; the SQLite side files are
  tightened under the names SQLite actually creates (`-wal`/`-shm` appended to
  the database file name).
