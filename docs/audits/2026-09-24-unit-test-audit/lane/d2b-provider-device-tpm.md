# d2b-provider-device-tpm - unit-test audit
tests: 25 · src files: 12
net: -1 tests, -23 lines

## Findings (biggest net first)
- duplicate: `long_lived_argv_has_expected_shape` (src/swtpm_argv.rs:294) - covered by `audit_swtpm_input_parity_golden` (src/swtpm_argv.rs:275). Both pin the full long-lived swtpm argv rendered from the same `audit_swtpm_input()`; the golden test asserts byte-parity with `tests/golden/runner-shape/swtpm-argv-minimal.txt`, which pins strictly more - exact order and formatting of every argument, including the absence of `--daemon` - so the contains-based shape assertions (binary, `socket`, `--tpm2`, tpmstate/ctrl/server/flags/log/pid entries) add nothing the golden does not already pin.
- gap: `LiveTpmResourceEffectPort` error surface (src/effects_service.rs:341-487) - `identity_matches`, the migration-decision gate, `prepare_state_dir` (incl. the `zone_native_swtpm_state_row`/`row_posture` zone-native fallback at :490), and the consume-lifecycle-lease-once gate are the crate's production port, but no test anywhere in the crate drives them; the integration tests (`tests/resource_controller.rs`) run the controller against fake `ScriptedEffects`/`NoopEffects` ports, so every `StateIntegrity`/`Transient` branch of the real port is unpinned.
- gap: `declared_child` error paths (src/effects_service.rs:168) - missing `type`/`metadata.name`/`spec` → `EffectRejected` and unparseable ref → `InvalidDevice` are only ever exercised on the happy path (via `only_the_state_volume_is_ensured`); the malformed-document branches have no test.

## Keep
- `audit_swtpm_input_parity_golden` - pins byte-parity of the long-lived swtpm argv against the committed golden fixture.
- `flush_argv_matches_w3_invariant` - pins exact `swtpm_ioctl -i --unix <ctrl>` flush argv.
- `omits_startup_clear_when_disabled` - pins `startup_clear=false` renders no `--flags startup-clear`.
- `extra_args_appended_at_end` - pins extra args appended in order at argv end.
- `rejects_invalid_binary_path` - pins `InvalidBinaryPath` on non-absolute swtpm binary path.
- `rejects_empty_vm_name` - pins `EmptyVmName`.
- `rejects_non_absolute_state_dir` - pins `InvalidStateDir` on non-absolute state dir.
- `rejects_empty_ctrl_socket` / `rejects_empty_server_socket` - pin `EmptySocketPath` per socket field.
- `rejects_empty_log_path` / `rejects_empty_pid_path` - pin `EmptyFilePath` per file field.
- `rejects_log_level_out_of_range` - pins `LogLevelOutOfRange` at both bounds (0 and 21) of the generator.
- `flush_rejects_invalid_inputs` - pins all three flush-generator error paths (binary, vm name, ctrl socket).
- `generated_process_specs_round_trip_through_v3_contracts` - pins swtpm Process + flush EphemeralProcess specs decode through the closed v3 contracts (class, mounts, principal, namespaces, umask, restart/health, deadlines).
- `state_volume_owner_is_the_authenticated_device_reference` - pins Volume `ownerRef` is the authenticated `Device/vm-tpm` ref, not the uid-derived name.
- `state_child_names_preserve_the_full_device_incarnation` - pins distinct device uids yield distinct Volume names.
- `state_volume_grants_the_flush_principal_access_to_the_shared_state_dir` - pins daemon ownership, both ACLs, mode 0770, and decode through the closed `VolumeSpec`/`AclGrant` contracts.
- `flush_builder_rejects_non_host_execution_refs` - pins `InvalidExecutionRef` from the flush builder on a non-Host execution ref.
- `declared_row_names_follow_the_provider_projection` - pins the declared row names (`Process/…`, `EphemeralProcess/…`, `Endpoint/…`, state Volume) exactly as the Nix projection authors them.
- `only_the_state_volume_is_ensured` - pins only the controller-owned Volume is ensured and absent rows read as `Transient`.
- `declared_row_phases_gate_the_effects` - pins wait_ready transitions: absent/Pending → `Transient`, Ready → `Ok`, Failed → `EffectRejected`.
- `a_foreign_owned_row_fails_closed` - pins a row owned by another Device reads as `StateIntegrity`.
- `the_flush_gate_reads_the_one_shot_outcome_not_only_the_phase` - pins flush gating on the one-shot outcome: in-flight → `Transient`, failed → `EffectRejected`, succeeded → `Ok`, unclassified → `StateIntegrity`.
- `deletion_targets_the_declared_rows` - pins delete retires both declared worker rows by reference.
