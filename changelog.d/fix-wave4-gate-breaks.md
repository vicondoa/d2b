### Fixed

- Fixed the daemon's test-only owner-connection hook to intercept only the
  connections the installing test marks as its own. While a hook was
  installed, any concurrent test's Process or typed-shell owner connection was
  dropped without a reply, which failed that test's reply read.
- Stopped the `d2b-core` `OperationAuthz` conversion from cloning `Copy` enum
  fields (`SecretAccess`, `BrokerRequirement`, `AuditMode`), which the clippy
  check on the core test-support target rejects.
- Made `daemon_state_persistence` wait for the state-restore report file
  instead of the public socket before killing the restore daemon: the report is
  written during startup after the socket appears, so the kill could race it.
- Regenerated `docs/reference/daemon-api.md` after the daemon API contract
  changes, so the generated-artifact drift check agrees with the generator.
- Added `d2b-resource-client` to the guest workspace mirror (`flake.nix`,
  `tests/fixtures/guest-rust-workspace/Cargo.toml`, and
  `packages/Cargo.guest.lock`) after
  `d2b-provider-guest-cloud-hypervisor` gained a dependency on it, which the
  realized supply-chain lane requires.
- Refreshed the async-gate inventory so its recorded marker-honored sites match
  the current `d2bd` composition sources.
