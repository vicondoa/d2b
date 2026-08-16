### Added

- Added `cargo xtask heavy-gate`, the sole two-slot per-UID semaphore for every
  long-running validation lane. It uses open file description locks, retries
  acquisition every 250 ms up to a 30-minute ceiling, fails closed when the
  platform cannot provide those locks instead of degrading to unsynchronized
  execution, hands the locked descriptor to the child so the slot is held for
  the child's whole life, and owns the child's process group so an interrupt or
  timeout cannot orphan a running lane.
- Added the `heavy-check`, `heavy-test-integration`,
  `heavy-test-host-integration`, `heavy-test-hardware`, `heavy-cargo-test`, and
  `heavy-flake-check` Makefile targets, which route the container,
  host-integration, hardware, Rust, and building `nix flake check` lanes through
  that one semaphore so concurrent validation cannot oversubscribe the shared
  Nix store, cargo target directory, or KVM device.
