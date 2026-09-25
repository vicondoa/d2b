# d2b-provider-notification-desktop - unit-test audit
tests: 25 · src files: 17
net: -0 tests, -0 lines

## Findings (biggest net first)
No duplicate or trivial tests: all 25 pin distinct behavior - controller reconciliation invariants, sink delivery/revocation semantics, admission purpose checks, config bounds. Each was checked against same-file, same-crate, and this crate's `tests/*.rs` (notification_lifecycle.rs, provider_behavior.rs, redaction.rs, action_nonce.rs, stream_record.rs); the integration suite pins store-level nonce semantics, request bounds, and supervisor receipts, which complement rather than cover these unit tests.

- gap: the route-admission authentication surface is untested end to end - every invariant rejection in `SessionEvidence::from_daemon_route`/`from_daemon_route_for_guest`/`from_display_observer_route`/`from_display_dependency_route` (src/admission.rs:63,89,119,147), `DisplayDependencyEvidence::from_daemon_route` (src/controller.rs:116), and `NotificationRuntime::daemon_source_route_evidence`/`reconcile_daemon_routes` (src/runtime.rs:134) returns `SessionUnauthenticated`/`ReconciliationFailed`, and no test anywhere in the crate even constructs an `AuthenticatedSessionRouteBinding`. The daemon-local production path (d2bd wiring) runs on this code.
- gap: lifecycle identity constructor rejection branches - `NotificationSourceIdentity::new` (src/lifecycle.rs:25) and `NotificationHostSinkIdentity::new` (src/lifecycle.rs:90) reject wrong resource types, zero generations, and oversized digests with `notification-lifecycle-*-invalid`; only valid constructions are exercised (tests/notification_lifecycle.rs builds well-formed identities only).
- gap: `NotificationTelemetryFrame::validate_collector_fields` (src/metrics.rs:93) injection branches - duplicate keys, newline values, >128-char values, `d2b.provider` spoofing, and category mismatch are untested; only the forbidden-key (tests/redaction.rs) and closed-vocab (src/metrics.rs:157) rejections are pinned.

## Keep
- `purpose_specific_admission_rejects_cross_role_reuse` (src/admission.rs:361) - admit_source/admit_observer cross-role rejection.
- `session_key_binds_subject_zone_and_reconnect_generation` (src/admission.rs:371) - session_key differentiates subject, zone, generation.
- `zero_reconnect_generation_is_not_admitted` (src/admission.rs:380) - generation-0 session fails admit().
- `authenticated_source_validation_rejects_observer_reuse` (src/guest_source.rs:91) - observer reuse, binding mismatch, stale generation denied; success path.
- `delivery_requires_observer_purpose_and_returns_opaque_action_state` (src/host_sink.rs:573) - source-as-observer denied; opaque action keys issued; session-bound one-shot invoke.
- `delivery_rejects_cross_zone_source_and_observer_sessions` (src/host_sink.rs:607) - cross-zone delivery denied.
- `session_close_revokes_projection_nonces_and_idempotency` (src/host_sink.rs:623) - close_session revokes projections/nonces/idempotency; re-delivery re-issues.
- `idempotent_retry_does_not_return_expired_action_capabilities` (src/host_sink.rs:665) - TTL-expired idempotent retry re-delivers instead of replaying dead keys.
- `failed_delivery_does_not_evict_the_previous_projection` (src/host_sink.rs:688) - port failure returns SinkUnavailable, prior projection survives.
- `observer_policy_and_acknowledgement_timeout_are_enforced` (src/host_sink.rs:720) - observer-disabled rejection; ack-timeout evicts projection and revokes nonce.
- `telemetry_outcomes_and_categories_are_closed` (src/metrics.rs:157) - closed-vocab frame validates; out-of-vocab outcome rejected.
- `finalization_drains_the_effect_plan_before_releasing_authority` (src/runtime.rs:332) - finalize drains, releases authority once, idempotent.
- `authenticated_evidence_delivery_redacts_and_issues_bounded_action_keys` (src/runtime.rs:350) - end-to-end deliver_evidence redacts and issues bounded keys.
- `planning_requires_ready_same_zone_display_evidence` (src/controller.rs:1475) - Pending display omits host sink; cross-zone config rejected.
- `configured_display_dependency_is_exact_and_bounded` (src/controller.rs:1501) - config bounds: display dep exact, pending ≥8, nonce TTL ≥30, store ≥64.
- `ready_sink_requires_the_configured_display_dependency` (src/controller.rs:1523) - plan fails without configured display dependency.
- `disabled_dbus_sink_never_plans_or_restarts_the_host_sink` (src/controller.rs:1539) - dbus-disabled never plans or starts/stops the sink.
- `source_reconciliation_starts_stops_and_drains_exact_endpoints` (src/controller.rs:1557) - full lifecycle: start/stop exact endpoints, display-generation restart, Pending drains sink, recovery restarts, drain_sources.
- `source_generation_change_drains_and_restarts_the_exact_endpoint` (src/controller.rs:1616) - generation change yields both start and stop, endpoints carry generations.
- `duplicate_authenticated_source_sessions_are_rejected` (src/controller.rs:1636) - ambiguous evidence rejected.
- `missing_authenticated_source_stops_owned_endpoint_before_refusing` (src/controller.rs:1675) - missing evidence errors after effects drain; owned state cleared.
- `partial_source_evidence_drains_without_starting_a_valid_subset` (src/controller.rs:1691) - partial evidence drains subset, never starts it (exact plan asserted).
- `sink_policy_changes_restart_the_owned_host_sink` (src/controller.rs:1726) - observer-policy change restarts owned sink.
- `reconciliation_commits_source_ownership_only_after_effects_succeed` (src/controller.rs:1742) - failed effects leave prior ownership committed.
- `effect_receipts_bind_to_the_complete_plan_digest` (src/controller.rs:1768) - receipts match complete ack set and plan digest; incomplete ack refused.
