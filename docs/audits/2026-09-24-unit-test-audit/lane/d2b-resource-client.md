# d2b-resource-client - unit-test audit
tests: 40 · src files: 8
net: -2 tests, -37 lines

Note: census counts 41; actual `#[test]`/`#[tokio::test]` fns = 40. The 41st census hit is a comment (`// Plain #[test] harness driving one async call synchronously;` at src/process_attach.rs:1376), not a test.

## Findings (biggest net first)
- duplicate: `a_zone_outside_the_table_is_refused_before_any_call_is_prepared` (src/client.rs:166) - covered by `route_lookup_is_keyed_on_the_exact_zone_path` (src/target.rs:661). Both pin: a target whose zone path has no route record resolves to `ClientError::RouteUnavailable`. The client test re-asserts the identical refusal through the one-line `ResourceClient::resolve` delegate (client.rs:53), adding nothing the table-level test (which also pins owner-class keying and carriage mismatch) does not; the facade composition path is already exercised by the keeper `a_k0_caller_reaches_k1_over_the_uplink_and_k0_locally`.
- duplicate: `a_cancelled_cross_zone_call_is_refused_at_the_client_boundary` (src/client.rs:181) - covered by `cancellation_is_forwarded_before_every_attempt` (src/dispatch.rs:495). Both pin: `CallDriver::begin_attempt` with a cancelled token returns `ClientError::Cancelled`. The dispatch test pins strictly more (retryable failure first, `attempts_made` unchanged, cancelled session failure terminal); the client test only adds the `prepare_call` delegation hop (client.rs:66), and the facade wiring is already covered by the keeper client test.
- gap: `record_remote_verdict` classification has no test (dispatch.rs:293; reached via process_attach.rs:795 and zone_client.rs:946) - the reduced-verdict classifier (Immediate → RetryNow; AfterDelay/Never/Reauthorize → terminal; attachments/exhausted budget suppress) is distinct logic from `record_remote_error` and is entirely unpinned.
- gap: `GuestControlEndpoint::new` rejection path untested (zone_client.rs:163) - non-Endpoint/non-Guest ref types, empty zone, zero generations, and not-ready must each refuse `InvalidTarget`; only the success path is tested.
- gap: `GuestControlEndpoint::validate_for` mismatch untested (zone_client.rs:242) - any identity/generation/schema-digest mismatch must refuse `TransportPolicyMismatch`; zero coverage.

