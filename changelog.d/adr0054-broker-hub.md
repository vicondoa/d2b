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
  migrate to compiled `./d2b-nix`, which accepts canonical Nix store
  executables through system, root, and per-user profiles while isolating Nix
  from caller home, config, plugins, startup files, functions, and path.
  Install or fix Nix at one of those documented profiles if the launcher
  refuses all candidates. The `main`, `broker`, and `guest` hub identifiers are
  retired with copy-verbatim `product` replacements through that launcher;
  `walker` remains. Existing Layer-1 supply-chain, drift, and flake targets
  recurrently run policy and wiring checks. Separate native x86_64 and aarch64
  runners each realize their four wrappers and static guest ELF check without a
  foreign system or remote builder. Existing contract-crate coverage remains
  enforcing, and the six guest license findings require a narrow update.
