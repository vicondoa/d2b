### Added

- Added declared Bazel carriers and explicit local integration successors for
  the remaining Layer-1 Nix, policy, fixture, metadata, and workflow checks.
- Added eligibility and local-only reason parity checks, including the
  hermetic pure-evaluation proof and non-baseline Nix realization worker-image
  experiment.

### Changed

- Centralized the guest static ELF smoke expression and preserved the existing
  static-binary assertions in the flake checks.
- Run standalone proof crates from writable per-test workspaces so Bazel
  execution cannot inherit the root Cargo workspace.
- Fail closed on ambiguous remote fallback failures, preserve dispatch hints
  through evidence redaction, and require a BEP `testResult` event on success.
