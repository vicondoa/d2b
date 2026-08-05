### Added

- Proposed ADR 0054, selecting a committed generated splice workspace for the
  privileged broker's independent Bazel dependency hub. The decision preserves
  Cargo authority and independent locks, splits complete production and test
  contexts where their configured graphs differ, gives the witness one
  generator and a read-only drift check, and requires exact Cargo-derived B,
  M, spoke, and actual `@broker` fidelity. Broker repin remains a separate
  undecided lifecycle, so Spec 003 stays blocked pending its own accepted ADR
  and later spec amendment.
