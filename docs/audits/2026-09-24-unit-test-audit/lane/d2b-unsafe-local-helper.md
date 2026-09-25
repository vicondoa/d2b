# d2b-unsafe-local-helper - unit-test audit
tests: 29 · src files: 6
(census regex counts 40: 11 extra hits are `#[test]` mentions inside comments - 1343/1359/1389/1431/1667/1699/1791/1807/1840/1877/1907 in runtime.rs)
net: -1 tests, -15 lines

## Findings (biggest net first)
- trivial: `adoption_degrades_identity_ambiguity_without_stopping_scope` (src/runtime.rs:1565) - re-implements snapshot()'s `inspect_scope` match expression verbatim on a hand-built `ScopeInspection` and asserts the copy equals itself; never calls production code, so it cannot fail if the production match changes. Nothing lost.

## Keep
- `manager_environment_is_complete_and_debug_redacted` (src/environment.rs:176) - parse keeps all entries; Debug redacts values; non-graphical child_entries preserves entries.
- `graphical_environment_never_falls_back_to_real_display` (src/environment.rs:199) - graphical child_entries: no proxy → ProxyUnavailable; valid proxy replaces DISPLAY with WAYLAND_DISPLAY; 6 invalid proxy displays rejected.
- `wayland_display_accepts_socket_basename_and_rejects_invalid_values` (src/environment.rs:238) - wayland_display() accepts basename, rejects missing/empty/NUL/`..`.
- `malformed_or_ambiguous_environment_fails_closed` (src/environment.rs:259) - parse rejects no-`=` entry, duplicate key, invalid key start.
- `socket_buffers_meet_frozen_effective_minimum` (src/protocol.rs:459) - configure_socket_buffers on a real socketpair; ok iff effective sizes meet minimum.
- `socket_buffer_minimum_is_exact_and_two_sided` (src/protocol.rs:482) - effective_socket_buffers_sufficient boundary: (min,min) ok, (min-1,min) and (min,min-1) fail.
- `completed_operation_wakes_idle_control_loop` (src/protocol.rs:494) - wake_response_loop write makes wait_for_control_or_response return Response.
- `peer_credential_auth_accepts_only_the_exact_non_root_uid` (src/protocol.rs:516) - peer_uid_is_exact: equal non-root accepted; mismatch or root on either side rejected.
- `concurrent_reservation_allows_only_one_launch_owner` (src/runtime.rs:1430) - 16-thread race on ledger.begin: exactly one Started, rest OperationInProgress, one reservation.
- `reservation_rejects_changed_fingerprint_and_replays_committed_scope` (src/runtime.rs:1471) - begin rejects changed fingerprint and stale identity; committed scope replays AlreadyCommitted.
- `failed_launch_clears_only_its_own_reservation` (src/runtime.rs:1523) - clear with wrong owner is a no-op; correct owner releases the reservation.
- `persisted_scope_debug_hides_scope_identifiers` (src/runtime.rs:1550) - PersistedScope Debug redacts unit_name/invocation_id/control_group.
- `supervisor_spec_debug_redacts_every_sensitive_surface` (src/runtime.rs:1581) - SupervisorSpec and HelperLaunchRequest Debug redact program/args/env/cwd/argv.
- `graphical_spec_paths_and_proxy_arguments_are_strict_and_argv_free` (src/runtime.rs:1595) - proxy_arguments carries all required flags and no app argv; display/private_directory strict; validate rejects 4 invalid displays; timeout ordering.
- `readiness_validation_rejects_order_identity_protocol_and_failure_drift` (src/runtime.rs:1639) - validate_readiness_event rejects wrong stage, target, protocol version, Failed state.
- `readiness_parser_is_bounded_and_rejects_malformed_frames` (src/runtime.rs:1666) - read_event rejects non-JSON and oversized frames.
- `fake_proxy_and_app_complete_typed_readiness_and_cleanup` (src/runtime.rs:1698) - end-to-end readiness handshake: private dir 0o700, CLOEXEC, typed Upstream/Listener/FirstClient events, app connects, teardown removes dirs.
- `plain_supervisor_behavior_is_unchanged` (src/runtime.rs:1774) - run_plain_supervisor spawns the app and acks [1].
- `plain_supervisor_reaps_child_when_started_ack_fails` (src/runtime.rs:1806) - ack write failure → Internal and child reaped (pid gone).
- `first_client_wait_fails_immediately_when_app_exits` (src/runtime.rs:1839) - app exit before FirstClient → FirstClientTimeout in <1s.
- `readiness_wait_uses_an_absolute_deadline` (src/runtime.rs:1876) - 30ms deadline honored; Upstream failure in <250ms.
- `test_child_hold` (src/runtime.rs:1906) - child-process fixture (pid marker / hold) used by the three supervisor tests above; not deletable while they live.
- `immutable_proxy_path_rejects_mutable_and_non_executable_paths` (src/runtime.rs:1919) - validate_immutable_proxy_binary rejects /usr/bin and relative paths.
- `wire_scope_identity_remains_redacted` (src/runtime.rs:1931) - ScopeIdentity Debug redacts invocation_id.
- `cgroup_identity_requires_exact_scope_leaf` (src/systemd.rs:384) - control_group_matches_unit: exact leaf ok; foreign leaf and empty rejected.
- `verified_scope_debug_redacts_all_manager_identity` (src/systemd.rs:398) - VerifiedScope Debug redacts unit_name/invocation_id/control_group.
- `scope_identity_waits_for_transient_unit_properties` (src/systemd.rs:410) - await_scope_identity retries QueryFailed then returns Active scope (attempts == 2).
- `dbus_method_timeouts_remain_typed` (src/systemd.rs:440) - map_user_manager_error maps InputOutput(TimedOut) to Timeout.

## Gap
- resolve_program PATH search and ExecutableUnavailable error path (src/environment.rs:115) - the helper's core launch job; program resolution (absolute path check, PATH walk, dedupe) has no test anywhere in the crate.
- load_ledger/persist_ledger corruption and atomicity paths (src/runtime.rs:1261, 1301) - oversized/duplicate-operation/bad-JSON ledger rejection, atomic rename, 0o600/0o700 modes untested.
- send_frame/receive_frame framing error paths (src/protocol.rs:337, 357) - FrameTooLarge, InvalidFrame (incl. SCM_RIGHTS rejection and declared-length mismatch) untested.

route-out: none.
