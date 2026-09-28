### Changed

- The swtpm state-directory identity is derived once, in the broker, from the
  verified bundle. `resource_backed_swtpm_identity` in
  `packages/d2b-provider-process/src/operations.rs` re-implemented the same
  trusted-path derivation the broker already owned in
  `packages/d2b-broker/src/ops/swtpm_identity.rs`, and the two could drift:
  a change landing in one would leave the daemon telling the broker a
  different path than the broker computes for itself. The `spawn-process`
  kernel now resolves the identity from the captured bundle path - the same
  per-request reload authority its USBIP and Device-worker branches already
  use - reusing one resolver load for the state identity and the per-Guest
  runtime posture.

### Removed

- The `swtpmIdentity` launch-plan field, its daemon-side serialization, and
  the broker's `optional_parse_swtpm_identity` wire parser. The trusted state
  path no longer rides the wire at all, so a launch plan can no longer assert
  one. The committed `spawn-process` payload schema and every generated view
  of it (`gen-broker-operations`) drop the field.
- `resource_backed_swtpm_identity` in `packages/d2b-broker/src/runtime.rs`,
  plus `DispatchBackend::spawn_runner` and both its implementations, the last
  callers of it. That trait method was already unreachable: U10 retired the
  typed `SpawnRunner` wire arm, so `BrokerRequest::SpawnRunner` falls to the
  dispatch match's catch-all refusal and never reaches a backend. Removing it
  leaves no path that could resolve an identity for a launch the broker does
  not perform.
- The swtpm literals this left in `packages/d2b-provider-process`, including
  its own `ResourceBackedSwtpm` struct, `state_volume_name`, `storage_root`,
  and `storage_path` helpers.
- Four `ProviderFamilyKnowledgeExemption` rows for
  `packages/d2b-provider-process/src/operations.rs` (`device_gpu`,
  `device_security_key`, `device_tpm`, `device_usbip`). They were already
  stale before this change - no such token appears in that module - and
  `//bazel/checks/policy:provider_crate_layout` refuses a stale exemption, so
  `make test-policy` could not reach 26/26 until they were dropped.

The sandbox, the seccomp policy, the `writable_paths` boundary, and the
ownership check are untouched. The state-directory access grants are
unchanged: `grant_swtpm_state_dir_traversal` still opens the same trusted
state directory to the launched principal, now from a broker-resolved
identity rather than a daemon-sent one, and its regression tests
(`swtpm_state_dir_traversal_*`) and the `DeviceWorkerSocketGrant` per-Guest
runtime-directory tests still hold.
