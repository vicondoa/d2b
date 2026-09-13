### Added

- Added the `d2b-resource-runtime` crate as the scaffold for the v3 resource
  runtime rewrite: a module skeleton (manager, resource, driver, context,
  provider, target, guest_target, watch, spec_store, identity, error,
  revision) plus a compile-and-run smoke test, registered in the Bazel graph
  (`all-tests` suite and `rust-main-packages` entry).
- Added the workspace dependencies `ractor` 0.16 (default features, no
  cluster) and `rusqlite` 0.40 with `bundled` SQLite, with cargo-deny
  admission updated in the same commit that first consumes them.
