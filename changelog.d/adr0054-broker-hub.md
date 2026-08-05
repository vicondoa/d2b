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
  and ignored state outside existing bounded build roots, share one stable
  user-bookkeeping OFD writer lock with the generator and repository cleanup,
  and delegate all direct `--batch` Bazel work to one descendant-reaping
  monitor. Ambient rc discovery is disabled and only the committed `.bazelrc`
  is loaded. Exact Git and semantic postchecks accept only `broker.lock` or a
  no-op; failure may leave that lock dirty and requires one operator-run
  restore command. No detached snapshot, namespace sandbox, candidate
  exchange, receipt, quarantine, or publication transaction is claimed.
- Corrected the authoritative inventory to four hub/workspace Cargo locks,
  using stable tokens `main`, `broker`, `guest`, and `walker`, with
  `packages/Cargo.guest.lock` retained separately and the supply-chain scope
  explicitly limited to three locks. Broker variants remain library-only;
  exact F, B, and M censuses and independent first-party and direct-spoke
  mutations fail closed before the Bazel migration can proceed.
