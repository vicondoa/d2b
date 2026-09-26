### Fixed

- The `seccomp_profile` module of `d2b-provider-seccomp-profile` no longer
  re-exports its entire surface through the crate root; only the items the
  provider's public API and its consumers use are re-exported, so the
  resource type's spec shapes are reachable at exactly one path.