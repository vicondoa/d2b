# d2b-provider-toolkit — unit-test audit
tests: 69 · src files: 32
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: `ProviderEntrypoint` name validation and builder double-set guards (src/base/runtime.rs:249-252, 321-322, 336-345, 356-358, 370-372) — `ProviderRuntimeError::InvalidName` (empty/over-128/non-ASCII name, non-`Provider` ref in `with_provider`, repeat `with_execution_target`/`with_controller_process`/`with_generations`) has no test in src or tests/; every other `ProviderRuntimeError` variant is pinned by at least one test.

Nothing to cut. Ship. Checked all 69 unit tests across 32 src files (credential, bootstrap, runtime, server, shared-provider driver, plane, declaration, audit, testing fakes/fixture/conformance) against each other, the crate's 5 integration files, and the error-enum surfaces; every test pins a distinct observable behavior, boundary, or error path, with no intra-crate duplicates and no trivial plumbing asserts.

## Keep
- `a_denied_call_yields_no_identity_bearing_record` — denied credential audit produces no record.
- `an_authorized_call_never_renders_the_presented_identity` — success record's wire form and Debug never contain the presented identity.
- `a_frame_carries_only_closed_values` — telemetry frame fields all validate as closed collector values.
- `a_future_that_arms_a_timer_while_polling_completes_without_a_runtime` — credential `block_on` enters a runtime so a timeout-arming future completes.
- `factory_serves_the_declared_rows` — driver factory serves exactly the family's declared resource types.
- `validate_rejects_a_provider_outside_the_family` — row naming a Provider outside the family fails validation terminally.
- `reconcile_commits_the_child_set_then_runs_the_effect` — child set committed before effect; pending pass self-requeues with cadence.
- `reconcile_leaves_rows_of_a_child_less_component_alone` — undeclared rows survive a reconcile pass.
- `delete_runs_provider_teardown_then_retires_children` — finalize before endpoint before process retirement order.
- `delete_is_retryable_while_the_provider_stage_is_pending` — pending teardown yields retryable delete failure.
- `recover_adopts_the_committed_child_set` — adopt only when complete child set committed; recovery runs no effect.
- `audit_capacity_is_closed_and_bounded` — audit log capacity bounds and ring-buffer drop semantics.
- `an_event_never_renders_its_principal_or_method_in_debug` — audit event Debug redacts principal, zone, provider.
- `a_wrapped_value_never_renders_itself` — Redacted Debug/Display render `<redacted>`; expose/into_inner recover.
- `a_nested_wrapped_value_never_leaks_through_a_container` — redaction survives container Debug.
- `an_allocator_issued_binding_admits_the_expected_agent_identity` — matching bootstrap binding admitted with identity.
- `a_binding_for_another_provider_or_zone_fails_closed` — provider/zone mismatch errors.
- `a_binding_that_is_not_a_provider_resource_fails_closed` — non-Provider resource type refused.
- `a_wrong_purpose_or_non_local_session_fails_closed` — purpose mismatch and remote-locality rejection.
- `the_identity_never_renders_its_principal_in_debug` — bootstrap identity Debug redacts zone and provider.
- `every_code_is_unique_and_matches_the_frozen_grammar` — ProviderToolkitError closed code set.
- `readiness_is_not_published_before_registration` — lifecycle Starting→Ready→Stopped; no controller readiness pre-registration.
- `readiness_io_failure_does_not_enter_ready_state` — publish_ready_to IO error stays Starting.
- `draining_refuses_new_registration` — drain blocks while registration held; admit refused during drain.
- `drain_completes_when_the_last_registration_is_released_on_the_polling_runtime` — single-threaded runtime drain observes same-runtime release.
- `authenticated_readiness_requires_the_live_route_and_registration` — matching trio ok; wrong service version or foreign registration refused.
- `controller_readiness_requires_a_controller_generation` — route without controller generation refused.
- `a_foreign_session_admission_cannot_publish_this_entrypoint_ready` — foreign admission+route pair refused against local registration.
- `reconnect_rebind_requires_the_same_controller_identity_and_a_new_generation` — rebind ok on new generation, old generation refused.
- `signed_controller_construction_rejects_non_launchable_or_unsupported_roles` — Guest signed controller ok; Zone target, in-process controller, host route, generation/process-bound refs refused.
- `session_admission_rejoins_a_matching_operation_row` — admit New→Existing; rebind updates session generation.
- `dead_session_route_cannot_admit_or_rebind_an_operation` — dead route refused; original operation row preserved.
- `generated_schema_is_canonicalizable` — canonical schema emission has no trailing newline.
- `a_pair_two_creators_declare_authorizes_only_the_driver_owned_row` — creation fence authorizes driver-owned row, refuses controller-owned twin.
- `the_limit_is_closed_and_bounded` — DispatchLimiter bounds and default ceiling.
- `saturation_refuses_and_a_released_permit_restores_the_slot` — acquire/release slot accounting.
- `credential_service_dispatches_a_valid_typed_revoke` — RevokeToken dispatch yields Revoked state/outcome.
- `credential_service_refuses_a_stale_authenticated_route` — stale route refused at dispatch.
- `route_authorization_builds_a_bounded_delivery_binding` — delivery binding carries uid/generation and bounded token bytes.
- `shutdown_drains_an_admitted_request_before_reporting_idle` — shutdown blocks on in-flight permit; server retires after release.
- `generated_server_admits_and_dispatches_one_closed_method` — health dispatch returns state; one generated service.
- `generated_server_requires_an_authenticated_controller_route_for_session_serving` — matching route binds; wrong service refused.
- `authenticated_request_accepts_route_bound_identity_and_authorization` — matching request accepted.
- `authenticated_request_rejects_retargeted_zone_or_provider` — wrong zone and forged provider refused.
- `authenticated_request_rejects_mismatched_authorization_before_dispatch` — wrong service or target in authorization refused.
- `authenticated_request_rejects_missing_provider_route` — route without provider refused.
- `provider_route_validation_rejects_identity_service_and_generation_mismatches` — wrong provider/service/missing generation refused.
- `a_new_provider_resource_type_binding_passes_descriptor_and_live_conformance` — descriptor + live conformance ok path.
- `a_binding_for_an_uninstalled_resource_type_fails_closed` — ResourceTypeNotInstalled.
- `a_divergent_base_fingerprint_fails_closed` — BaseSchemaMismatch.
- `a_descriptor_with_no_binding_or_a_repeated_binding_fails_closed` — NoResourceTypeBinding and DuplicateResourceTypeBinding.
- `a_provider_extension_is_not_a_minimal_base_spec` — MinimalBaseRejected.
- `a_capability_is_refusable_only_when_the_matrix_declares_it_unsupported` — capability disposition/refuse matrix semantics.
- `a_capability_named_on_both_sides_is_rejected` — CapabilityMatrixOverlap.
- `the_closed_code_set_check_rejects_empty_duplicate_and_malformed_sets` — checker rules: empty/duplicate/grammar/length.
- `every_conformance_code_is_unique_and_matches_the_frozen_grammar` — ConformanceError closed code set.
- `every_refusal_code_is_unique_and_matches_the_frozen_grammar` — FakePortError closed code set.
- `a_fault_plan_is_consumed_call_by_call` — scheduled faults consumed; exhausted plan stops injecting.
- `the_bus_resolves_one_alias_and_never_a_table` — alias resolution and recording.
- `the_store_refuses_a_status_write_outside_the_owned_set` — owned-set enforcement.
- `a_recorded_call_renders_nothing_it_was_handed` — CallRecorder Debug redacts handed values.
- `an_injected_fault_stops_the_effect_from_being_recorded` — fault prevents recording; later call recorded.
- `every_fixture_publishes_a_closed_descriptor_and_deterministic_time` — all provider classes build valid fixtures with fixed clock.
- `the_fake_provider_returns_health_inspection_and_observability_in_order` — conformance sequence accepted; call count 3.
- `the_clock_is_explicit_and_never_wraps` — DeterministicClock saturates at u64::MAX.
- `a_ready_future_completes_on_the_calling_thread` — harness noop-waker `block_on` completes a ready future without a runtime.
- `an_undeclared_type_cannot_be_admitted` — undeclared type refused with `undeclared-type`.
- `a_declared_reference_must_resolve_before_the_spec_is_committed` — unresolved ref refuses commit; resolves once row committed.
- `attaching_claims_only_declared_roots` — empty declaration attaches with no plane calls.