### Changed

- Reduce Nix-unit evaluation memory by sharing focused realm, workload,
  resource, gateway, niri, and observability fixtures while retaining full
  integration coverage.
- Share the configured local-VM workload predicate between limit validation
  and private workload emission.
