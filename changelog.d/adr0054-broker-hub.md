### Added

- Recorded that the privileged broker remains a standalone Cargo workspace
  with its own authoritative lock while its Bazel dependency hub reads a
  committed generated workspace that exactly represents the realized
  path-dependency closure. The generated fidelity check covers source
  identity, checksums, features, target kinds, and dependency edges, retains a
  byte-identical broker lock mirror, and requires locked offline Cargo
  metadata to succeed.
- Assigned generated Bazel inputs solely to `cargo xtask gen-bazel`, whose
  `--check` form is a strictly read-only exact byte-and-census gate. Broker
  repin first refuses stale generated inputs with the generate, review,
  commit, then repin remedy and then writes only
  `bazel/cargo/broker.lock`. The explicit two-command workflow keeps
  single-writer ownership and the repin output set lock-only.
- Corrected the authoritative inventory to four hub/workspace Cargo locks,
  with `packages/Cargo.guest.lock` retained separately as a generated and
  cache-key input. Broker path-dependency variants remain library-only, their
  tests remain main-owned, and exact target censuses and independent
  first-party and third-party-spoke isolation checks fail closed before the
  Bazel migration can proceed.
