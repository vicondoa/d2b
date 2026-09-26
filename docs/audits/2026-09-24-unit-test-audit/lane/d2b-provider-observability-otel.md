# d2b-provider-observability-otel - unit-test audit
tests: 28 · src files: 8
net: -2 tests, -43 lines

## Findings (biggest net first)
- duplicate: `policy_runs_before_capacity_and_rejects_the_whole_frame` (src/ingress_policy.rs:855) - covered by `every_ingress_covers_each_policy_failure_and_capacity_rejection` (tests/ingress_metric_policy.rs). Both pin: valid frame with capacity=false → (Rejected, None) and `vm`/`work` label frame → (Rejected, KeyForbidden). `admit()` is literally `admit_for_connection(ingress, 0, …)` (src/ingress_policy.rs:389-395), and the integration test runs the same path for all 4 ingresses incl. EmitterUnix, plus the other 6 policy classes - strictly more behavior.
- duplicate: `raw_unknown_descriptor_is_rejected_before_series_accounting` (src/ingress_policy.rs:1004) - covered by `repeated_unknown_families_cannot_bypass_the_closed_descriptor_registry` (src/ingress_policy.rs:1079). Both pin: unknown descriptor via `admit_raw` → (Rejected, Malformed) and `series_count() == 0`; the 64-iteration loop's first iteration is exactly this scenario and pins strictly more (distinct repeated families cannot bypass the closed registry).
- cross-check: `forbidden_identity_keys_fail_structurally` / `descriptor_validation_rejects_identity_canaries` / `data_point_label_set_mismatch_precedes_actual_label_policy` (src/metric_policy.rs:97,110,125) re-pin behavior implemented in d2b-contracts-provider v3 telemetry_policy (validate_label_key, validate_data_point, FORBIDDEN_LABEL_KEYS are re-exports). If the contracts crate's unit tests pin the same policy, these are cross-crate duplicates - C2 to resolve.
- cross-check: remaining ingress-policy unit tests overlap tests/ingress_metric_policy.rs (7 policy classes, quarantine-at-threshold matrix) but each pins extra (admit_raw path, injected-clock expiry, credit accounting, series accounting) - kept here; C2 may re-verify the `policy_runs_before_capacity` citation.
- gap: EmitterSocket::bind parent validation (`validate_socket_parent`, src/emitter_socket.rs:349-379) - non-absolute path, symlink parent, and world-writable parent rejections are untested anywhere in the crate; this is the deny-by-identity socket boundary.
- gap: bounded-storage eviction (byte-budget eviction loop and MAX_RETAINED_AGE `prune_expired`, src/emitter_socket.rs:216-230, 337-346) - no test exercises eviction or age-based drop.
- gap: `ProviderAgentProcess::process_effect` (src/agent.rs:279-304) and the `AuditBackpressure` error path (src/agent.rs:326-330) - process_effect's closed-token parsing and the full-ring rejection have no test; only session_connect is covered.

## Keep
- `agent_emits_session_connect_event_without_payload` (src/agent.rs:373) - pins event field shape and zone redaction in JSON and Debug output.
- `agent_rejects_unknown_outcomes_without_retaining_input` (src/agent.rs:393) - pins invalid outcome → InvalidInput with no event retained.
- `only_self_metrics_is_accepted` (src/config.rs:186) - pins strict config shape: empty → default true, unknown key and non-bool `enable` → Invalid.
- `ambient_exporter_credential_chains_are_rejected_without_reading_values` (src/config.rs:204) - pins ambient credential-chain rejection by name only; benign keys pass.
- `receiver_drains_datagrams_and_reports_ready` (src/emitter_socket.rs:405) - pins drain→Ready transition, pop returns the redacted canonical frame.
- `receiver_uses_closed_descriptor_accounting_before_queue_insertion` (src/emitter_socket.rs:431) - pins unknown-descriptor frames dropped before queue insertion, valid frames queued.
- `receiver_redacts_or_drops_forbidden_frames_within_bounds` (src/emitter_socket.rs:461) - pins parsed-frame redaction (no canary leak) and malformed datagram drop.
- `inode_checked_drop_does_not_remove_replacement_socket` (src/emitter_socket.rs:485) - pins inode-checked unlink: Drop never removes a replacement socket at the same path.
- `import_stream_has_no_credits_after_quarantine` (src/ingress_policy.rs:881) - pins quarantine at threshold AND `available_import_credits_for == 0` (credit assertion not in integration test).
- `quarantine_expires_on_injected_clock_and_disconnect_releases_state` (src/ingress_policy.rs:902) - pins injected-clock quarantine expiry, prune, and reset restoring credits.
- `raw_emitter_admission_enforces_the_provider_series_cap` (src/ingress_policy.rs:925) - pins series cap on the admit_raw JSON path.
- `raw_admission_counts_resource_attributes_in_series_identity` (src/ingress_policy.rs:971) - pins resource attributes in series identity through the raw path.
- `raw_known_descriptor_requires_its_canonical_label_set` (src/ingress_policy.rs:1025) - pins canonical label-set enforcement on the raw path (missing label and non-canonical value → Malformed).
- `repeated_unknown_families_cannot_bypass_the_closed_descriptor_registry` (src/ingress_policy.rs:1079) - pins closed registry against repeated distinct unknown families.
- `resource_attributes_are_part_of_provider_series_identity` (src/ingress_policy.rs:1100) - pins provider-cap series identity including resource attributes (structured path).
- `resource_attributes_count_toward_the_identified_producer_quota` (src/ingress_policy.rs:1134) - pins resource attributes counting toward per-producer quota.
- `shared_series_survives_the_first_producer_disconnect` (src/ingress_policy.rs:1165) - pins shared-series membership surviving one producer disconnect, release on both.
- `expiry_removes_only_the_expired_producer_membership` (src/ingress_policy.rs:1208) - pins clock-based membership expiry removing only the expired producer.
- `shared_emitter_scope_does_not_create_a_fake_producer` (src/ingress_policy.rs:1261) - pins connection-id-0 emitter scope never creating producer entries.
- `series_cap_reclaims_only_after_monotonic_idle_expiry_or_connection_reset` (src/ingress_policy.rs:1286) - pins cap reclaim timing: no reclaim before idle horizon, reclaim after, reset reclaims.
- `identified_producer_quota_leaves_capacity_for_later_valid_series` (src/ingress_policy.rs:1377) - pins per-producer quota not starving other producers.
- `forbidden_identity_keys_fail_structurally` (src/metric_policy.rs:97) - pins every FORBIDDEN_LABEL_KEYS entry and identity-suffix keys rejected structurally. [cross-check: contracts-crate policy tests]
- `descriptor_validation_rejects_identity_canaries` (src/metric_policy.rs:110) - pins identity canaries rejected as ValueNotAllowlisted. [cross-check: contracts-crate policy tests]
- `data_point_label_set_mismatch_precedes_actual_label_policy` (src/metric_policy.rs:125) - pins LabelSetMismatch precedence over value policy. [cross-check: contracts-crate policy tests]
- `resource_attributes_have_a_separate_allowlist` (src/metric_policy.rs:136) - pins local `validate_resource_attributes`: allowlisted keys with canonical digest pass, non-allowlisted key and credential-bearing value fail.
- `self_metric_descriptor_uses_closed_labels` (src/metrics.rs:24) - pins the self-metric descriptor validating against closed-label policy and all SELF_METRICS names canonical.
