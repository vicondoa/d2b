# d2b-provider-device-security-key - unit-test audit
tests: 39 · src files: 12
net: -10 tests, -120 lines

## Findings (biggest net first)
- duplicate: `parse_init_packet_identifies_cmd_and_cid` (src/relay_service.rs:668) - covered by `parses_initialization_and_continuation_reports` (src/relay.rs:353). Both pin init-report parse yielding cid/cmd/bcnt; the relay.rs test pins init+continuation, strictly more.
- duplicate: `cid_translator_allocs_fresh_host_cid` (src/relay_service.rs:721) - covered by `cid_translation_isolated_and_released` (src/relay.rs:370). Both pin fresh host CID (≠ broadcast) with bidirectional mapping; the relay.rs test also pins release.
- duplicate: `cid_translator_release_removes_mapping` (src/relay_service.rs:768) - covered by `cid_translation_isolated_and_released` (src/relay.rs:370). Both pin `release_guest_cid` dropping both mapping directions.
- duplicate: `lease_acquire_succeeds_when_available` (src/relay_service.rs:780) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). Both pin acquire on an available state succeeds and the state becomes Leased.
- duplicate: `lease_acquire_fails_when_held_by_other_vm` (src/relay_service.rs:788) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). Both pin a second VM cannot acquire while the first holds.
- duplicate: `lease_release_makes_key_available` (src/relay_service.rs:799) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). Both pin owner release returning the state to Available (relay.rs proves it by the next acquire succeeding).
- duplicate: `lease_release_wrong_vm_does_not_release` (src/relay_service.rs:807) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). Both pin wrong-owner release is a no-op (relay.rs proves it by the lease staying busy).
- duplicate: `contention_second_vm_cannot_acquire_active_lease` (src/relay_service.rs:843) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). Same busy-exclusion; the only extra assertion (`id.as_u64() > 0`) is trivial LeaseId counter plumbing.
- duplicate: `hidraw_device_write_report_to_socket_succeeds` (src/relay_service.rs:946) - covered by `hidraw_device_write_report_prefixes_report_id` (src/relay_service.rs:964). Both pin `write_report` emitting the 64-byte report prefixed with report-ID byte 0x00; the keeper asserts length + prefix + payload explicitly (strict superset).
- trivial: `disabled_vm_is_not_registered_in_enabled_set` (src/relay_service.rs:854) - asserts a fresh `SecurityKeyState` has an empty `enabled_vms` set, i.e. a default value; the real behavior (disabled VM rejected) is pinned by `run_connection_rejects_disabled_vm` (src/relay_service.rs:1051). Nothing lost.

## Keep
- `parses_initialization_and_continuation_reports` (src/relay.rs:353) - pins init parse (cid/cmd/bcnt) and continuation parse (cid/seq); keeper of the parse family.
- `cid_translation_isolated_and_released` (src/relay.rs:370) - pins alloc, bidirectional mapping, and release; keeper of the translator family.
- `lease_busy_and_release_are_owner_bound` (src/relay.rs:382) - pins busy exclusion, owner-bound release, and re-acquire; keeper of the lease family.
- `parse_continuation_packet_identifies_seq_and_cid` (src/relay_service.rs:687) - pins cont parse cid/seq plus payload byte landing in `data` (extra not in the relay.rs parse test).
- `broadcast_cid_parsed_in_init_packet` (src/relay_service.rs:705) - pins broadcast CID parse boundary in init reports.
- `cid_translator_two_guests_get_different_host_cids` (src/relay_service.rs:731) - pins per-guest host CID uniqueness.
- `cid_translator_reallocating_guest_cid_removes_old_reverse_mapping` (src/relay_service.rs:739) - pins stale reverse mapping removed on re-alloc.
- `cid_translator_broadcast_passes_through` (src/relay_service.rs:755) - pins broadcast CID passthrough in both directions.
- `lease_expired_returns_is_expired_true` (src/relay_service.rs:816) - pins `is_expired` boundary for an overdue lease.
- `expired_lease_is_evicted_on_next_acquire` (src/relay_service.rs:827) - pins eviction transition on acquire.
- `framing_round_trip_over_buffer` (src/relay_service.rs:868) - pins length-prefixed framing round-trip.
- `framing_rejects_wrong_length_prefix` (src/relay_service.rs:886) - pins the wrong-length error path.
- `build_error_report_sets_correct_fields` (src/relay_service.rs:900) - pins error report fields (cid, CTAPHID_ERROR, bcnt=1, error code).
- `build_cancel_packet_targets_given_cid` (src/relay_service.rs:915) - pins cancel packet fields (cid, CTAPHID_CANCEL, bcnt=0).
- `hidraw_device_from_owned_fd_wraps_without_unsafe` (src/relay_service.rs:934) - pins fd wrap and EOF read error path.
- `hidraw_device_write_report_prefixes_report_id` (src/relay_service.rs:964) - pins report-ID prefix + full payload write; keeper of the write family.
- `authenticate_peer_accepts_matching_current_process_credentials` (src/relay_service.rs:988) - pins accept path of SO_PEERCRED check.
- `authenticate_peer_rejects_mismatched_expected_identity` (src/relay_service.rs:996) - pins `PeerCredentialMismatch` error path.
- `accept_socket_bind_creates_socket` (src/relay_service.rs:1007) - pins bind creates the socket with tightened 0o770 mode.
- `run_connection_rejects_mismatched_peer` (src/relay_service.rs:1027) - pins connection rejection on peer-credential mismatch.
- `run_connection_rejects_disabled_vm` (src/relay_service.rs:1051) - pins connection rejection for a VM not in `enabled_vms`.
- `run_connection_acquires_and_releases_lease` (src/relay_service.rs:1067) - pins full connection lifecycle: lease acquired then released back to Available.
- `sk_session_table_abort_on_duplicate_register` (src/relay_service.rs:1091) - pins duplicate register aborting the replaced accept loop and `stop_vm` aborting the live one.
- `physical_backing_claim_excludes_other_vms_until_release` (src/relay_service.rs:1122) - pins backing claim exclusivity and release.
- `live_same_vm_claim_is_adopted` (src/relay_service.rs:1135) - pins same-VM re-claim adoption with a live relay.
- `stopping_vm_releases_backing_and_aborts_relay` (src/relay_service.rs:1150) - pins `stop_vm` releasing backing and aborting the relay.
- `descriptors_declare_the_security_key_types` (src/driver.rs:565) - pins the two declared ResourceTypes, exportability flags, and relay creations.
- `rows_keep_the_preserved_identity` (src/driver.rs:607) - pins registration rows' resource types, effect ids, provider ref, and resync cadence.
- `security_key_relay_endpoint_purpose_is_a_closed_token` (src/driver.rs:634) - pins relay Endpoint purpose as a closed `BoundedToken` that decodes as `EndpointSpec`.