# d2b-provider-seccomp-profile - unit-test audit
tests: 3 · src files: 3
net: -0 tests, -0 lines

## Findings
- cross-check: `the_wire_shape_round_trips_and_is_closed` (src/seccomp_profile.rs:304) - canonical-json roundtrip of `SeccompProfileSpec` may be pinned by d2b-contracts-resource schema/contract tests (`canonical_json_bytes` family); resolve in C2.
- gap: `TooManyDeviceBinds` error path (src/seccomp_profile.rs:186-188, `SeccompProfileSpec::new` rejects `devices.len() > MAX_SECCOMP_DEVICE_BINDS`) - declared error variant with no test anywhere in the crate; sibling bound (`TooManySyscalls`) is tested.
- gap: `DeviceNodePath` length bound (src/seccomp_profile.rs:33, `parse` rejects paths over `MAX_DEVICE_NODE_PATH_BYTES` = 255) - the existing path test covers prefix and control characters but not the length boundary.
- gap: wire-level invalid device path (src/seccomp_profile.rs:54-59, `DeviceNodePath::deserialize` maps parse errors to serde errors) - no test proves a JSON profile with a bad `/dev` path is rejected at deserialization.

## Keep
- `device_paths_outside_dev_and_control_characters_are_refused` - pins `DeviceNodePath::parse` rejecting non-`/dev/` paths and control characters, accepting `/dev/null`.
- `list_bounds_fail_closed` - pins `SeccompProfileSpec::new` failing closed with `TooManySyscalls` at `MAX_SECCOMP_SYSCALLS + 1`.
- `the_wire_shape_round_trips_and_is_closed` - pins canonical-json roundtrip of a full profile and closed-schema rejection of unknown wire fields (`profilePath`).
