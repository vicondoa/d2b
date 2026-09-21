### Fixed

- The provider-crate layout matrix test resolves workspace metadata without
  touching the network or the registry, so a busy machine or an unreachable
  registry can no longer fail it or make it outlast its budget, and a future
  failure names cargo's own error instead of a fixed string.
