### Added

- Added the pinned Bazel 8.6.0 Linux sandbox with an immutable seccomp policy,
  a static execution supervisor, deterministic product and walker dependency
  hubs, consolidated first-party target inventories, and generated selected
  broker and guest package-policy contexts.
- Added native x86 and arm artifact contracts for the dedicated broker and
  static-PIE guest derivations. The contracts bind exact ELF linkage, measured
  binary size, recursive closure count and digest, and selected policy digest
  without persisting Nix store paths.

### Changed

- Unified product packages under the root resolver-v2 Cargo workspace and
  lock while retaining package-selected broker and guest build, test, policy,
  target-directory, and Nix derivation boundaries. The no-bash walker remains
  an independent tool workspace and lock.
- Updated the release and Layer-1 workflow generators for root-workspace
  package selectors, explicit gate target directories, and an enforcing native
  arm lane that realizes six checks and runs the supply-chain gate on one
  stable commit.

### Security

- Governed Bazel actions use the patched Linux sandbox and load the fixed
  no-network seccomp policy before the action command. Verified executable
  authority remains consumed by the safe Rust boundary and handed to the
  immutable supervisor by open file description, with no Rust unsafe escape.
