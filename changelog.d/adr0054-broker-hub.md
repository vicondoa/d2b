### Added

- Recorded that the privileged broker remains a standalone Cargo workspace
  with its own authoritative lock while its Bazel dependency hub reads a
  committed generated witness of the realized path-dependency closure. Exact
  authoritative and witness projections compare package, source, checksum,
  feature, target, and dependency-edge identity, while separate checks cover
  intentionally omitted declarations and inert generated targets.
- Assigned generated Bazel inputs solely to `cargo xtask gen-bazel`, whose
  `--check` form is strictly read-only and creates no state. The dependency
  workflow commits all changed authoritative Cargo inputs and generated
  outputs together before broker repin runs Bazel in a bounded isolated
  detached worktree at exact `HEAD`.
- Required broker repin to validate actual Bazel lock and repository source
  identity in the snapshot, contain batch children with process and namespace
  controls, and publish only `bazel/cargo/broker.lock` through anchored
  no-symlink exchange with preallocation, crash recovery, and shared
  generator/repin exclusion.
- Corrected the authoritative inventory to four hub/workspace Cargo locks,
  with `packages/Cargo.guest.lock` retained separately as a generated and
  cache-key input. Broker variants remain library-only, exact B and M
  censuses precede edge checks, and independent first-party and direct-spoke
  mutations fail closed before the Bazel migration can proceed.
