### Changed

- Recorded the decision to make Bazel the build and test scheduler for the
  Rust gate. The current `make test-rust` path and the existing Rust
  continuous-integration jobs stay authoritative; a new `make test-bazel-rust`
  target and a separate, non-required workflow run the Bazel path beside them
  so the two can be compared. Switching over requires a complete
  surface-by-surface coverage map, evidence that each check still fails when it
  should, an unchanged pinned test inventory, and measured wall-clock ceilings
  of ten minutes for a warm local run and fifteen minutes for a cold local run
  and a cold continuous-integration run. Cargo manifests, lock files, and the
  pinned Rust toolchains remain the authoritative dependency and toolchain
  inputs, and the decision covers Rust only.
