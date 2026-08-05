### Added

- Proposed ADR 0054, selecting one Cargo workspace and dependency hub for d2b
  product packages while retaining the no-bash walker as a separate tooling
  workspace. The decision accepts the shared lock's dependency union and
  enforces privileged and static minimality through package-selected Cargo and
  Nix builds, native Bazel target edges, production closure inventories, and
  reproducible dev-inclusive package deny and audit inputs. It preserves
  contract-crate clippy and fixture coverage and records the guest
  real-libshpool policy's six existing license findings as an implementation
  blocker requiring a narrow reviewed policy update.
