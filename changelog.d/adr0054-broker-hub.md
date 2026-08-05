### Added

- Proposed ADR 0054, selecting one Cargo workspace and dependency hub for d2b
  product packages while retaining the no-bash walker as a separate tooling
  workspace. The decision accepts the shared external package and feature
  superset while keeping selected Cargo closure policy authoritative for
  security and exact native Bazel context censuses authoritative for
  first-party edges. Broker and real-libshpool guest production and
  root-dev-inclusive policy inputs are generated separately for x86_64-linux
  and aarch64-linux with target/build edges, exact pinned offline sources, and
  system-bound Nix checks. Root commands preserve the packages toolchain and
  Cargo configuration. Distinct seeded failures cover every completeness,
  containment, target, source-integrity, policy, and generated-drift refusal.
  Existing contract-crate coverage remains enforcing. The guest policy's six
  existing license findings remain blocked on a narrow reviewed update.
