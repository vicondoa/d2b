# d2b-contracts-control - unit-test audit
tests: 34 · src files: 6
net: -1 tests, -23 lines

## Findings (biggest net first)
- duplicate: `audio_set_volume_rejects_out_of_range_level` (src/public_wire.rs:3711) - covered by `level_percent_validates_range_at_wire_boundary` (src/public_wire.rs:3688). Both pin wire-level rejection of out-of-range level 101; the args-level rejection is delegated to `LevelPercent::deserialize`, whose wire boundary the keeper already pins (plus construction range and cap-100 acceptance). The duplicate's remaining assertions (level 75 decodes, channel field echo) are plumbing.
- gap: `cli_output` module wire shapes (src/cli_output.rs, 411 lines) - zero tests anywhere in crate for its `deny_unknown_fields`/`skip_serializing_if`/camelCase output DTOs (`ListOutputV2`, `UsbProbeOutputV1`, `RealmListOutputV1`, …); a rename or field-shape drift ships unpinned.
- gap: `ProxyReadinessEvent::failed` path (src/proxy_readiness.rs:75) - only `ready()` is round-trip tested (`readiness_round_trip_is_path_free`); the failed variant's failure-reason serialization is unpinned.

## Cross-check (for C2)
- `vm_lifecycle_keeps_booted_variant` (src/public_wire.rs:2701) - unit-variant serde shape `"Booted"`; cross-check: generated-schema/contract tests in d2b-contracts may pin the same representation.
- `named_process_stream_frames_round_trip_without_identity_fields` (src/public_wire.rs:2707) - serde round-trip half; cross-check: schema/golden coverage. Debug-redaction half is unique to this crate.
- `console_public_wire_json_shape_is_stable` (src/public_wire.rs:3485), `audio_public_wire_json_shape_is_stable` (src/public_wire.rs:3574) - wire-shape pins; cross-check: tests/golden or consumer-side contract tests.
- `launch_requests_round_trip_and_correlate` (src/unsafe_local_wire.rs:487) - round-trip half; cross-check: d2b-contracts-resource identity tests. Wire-omission assertions (`request_id`/`operation_id` absent) are unique.

## Keep
- `readiness_round_trip_is_path_free` - proxy readiness event round-trips path-free, no `/run/` or argv in JSON.
- `zone_identity_fences_same_name_requests_and_excludes_realm_fields` - zone identity equality fences across zones; realm fields excluded from wire; legacy `realmId` rejected; Debug redacted.
- `zone_identity_changes_are_not_accepted_as_the_same_resource` - uid/generation changes break identity equality while `resource_ref` stays equal.
- `helper_requests_reject_non_execution_resource_identities` - non-`Process` resource refs rejected in `HelperLaunchRequest` and `HelperSnapshot`.
- `launch_requests_round_trip_and_correlate` - launch round-trips; `request_id`/`operation_id` omitted from wire.
- `helper_frames_reject_unknown_and_forbidden_fields` - hello/launch frames reject unknown payload fields and forbidden `cwd`.
- `older_helper_versions_are_rejected` - protocol v1/v2 rejected, v3 supported, constant pinned at 3.
- `realm_accent_color_is_strict_and_canonical` - lowercase-hex-only color validation at construction and wire.
- `helper_socket_buffer_floors_stay_closed` - buffer-size constant invariant (request ≤ frame, effective = 2× frame).
- `terminal_debug_redacts_sensitive_values` + `output_chunk_debug_redacts_payload` - terminal stdin/chunk Debug redacts session, keys, payload; exposes only length metadata.
- `exec_dto_debug_redacts_secrets` - sentinel sweep: no exec DTO Debug leaks argv/env/cwd/handles/stdio bytes; only shape counts observable.
- `shell_dto_debug_redacts_names_handles_and_output` + `shell_name_enforces_adr_shape` - shell DTO Debug redaction; ADR name shape (length, charset, no path/space/brace/newline) at construction and wire.
- `console_public_wire_json_shape_is_stable` + `console_session_handle_is_redacted_in_debug` - console wire shape (kind/op/args) stable; attach-result and args Debug never leak session handle.
- `audio_public_wire_json_shape_is_stable` - audio status/mute/setVolume wire shapes, enum strings, error kinds, round-trips.
- `level_percent_validates_range_at_wire_boundary` - 0..=100 round-trips, 101 rejected at construction and wire, cap accepted.
- `audio_status_unknown_fields_fail_closed` - `AudioStatusArgs` rejects unknown fields.
- `vm_lifecycle_keeps_booted_variant` - `VmLifecycleState::Booted` serializes to `"Booted"`.
- `named_process_stream_frames_round_trip_without_identity_fields` - stream request/response/frame round-trips; `session` absent from wire; Debug redacts output and error slugs.
- `public_response_deserializes_success_envelopes` - capabilities/auth-status/list/status/audit success envelopes decode through the public contract.
- `public_response_deserializes_error_envelope_losslessly` - error envelope round-trips losslessly with kind/code/message.
- `paginated_audit_response_decodes_through_public_contract` + `complete_audit_page_omits_cursor` + `incomplete_audit_page_requires_cursor` + `audit_response_rejects_complete_cursor_and_unknown_legacy_fields` - audit pagination contract: cursor decode, complete-page cursor omission, incomplete-page cursor requirement (error names `nextCursor`), complete+cursor and unknown/legacy fields fail closed.
- `status_payload_rejects_unknown_fields` - status payload unknown field rejected through frame encode/decode.
- `vm_lifecycle_force_defaults_false_for_compatibility` + `vm_lifecycle_omits_false_force_but_serializes_true` - legacy payloads default `force`/`no_wait_api` false; serialization skips false `force`, emits true.
- `runtime_summary_omits_default_runtime_seam_fields` + `runtime_summary_serializes_positive_capabilities_and_services` - default capability/service fields omitted; positive values and role enum serialize canonically.
- `usb_enroll_is_not_public_wire` - removed `usb enroll` verb fails closed on the public wire.