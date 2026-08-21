### Fixed

- Stabilized Bazel external-seal fixtures by binding nested Cargo checks to
  the Bazel Rust toolchain, including its standard library and unique scratch
  ownership, instead of ambient compiler state.
- Materialized large Bazel fixture source inventories through a manifest
  instead of action arguments, avoiding local `ARG_MAX` failures.
