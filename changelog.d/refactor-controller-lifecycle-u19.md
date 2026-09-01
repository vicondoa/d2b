### Changed

- Converged the shared Cargo, copied Guest, Bazel, and policy graphs with the
  current Zone and controller-owned Guest lifecycle.
- Regenerated locked dependency projections, policy closures, schemas, CLI
  artifacts, and current consumer examples from the active sources.

### Removed

- Removed retired gateway and realm-core edges from the shared workspace,
  fixture, aggregate Bazel, and policy surfaces.
- Removed stale Realm and VM-first CLI schema artifacts from the current
  generated contract set.
