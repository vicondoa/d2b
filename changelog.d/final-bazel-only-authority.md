### Changed

- Make, contributor, test, and pull-request authority now use the fixed
  Bazel Layer-1 aggregate and owner-local Bazel labels; Cargo manifests and
  lockfiles remain rules_rs metadata inputs rather than gate entrypoints.

### Removed

- Remove the retired Layer-1 workflow generator and coverage shim, direct
  Cargo compatibility instructions, and stale package test commands.
