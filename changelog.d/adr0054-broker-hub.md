### Added

- Recorded that the privileged broker remains a standalone Cargo workspace
  with its own authoritative lock while its Bazel dependency hub reads a
  committed generated witness of the realized path-dependency closure.
  Authority, witness, actual Bazel-lock, and actual repository projections
  compare every representable source, checksum, revision, feature, target,
  alias, and per-edge field; omitted declarations use a separate exact ledger.
- Assigned generated Bazel inputs solely to `cargo xtask gen-bazel`, whose
  `--check` form is read-only and creates no state. Contributors commit the
  complete authoritative Cargo-input and generated-output change together
  before broker repin; repin never generates in place.
- Required broker repin to start from clean `HEAD`, index, tracked, untracked,
  and ignored state outside existing bounded Bazel roots, share one
  worktree-local OFD writer lock with the generator, run Bazel directly with
  `--batch`, and accept only `broker.lock` or a no-op after exact Git and
  semantic postchecks. Failure may leave that lock dirty and has one exact
  restore command; no detached snapshot, namespace sandbox, candidate
  exchange, receipt, quarantine, or publication transaction is claimed.
- Corrected the authoritative inventory to four hub/workspace Cargo locks,
  using stable tokens `main`, `broker`, `guest`, and `walker`, with
  `packages/Cargo.guest.lock` retained separately and the supply-chain scope
  explicitly limited to three locks. Broker variants remain library-only;
  exact F, B, and M censuses and independent first-party and direct-spoke
  mutations fail closed before the Bazel migration can proceed.
