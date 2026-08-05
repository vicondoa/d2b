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
  start from an absolute clean environment before any shell; hostile startup
  files and exported command functions prove pinned tools execute. The
  `main`, `broker`, and `guest` hub identifiers are retired with
  punctuation-free, copy-verbatim `product` replacements; `walker` remains.
  Existing Layer-1 supply-chain, drift, and flake targets recurrently run the
  policy logic, planted wiring checks, and all eight wrappers on native x86_64
  and aarch64 runners. Both architectures also realize the static guest ELF
  check, with realized-class and dual-system matrix pins regenerated. Existing
  contract-crate coverage remains enforcing, and the six guest license
  findings require a narrow update.
