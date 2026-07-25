### Changed

- The delivery workflow success JSON is now a pinned, version-coupled contract.
  Its `operation` and `status` values are typed closed domains rather than free
  strings, and a golden contract test fixes the complete wire shape (field
  names, omitted-when-empty optional fields, and both value domains). An
  incompatible change to the shape or either domain now fails the build unless
  it travels with a `schema_version` bump, so a consumer that reads this JSON
  can no longer break silently against a drifted producer.

### Security

- The heavy-gate semaphore now uses a single canonical per-uid namespace
  (`/run/user/<uid>`, falling back to `/tmp` only when the runtime directory is
  unavailable). It no longer honours `XDG_RUNTIME_DIR` or `TMPDIR`, so a caller
  can no longer point a lane at a private directory to obtain an independent
  slot pool and defeat the global two-slot concurrency limit while still passing
  slot verification honestly.
- The container integration lane now holds a heavy-gate slot. The aggregating
  `tests/test-integration.sh` runner and the standalone
  `tests/integration/containers/ubuntu-host-check.sh` entrypoint both prove a
  genuinely held slot before doing any podman or Nix work, closing a lane that
  previously ran entirely unsynchronised against the shared Nix store and cargo
  target.
- The heavy-entrypoint inventory guard is now closed-world across every heavy
  lane. It classifies each lane directory as gated or explicitly out of scope
  and fails when a lane is neither, so a new heavy lane cannot appear
  unsynchronised without being classified. Sourced support libraries (for
  example `tests/integration/containers/lib.sh`) are distinguished from runnable
  entrypoints and are not required to hold a slot.
- Delivery-state evidence reads, directory listings, and writes now all resolve
  fd-relative from the verified root on the same inode chain, matching the
  hardened write path. Reads and listings are no longer path-based
  check-then-open, so an attacker who controls a writable ancestor can no longer
  swap trees during the read phase and seal forged evidence into legitimate
  state.
- Diagnostics from the delivery workflow, heavy-gate slot verification, and the
  wave-snapshot Git path now name components by semantic role and
  repository-relative key only. They no longer interpolate absolute host paths
  (including `HOME` and the per-user runtime directory), the caller's numeric
  uid, or raw Git subprocess stderr, so an error surfaced to operator stderr or
  a CI log no longer discloses host filesystem layout or user identity.
  Negative-output tests now assert that a forced failure in each of these
  surfaces emits no absolute path and no uid.

### Fixed

- The shared heavy-gate self-guard helper is now bounded and derives everything
  from its own on-disk location. It always rebuilds `xtask` from the canonical
  checkout (so a stale binary without the slot-verification subcommand can never
  be used as-is), normalises a relative target directory against that checkout,
  ignores the caller-supplied root and target-directory variables when locating
  the binary, and enforces a fail-closed re-exec depth limit so a binary that
  keeps failing verification can no longer loop forever.
- Heavy-gate slot verification now distinguishes a genuine "no slot held"
  verdict from a verifier malfunction. Environment, permission, and unsupported
  errors during the ownership proof are returned as typed errors with distinct
  non-zero exit codes instead of collapsing into the "unheld" verdict, and the
  shell guard branches explicitly: proceed when held, re-acquire when unheld,
  and propagate anything else unchanged so a broken verifier fails closed rather
  than silently re-acquiring.
- The concurrent-candidate-creation regression test now forces the `mkdirat`
  `EEXIST` race it is meant to cover. A test-only synchronization point releases
  both racing writers only after both have observed the directory absent, so the
  test provably exercises the concurrent-creation branch and asserts both
  writers still succeed.
