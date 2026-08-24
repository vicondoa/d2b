### Fixed

- Keep trusted CI compatible with the root Rust toolchain authority while the
  protected workflow still reads the former package-local path.
- Remove the retired `rust-local` CI shard, make broker profile tests
  runner-UID independent, and use native Bash for Bazel actions on FHS hosts.
- Give cold realized Nix and advisory performance jobs sufficient hosted-runner
  time, and keep the async blocking-adapter heartbeat below its backend delay
  without assuming sub-5-ms scheduler jitter.
