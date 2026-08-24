### Fixed

- Keep trusted CI compatible with the root Rust toolchain authority while the
  protected workflow still reads the former package-local path.
- Remove the retired `rust-local` CI shard, make broker profile tests
  runner-UID independent, and use native Bash for Bazel actions on FHS hosts.
