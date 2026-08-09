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
- Made Bazel and package-policy generator recovery explicit: preview output is
  complete and replaceable, while `--install` atomically promotes the exact
  owned tracked paths and removes stale sidecars. Schema generation now writes
  and reproducibly checks the committed `docs/reference/schemas/v2/` root.
- Updated the release and Layer-1 workflow generators for root-workspace
  package selectors, explicit gate target directories, and an enforcing native
  arm lane that realizes six checks and runs the supply-chain gate on one
  stable commit.
- Added one committed native policy/check manifest shared by the Rust, Nix,
  shell, CI, and Bazel inventories, and made policy mutation coverage call the
  production artifact validator.
- Generated Bazel inventories now enumerate Cargo libraries, binaries, tests,
  benches, examples, required features, harness shape, target conditions, and
  effective dependency closures. Removed the unused runner, locator, and
  per-package BUILD renderer scaffolding.
- Preserved the existing one-line xtask generator stdout contract, while
  making command failures bounded and redacted. Schema and generated-output
  writers now support safe external output directories and reject symlinked
  parents and tracked targets.

### Security

- Governed Bazel actions use the patched Linux sandbox and load the fixed
  no-network seccomp policy before the action command. Verified executable
  authority remains consumed by the safe Rust boundary and handed to the
  immutable supervisor by open file description, with no Rust unsafe escape.
