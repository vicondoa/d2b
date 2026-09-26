### Fixed

- d2b-session no longer depends on the unused d2b-audit and d2b-telemetry crates; its
  bazel targets drop the matching explicit deps.
- The unused serde_json dev-dependency is removed from the d2b-session manifest.