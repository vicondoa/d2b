### Changed

- `make test-fixture-contracts` no longer reruns fixture-independent policy
  binaries already enforced by `make test-policy`, avoiding duplicated
  repository-wide source and documentation scans.
