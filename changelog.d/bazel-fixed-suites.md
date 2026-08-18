### Changed

- Define the fixed Bazel job sets with recursive portable Rust target patterns,
  focused broker and guest suites, existing policy/tooling suites, fixed Nix
  evaluation/realized/aarch64 suites, and a small fixtures/proofs suite.
- Extend the existing realized Nix target with the `rust-deny`,
  `guest-rust-deny`, and `rust-audit` flake checks for supply-chain coverage.
