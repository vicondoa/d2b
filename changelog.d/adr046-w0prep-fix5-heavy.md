### Changed

- The delivery workflow success JSON is now a pinned, version-coupled contract.
  Its `operation` and `status` values are typed closed domains rather than free
  strings, and a golden contract test fixes the complete wire shape (field
  names, omitted-when-empty optional fields, and both value domains). An
  incompatible change to the shape or either domain now fails the build unless
  it travels with a `schema_version` bump, so a consumer that reads this JSON
  can no longer break silently against a drifted producer.

### Security

- Delivery-state evidence reads, directory listings, and writes now all resolve
  fd-relative from the verified root on the same inode chain, matching the
  hardened write path. Reads and listings are no longer path-based
  check-then-open, so an attacker who controls a writable ancestor can no longer
  swap trees during the read phase and seal forged evidence into legitimate
  state.
- Diagnostics from the delivery workflow and the wave-snapshot Git path now
  name components by semantic role and
  repository-relative key only. They no longer interpolate absolute host paths
  (including `HOME` and the per-user runtime directory), the caller's numeric
  uid, or raw Git subprocess stderr, so an error surfaced to operator stderr or
  a CI log no longer discloses host filesystem layout or user identity.
  Negative-output tests now assert that a forced failure in each of these
  surfaces emits no absolute path and no uid.

### Fixed

- The concurrent-candidate-creation regression test now forces the `mkdirat`
  `EEXIST` race it is meant to cover. A test-only synchronization point releases
  both racing writers only after both have observed the directory absent, so the
  test provably exercises the concurrent-creation branch and asserts both
  writers still succeed.
