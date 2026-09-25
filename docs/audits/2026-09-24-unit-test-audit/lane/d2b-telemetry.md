# d2b-telemetry - unit-test audit
tests: 32 · src files: 8
net: -1 tests, -19 lines

(census regex reports 36: 4 `#[test]` hits are text inside `// Plain #[test] helper` comments in emitter.rs - real fns are 32)

## Findings (biggest net first)
- duplicate: `canonical_metric_frames_are_admitted` (src/emitter.rs:714) - covered by `emit_parses_one_shared_frame_for_metric_admission_and_redaction` (src/emitter.rs:768). Both emit the identical canonical metric frame (`d2b_api_watch_active`, `labels: {}`) and both assert `EmitOutcome::Buffered`; the keeper additionally asserts `raw_frame_parse_count() == 1`. The keeper strictly subsumes the deleted test (no buffered-frame byte asserted in either). -19 lines; no helper becomes dead (`socket_path`/`cleanup_socket`/`encode_frame` in use by the remaining tests).
- cross-check: metric-label policy tests (`metric_label_policy.rs` - `validate_label_key`/`validate_data_point`/`validate_labels`/`validate_descriptor`) exercise functions re-exported from `d2b-contracts-provider::v3::telemetry_policy`; the source crate's own unit test there covers only descriptor-registry closure, so no in-reach duplicate - C2: confirm no sibling tests pin these validators.
- cross-check: `trace_context.rs` and `audit_hash.rs` canonical-digest/Debug-masking surfaces mirror contract-type guarantees from `d2b-contracts-resource` (`is_canonical_digest`); no in-reach duplicate - C2: confirm no consumer crate re-tests them.

## Keep
- `hashes_have_one_canonical_shape` (audit_hash.rs:199) - canonical `sha256:` round-trip parses, Debug renders `<redacted>`, upper-case rejected.
- `record_hash_changes_when_predecessor_changes` (audit_hash.rs:207) - envelope digest binds the predecessor hash.
- `chain_link_verification_checks_sequence_when_supplied` (audit_hash.rs:217) - `verify_at` rejects a wrong sequence, accepts the right one.
- `frames_buffer_then_drain_fifo_when_socket_appears` (emitter.rs:589) - frames buffer while no socket, then drain FIFO (redacted) once a trusted socket appears.
- `ring_full_drops_oldest_frame_and_counts_its_signal` (emitter.rs:619) - byte-capacity overflow evicts the oldest frame and increments its signal drop counter.
- `metric_emission_rejects_an_out_of_policy_label` (emitter.rs:640) - typed `emit_metric` entrypoint rejects an out-of-policy label key before serialization/buffering.
- `raw_metric_frames_are_checked_before_buffer_admission` (emitter.rs:659) - raw frame with forbidden label key rejected by the raw `emit` path, nothing buffered.
- `raw_metric_frames_preserve_the_max_label_guard` (emitter.rs:680) - 17-label metric frame rejected `DescriptorMalformed` (label-count guard).
- `raw_metric_frames_require_a_canonical_descriptor` (emitter.rs:734) - unregistered metric name rejected by policy; nothing buffered.
- `expected_signal_is_checked_before_metric_shape` (emitter.rs:755) - signal mismatch rejected `FrameRedaction` before metric-shape validation.
- `emit_parses_one_shared_frame_for_metric_admission_and_redaction` (emitter.rs:768) - one parse serves both admission and redaction (`RAW_FRAME_PARSE_COUNT == 1`); canonical metric admitted `Buffered`.
- `raw_observation_frames_are_rejected_before_retention` (emitter.rs:790) - non-JSON raw bytes rejected `FrameRedaction`, nothing buffered.
- `forbidden_observation_fields_are_rejected_before_retention` (emitter.rs:802) - trace frame with an extra unallowlisted field rejected `FrameRedaction`.
- `raw_oversize_is_rejected_before_parse_or_queue_eviction` (emitter.rs:822) - oversize frame rejected `FrameTooLarge` without evicting the prior buffered frame.
- `identity_values_are_redacted_before_socket_export` (emitter.rs:839) - zone/path/env-TOKEN identity values redacted in bytes actually sent to the socket.
- `count_and_age_bounds_prune_deterministically` (emitter.rs:872) - frame-count and age limits prune deterministically, drain reflects the freed ring.
- `target_buckets_are_present` (meter_registry.rs:176) - SLO-critical latency bucket boundaries present in the exported bucket constants.
- `registry_rejects_identity_labels` (meter_registry.rs:183) - `MetricFamily::new` rejects a descriptor carrying a forbidden identity label key.
- `forbidden_identity_keys_fail_before_values` (metric_label_policy.rs:53) - every `FORBIDDEN_LABEL_KEYS` entry (and suffix-qualifying keys) rejected before value checks.
- `descriptor_and_identity_canary_validation_are_structural` (metric_label_policy.rs:66) - descriptor validates; a canary identity value becomes `ValueNotAllowlisted`.
- `untyped_labels_reject_out_of_policy_keys_and_identity_values` (metric_label_policy.rs:83) - `validate_labels` rejects a forbidden key and a non-allowlisted value.
- `resource_attributes_have_a_separate_allowlist` (metric_label_policy.rs:98) - OTEL resource attributes use their own allowlist (`d2b.zone` yes, `zone` no); digest value passes.
- `resource_allowlist_contains_v3_identity_attributes` (redaction_guard.rs:175) - `RedactionGuard::new` keeps allowlisted attrs (e.g. `d2b.zone`) while masking their identity values.
- `sensitive_span_fields_are_rejected` (redaction_guard.rs:187) - 9 high-risk span fields rejected (path/socket/argv/pid/realm/node/resource_ref/subject/no_isolation).
- `unknown_semantic_fields_and_values_fail_closed` (redaction_guard.rs:207) - unallowlisted span field and unallowlisted value both rejected.
- `canonical_semantic_fields_are_preserved` (redaction_guard.rs:219) - 6 closed semantic fields pass through with exact values, none masked.
- `session_metric_descriptors_have_no_resource_identity_labels` (session_metrics_sink.rs:117) - session `Connect` sink record with canonical profile/purpose_class/outcome labels succeeds end-to-end.
- `constructor_and_decoder_share_bounds` (trace_context.rs:111) - `TraceContext::new` and its serde decoder share the same bounds (empty/space identifiers rejected both ways).
- `debug_does_not_render_identifiers` (trace_context.rs:125) - `{:?}` renders `TraceContext(<redacted>)`, no identifier leakage.
- `child_span_propagates_only_the_validated_trace_id` (trace_context.rs:133) - child inherits the validated trace id, gets a new span id, rejects an invalid child span.
- `exported_ids_share_one_canonical_digest_domain` (trace_context.rs:142) - exported trace/span ids are canonical digests of the raw values under the shared domain.
