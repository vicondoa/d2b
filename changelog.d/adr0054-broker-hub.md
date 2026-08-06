### Added

- Accepted ADR 0054, selecting one Cargo workspace and dependency hub for d2b
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
- Required governed Rust actions to use the repository's Nix-pinned,
  Linux-sandbox-patched Bazel 8.6.0. The sandbox child loads the fixed seccomp
  policy before the complete action command, covering compiler commands, test
  setup, tests, and descendants without relying on an action wrapper. Exact
  source, patch, policy, output, executable, and capability identities,
  sandboxed-only strategies, inherited-capability checks, and pre-action
  network plants fail closed.
- Preserved the workspace-wide unsafe-code prohibition by placing verified
  executable ownership and its sole consuming API in one dependency-leaf
  crate. The consumer passes the verified descriptor through reviewed safe
  command-fd mapping to an identity-bound immutable Nix helper that sets
  close-on-exec and performs pathless `execveat`; direct invocation outside
  that API is an enforcing policy failure.
- Strengthened planning and disclosure checks with a complete Markdown task
  census, an independent exact task-ID census, byte-exact fixed diagnostics,
  and isolated hybrid-disclosure mismatch fixtures. Alias removal now owns an
  atomic diagnostic transition from existing shadow targets to enduring
  promoted aggregate and slice targets, so recovery text never names a
  nonexistent command.