## Keep
- `metadata_lifetime_bounds_fail_closed` (call.rs:363) - lifetime bounds: issued=0, zero/negative lifetime, >MAX rejected; MAX boundary accepted.
- `optional_metadata_fields_are_bounded` (call.rs:387) - correlation (empty/non-ASCII/>64), trace, idempotency (empty/>64) bounds + accessors.
- `retry_policy_bounds_are_exact` (call.rs:421) - RetryPolicy 0 and MAX+1 rejected; 1 and MAX accepted; `no_retry()` = 1.
- `cancellation_is_shared_idempotent_and_never_renders_state_as_identity` (call.rs:439) - token clone shares state, cancel idempotent, Debug renders state not identity.
- `metadata_debug_never_echoes_an_identifier` (call.rs:453) - MetadataInput Debug redacts request/correlation/trace/idempotency bytes.
- `retry_backoff_refuses_without_a_caller_runtime` (call.rs:471) - no-executor caller gets `RetryBackoffUnavailable`, not a panic.
- `retry_backoff_rides_the_caller_runtime_timer` (call.rs:485) - delay rides the caller's Tokio timer.
- `retry_backoff_observes_cancellation` (call.rs:493) - pre-cancelled token refuses immediately.
- `a_k0_caller_reaches_k1_over_the_uplink_and_k0_locally` (client.rs:122) - facade composition: resolve local + cross-zone routes, both drive identical call policy (begin_attempt, Disconnected→RetryNow, RetryLimitExceeded).
- `method_profiles_fail_closed_on_contradictory_declarations` (dispatch.rs:413) - MethodProfile rejects non-mutating-with-key, zero/over-MAX lifetime; valid write profile accessors.
- `admission_requires_a_matching_service_an_idempotency_key_and_a_live_deadline` (dispatch.rs:440) - CallDriver::new rejects service mismatch, missing key, expired deadline.
- `the_attempt_budget_is_exact_and_exhaustion_is_typed` (dispatch.rs:480) - attempts 1..2 admitted, 3rd → RetryLimitExceeded, attempts_made counts.
- `cancellation_is_forwarded_before_every_attempt` (dispatch.rs:495) - cancelled token refused before next attempt; cancelled failure terminal.
- `session_failure_classification_matches_the_carried_over_policy` (dispatch.rs:519) - RetryNow vs Fail mapping per failure class; unkeyed mutating never retries; ambiguous mutating terminal; attachments never replayed.
- `an_exhausted_budget_reports_the_retry_limit_rather_than_the_last_failure` (dispatch.rs:578) - record_session_failure on exhausted budget → RetryLimitExceeded.
- `a_peer_verdict_is_an_input_and_is_never_widened` (dispatch.rs:589) - remote error → RetryNow/RetryAfterMs/Fail per retry class; attachments and exhausted budget suppress retry.
- `the_attempt_timeout_never_exceeds_the_method_ceiling` (dispatch.rs:647) - ticket timeout ≤ method max lifetime.
- `every_label_is_unique_stable_and_client_prefixed` (error.rs:125) - all ClientError labels unique, `client-` prefixed, Display == label.
- `a_remote_refusal_collapses_to_one_low_cardinality_label` (error.rs:137) - every Remote kind/retry combo → `client-remote`.
- `authorized_attach_opens_a_named_stream_and_closes_once` (process_attach.rs:1023) - attach opens named stream, requests Zone service, send/receive round trip, close idempotent.
- `process_attach_stream_round_trips_the_shared_named_frame_codec` (process_attach.rs:1062) - frame codec serializes request, deserializes response.
- `cancellation_stops_a_pending_open_and_does_not_leak_a_stream` (process_attach.rs:1104) - cancellation during pending open → Cancelled.
- `retry_classification_retries_transport_but_not_authorization` (process_attach.rs:1137) - TransportFailed retried (2 opens), AuthorizationDenied not (1 open).
- `wrong_resource_type_and_wrong_zone_fail_before_open` (process_attach.rs:1196) - target type validation and wrong-zone RouteUnavailable before open.
- `shell_session_target_requires_qualified_type_and_host_or_guest_execution` (process_attach.rs:1237) - shell target requires SHELL_SESSION_TYPE and Host/guest host ref.
- `local_root_attach_uses_the_local_zone_owner_route` (process_attach.rs:1268) - local-root attach via ZoneLocal route succeeds.
- `reused_session_evidence_is_denied_by_the_zone_pin` (process_attach.rs:1312) - session pin mismatch → TransportPolicyMismatch, zero opens.
- `attach_inputs_and_diagnostics_are_bounded_and_redacted` (process_attach.rs:1337) - TerminalSize/ProcessAttachOptions bounds; Debug redaction of target and open request.
- `service_and_transport_labels_are_unique_and_stable` (target.rs:535) - exact service label catalogue + uniqueness.
- `every_target_names_exactly_one_routing_zone` (target.rs:574) - each TargetInput variant class + owner zone; declared_service.
- `resource_targets_preserve_the_exact_canonical_reference` (target.rs:647) - Resource target keeps canonical ref string.
- `route_lookup_is_keyed_on_the_exact_zone_path` (target.rs:661) - route table keying: local/child resolve, unknown zone refused, owner-class keying, carriage mismatch.
- `resolution_fails_closed_on_service_carriage_and_ambiguity` (target.rs:730) - declared-service contradiction, unspecified carriage, duplicated record → typed refusals.
- `debug_renderings_never_echo_a_zone_label_or_a_resource_name` (target.rs:778) - Debug redaction across target/owner/record/resolved/table.
- `resource_method_inventory_is_the_contract_catalogue` (zone_client.rs:999) - 13 ResourceVerb catalogue, mutating classification.
- `peer_and_session_pins_are_exact_and_redacted` (zone_client.rs:1007) - pin matches target; wrong peer refused; Debug redacted.
- `zero_session_generation_is_rejected` (zone_client.rs:1039) - generation 0 → ContractViolation.
- `guest_control_endpoint_keeps_only_exact_redacted_identity` (zone_client.rs:1054) - endpoint uid/zone accessors; Debug redacts refs and uid.