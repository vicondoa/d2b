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
  wrong-target, and wrong-edge-kind negatives. Contributor-only policy and
  repin mutations use the existing two-step workflow: enter `nix develop` at
  the repository root, then run the named `cargo xtask` command from
  `packages/`. They remain unreachable from workflows and Make targets; gates
  use approved Make targets and hermetic vendored policy inputs. The `main`,
  `broker`, and `guest` hub identifiers are retired with a fixed `product`
  command that runs from `packages/` and never repeats that path; `walker`
  remains. Existing Layer-1 supply-chain, drift, and flake targets recurrently
  run policy and wiring checks. Separate native x86_64 and aarch64 runners each
  realize their four wrappers and static guest ELF check, with pinned
  inventories and independent per-architecture foreign-system and
  remote-builder negatives. Existing contract-crate coverage remains
  enforcing, and the six guest license findings require a narrow update.
