# d2b-provider-zone-link - unit-test audit
tests: 42 · src files: 4
net: -1 tests, -9 lines

## Findings (biggest net first)
- duplicate: `matching_owner_proof_adopts_one_cursor` (src/zonelink.rs:426) - covered by `handler_owns_restart_cursor_adoption` (src/zonelink.rs:472). Both pin: one observation carrying the matching owner proof adopts the cursor and makes it retrievable via `cursor()`; the controller-level test additionally pins the handler record cursor sync, so the authority-only test adds nothing.
- gap: `RoutePolicyCommitted` refusal paths (src/zone_links.rs:1687) - missing route binding, `OperationClass::Attach` verb, and non-increasing `policy_revision` all fail closed with `RouteAdmissionBindingInvalid`, but no test asserts any of them (the event appears only as a superseding commit in `aborted_or_stale_route_passes_cannot_issue_admission`); the policy update itself is never asserted.
- gap: `transport_error_is_quarantine` (src/zonelink.rs:332) - the public StaleCommitProof/ReconcileInFlight → quarantine mapping is exercised nowhere in the crate.
- gap: `PskIssued` monotonicity refusal (src/zone_links.rs:1427) - an issuance event at or below the highest recorded issuance fails closed with `BootstrapPskInvalidated`, but tests only feed increasing issuances; only the admission-side invalidation is pinned (`issuing_a_fresh_psk_invalidates_the_prior_outstanding_psk`).

## Keep
- `durable_route_binding_and_bootstrap_psk_generation_are_exact` - cross-generation binding/PSK refused; route binding Debug redacts uid.
- `committed_route_operation_ids_are_non_lossy_across_intervening_commits` - dup op id across commits → Conflict; set non-lossy.
- `committed_route_operation_history_refuses_without_eviction_at_capacity` - capacity ceiling: replay → Conflict, new id → OperationCapacity, no eviction.
- `multiple_committed_route_ids_survive_versioned_restart_recovery` - dedup envelope roundtrip, version mismatch fail-closed, restart refuses committed ids.
- `aborted_route_ids_are_reusable_but_recreated_identity_is_isolated` - aborted id reusable; stale forensic state refused across identities.
- `route_admission_requires_ready_and_adopted_cursor` - Admission gated on Ready + adopted cursor; clearing adoption after commit fails issue.
- `route_admission_uses_only_committed_identity_and_daemon_issuer` - context carries only committed identity; issuer receives only the request.
- `aborted_or_stale_route_passes_cannot_issue_admission` - aborted pass and superseded proof cannot issue; committed id → Conflict.
- `stale_reconnect_generation_cannot_reuse_a_route_proof` - generation advance fences old proof, updates binding, tears down session.
- `route_admission_rejects_substitution_revocation_and_restart_ambiguity` - verb mismatch, restart-stale proof, revoked refusal.
- `forward_transitions_reach_ready_in_the_canonical_order` - canonical Unenrolled→IKpsk2→EnrollmentCommitted→Kk→Ready progression with phase/epoch/authorization.
- `a_second_pass_is_refused_while_one_is_open` - ReconcileInFlight; abort reopens.
- `an_aborted_pass_mutates_nothing_and_releases_no_effect` - abort leaves record untouched.
- `effects_require_a_commit_proof_and_release_exactly_once` - record mutates at commit before effects; fresh proof releases once.
- `a_stale_proof_releases_nothing` - stale proof refused on release.
- `resource_traffic_before_ready_is_refused_in_every_pre_ready_state` - traffic refused in all pre-ready states.
- `an_expired_bootstrap_psk_is_refused` - expiry boundary fail-closed.
- `issuing_a_fresh_psk_invalidates_the_prior_outstanding_psk` - admission-side PSK invalidation.
- `a_failed_bootstrap_handshake_burns_the_psk_and_returns_unenrolled` - handshake failure durable, PSK burned.
- `a_consumed_psk_crash_fails_closed_and_refuses_psk_reuse` - crash-after-burn restart refuses reuse, admits fresh PSK.
- `a_persist_crash_before_the_enrollment_commit_stays_unenrolled` - uncommitted seal lost on restart.
- `a_teardown_crash_rederives_unenrolled_from_the_invalidation_marker` - crash after revoke commit re-derives Unenrolled.
- `revocation_requires_a_fresh_psk_and_a_new_bootstrap` - revoke clears session, refuses old PSK, re-bootstraps.
- `a_pre_revocation_static_key_is_refused_before_any_resource_exchange` - key mismatch → Degraded, no effects, no connection.
- `an_enrolled_key_mismatch_retries_only_under_the_reconnect_budget` - budget exhaustion → Failed + ReconnectBudgetExhausted.
- `an_expired_kk_session_rehandshakes_from_the_enrollment_record` - lifetime expiry re-handshakes without PSK, epoch advances, no downgrade.
- `reconnect_reenters_at_kk_without_consuming_a_psk` - disconnect + restart re-enters at enrolled handshake, PSK untouched.
- `replaying_a_committed_seal_after_restart_is_idempotent` - seal replay is a no-op.
- `the_intent_queue_is_bounded_and_drains_only_when_ready` - queue ceiling + drain gate.
- `cursor_resync_is_monotonic_and_survives_restart` - per-field monotonic advance, restart-safe.
- `advertisement_renewal_reissues_only_from_ready` - issuance/renewal from Ready, refused after disconnect.
- `disabling_withdraws_advertisements_and_suppresses_reconnect` - disable withdraws/tears down, gates handshake+traffic, re-enable admits.
- `limits_reject_out_of_range_values` - InvalidLimits boundaries + default policy constants.
- `metric_labels_carry_no_identity` - label keys and sample values exclude identity material.
- `debug_surfaces_redact_identity_and_key_material` - Debug output redacts uid/fingerprint/generation.
- `every_error_label_is_a_bounded_lowercase_token` - all error labels lowercase kebab ≤64, Display == label.
- `missing_or_ambiguous_owner_is_quarantined` - no observation → OwnerProofMissing; differing proofs → AmbiguousOwner.
- `duplicate_owner_observations_are_quarantined` - identical observations still ambiguous (count-based, never dedup).
- `handler_owns_restart_cursor_adoption` - controller adopt_cursor syncs handler record.
- `controller_issues_only_after_unique_cursor_adoption_and_committed_state` - controller-level issue gating, wrong-owner quarantine, dup → Conflict.
- `matching_owner_with_substituted_cursor_stays_quarantined` - cursor ≠ record cursor → CursorInvalid.
