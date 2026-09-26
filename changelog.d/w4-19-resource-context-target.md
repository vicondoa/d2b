### Fixed

- `ResourceContext::new` no longer takes a `TargetHandle` argument that was
  silently discarded; callers no longer pass a value that has no effect.