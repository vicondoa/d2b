### Changed

- One producer now renders a resource row for the wire on the API side, and
  one produces the live status projection in the resource runtime. Readers
  of a converted type receive the canonical envelope, metadata, spec and
  observed status, so the shape can no longer differ between a `get`, a
  `list`, a mutation confirmation and a status read.
- The daemon's Core, guest, interaction and shared-provider views take their
  phase string from the runtime's wire-status projection instead of each
  mapping the status vocabulary locally.

### Fixed

- A delete confirmation and a no-op spec update no longer serve a stale or
  placeholder status while `get` serves the live one; both now report the
  actual observed state, and a row with no current observation reports the
  pending projection rather than status bytes that a previous actor had
  persisted.
