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
- Fixed `check-async-gate` resolving a relative `<paths>` argument against the
  caller's working directory instead of the repository root, so a subset scan
  could read whichever tree the process happened to be standing in and report
  drift that belonged to a different checkout.
- Fixed the async gate's zero-file-scan check so it builds the empty-scan
  condition on purpose (a workspace whose control plane resolves but whose
  crates hold no Rust) instead of depending on the default scan set coming out
  empty. The gate still fails closed on a scan that matched no `.rs` file.
- Dropped the stale `cloud_hypervisor` row for
  `packages/d2b-broker/src/ops/cgroup.rs` from the shared-family-knowledge
  ratchet. The dead-code scoping pass deleted the module's last carriers of the
  token (`runner_role_matches`, `resolve_kill_path`, `runner_cgroup_shape`), so
  the row's signal is gone and the two-way pin failed both
  `//bazel/checks/policy:provider_crate_layout` and
  `provider_crate_policy::tests::
  the_family_knowledge_ratchet_matches_the_committed_tree`. Re-seeding was not
  an option: the check refuses a row whose signal the tree no longer carries.
- Rewrote the ratchet's `retires_with` values to name the retirement by what it
  does (the family's own census step, its rollout into its provider crate, or a
  permanent carve-out) instead of by plan-unit label, so the diagnostics this
  table emits no longer carry internal plan identifiers.
