### Added

- Proposed ADR 0054, selecting one Cargo workspace and dependency hub for d2b
  product packages while retaining the no-bash walker as a separate tooling
  workspace. The decision accepts the shared external package and feature
  superset while keeping selected Cargo closure policy authoritative for
  security and exact native Bazel context censuses authoritative for
  first-party edges. Generated production inventories and pruned locks enforce
  binary and static minimality; dev-inclusive metadata and locks preserve deny
  and audit policy through exact pinned offline source checks. Existing
  contract-crate clippy, policy, test, and fixture compilation coverage remains
  enforcing. The guest real-libshpool policy's six existing license findings
  remain an implementation blocker requiring a narrow reviewed update.
