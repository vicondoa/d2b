### Fixed

- Host-prepare SHA-256 digests (nftables drift hash and hardlink-farm
  generation ids) are now hex-encoded into a single preallocated buffer
  instead of allocating one string per byte.
- The `d2b-activation-helper` digest test now runs its fixtures in a
  temporary directory instead of the CWD-relative `target/` build
  directory, so it no longer pollutes build artifacts and works when the
  build directory is read-only.