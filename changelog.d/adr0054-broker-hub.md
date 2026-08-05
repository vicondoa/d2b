### Added

- Proposed ADR 0054, selecting one Cargo workspace and dependency hub for d2b
  product packages while retaining the no-bash walker as a separate tooling
  workspace. The decision accepts the shared external package and feature
  superset while keeping selected Cargo closure policy authoritative for
  security and exact native Bazel context censuses authoritative for
  first-party edges. Broker and real-libshpool guest production and
  root-dev-inclusive policy inputs are generated separately for x86_64-linux
  and aarch64-linux: broker artifacts use matching GNU targets and static guest
  artifacts use matching musl targets. All eight dependency/package checks
  bind exact system-and-target inputs and carry early wrong-system,
  wrong-target, and wrong-edge-kind negatives. Root policy and repin commands
  use the repository environment scrubber, with hostile `BASH_ENV` and
  Cargo-function probes. The `main`, `broker`, and `guest` hub identifiers are
  retired with exact `product` replacements; `walker` remains. Implementation
  must refresh and test the flake matrix before the focused checks and
  aggregate deny/audit rechecks. Existing contract-crate coverage remains
  enforcing, and the six guest license findings require a narrow update.
