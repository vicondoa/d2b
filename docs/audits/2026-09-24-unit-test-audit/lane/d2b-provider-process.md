# d2b-provider-process - unit-test audit
tests: 56 · src files: 12
net: -1 tests, -31 lines

## Findings (biggest net first)
- duplicate: `durable_provider_effect_launch_failure_still_retries_under_the_budget` (src/driver.rs:4334) - covered by `retryable_reconcile_failure_requeues_exactly_once_with_backoff` (src/driver.rs:3830). Both pin the same behavior under the default restart policy: a provider launch-effect failure outside the terminal unresolvable-ticket set classifies Retryable, consumes one in-memory restart, and the next reconcile reports `RetryScheduled` with exactly one 1s policy-backoff requeue; the keeper additionally drives the backoff to a successful relaunch. The only assertion the duplicate adds (`restart_count() == 1`) is already pinned by `restart_budget_is_in_memory_only` (src/driver.rs:3882).
- gap: family operation handlers (src/operations.rs) - the 11 declared operations (OpenPidfd, SpawnRunner, CgroupKill, …) with their fail-closed refusal paths (`kernel-seam-unwired`, intent mismatch, `inherited-fds-unsupported`, `runner-unknown`) have no `#[cfg(test)]` or `tests/` coverage anywhere in this crate; the crate's tests never enter the handlers.
- gap: validate `ExecutionUnsupported` (src/driver.rs `validate`) - every driver test runs Host mode with `Host/host-system` refs, so `process-execution-unsupported` (a Guest `executionRef` under a Host driver, and the whole Guest-mode `execution_target_allowed` branch) has no test.
- gap: `restart_delay` multiplier/cap (src/driver.rs `restart_delay`) - every restart test observes restart_count == 1 (delay == base); the `base * multiplier^(count-1)` capped-at-`backoff_max` arithmetic for a 2nd+ restart is never exercised.

## Keep
- `identity_falls_back_to_the_authored_owner_reference` - pins owner-key-absent fallback to authored `metadata.ownerRef`.
- `guest_runtime_row_targets_its_owning_guest` - pins Guest-owned guest-runtime VMM row targeting its owning Guest; non-guest-runtime template keeps target unbound.
- `binding_owned_worker_identity_targets_the_attachment_guest` - pins binding-owned worker: owner ref = binding, target = attachment Guest, is_binding_worker, vm/launch_vm split.
- `binding_owned_worker_without_its_binding_row_fails_construction` - pins `process-identity-incomplete` at identity construction.
- `guest_owner_uid_resolution_keeps_the_old_composer_precedence` - pins linked-uid precedence over the Guest plane, single consultation, no non-Guest consultation.
- `resolution_refusals_are_never_identity_ambiguity` - pins `map_provider_error` closed-kind table and terminal classification.
- `device_worker_vm_resolves_from_the_owning_devices_guest_owner` - pins Guest-owner VM scope + missing/unreadable refusal names.
- `device_worker_family_accepts_both_video_postures` - pins template→DeviceWorkerFamily table (both video postures, gpu, swtpm, none).
- `factory_registers_both_process_family_resource_types` - pins factory `resource_types()` list and `create` for both types (cross-check: tests/process_family.rs registration overlap).
- `family_descriptors_register_both_member_types` - pins exact BUILTIN+STARTUP / no-RUNTIME source membership, non-exportable, registered-type order, 2 decoders (cross-check: tests/process_family.rs registration coverage).
- `ephemeral_launch_uses_the_one_shot_effect_and_start_deadline` - pins one-shot launch through ephemeral ticket with startDeadline budget, adopt-on-next-pass, 5s resync, no relaunch.
- `ephemeral_exit_is_terminal_succeeded_and_the_ttl_retires_the_row` - pins one-shot exit → Succeeded + successfulTtl retention and manager row retirement.
- `ephemeral_runtime_deadline_stops_and_reports_a_terminal_failure` - pins bounded-runtime stop (fixed 30/30) → Failed + failedTtl projection.
- `ephemeral_incident_hold_keeps_a_failed_row` - pins incidentHold: no cleanup timer, never auto-retired.
- `ephemeral_launch_refusal_is_terminal_and_never_restarts` - pins one-shot launch refusal terminal, no restart requeue.
- `ephemeral_recover_adopts_a_live_process_without_launching` - pins one-shot recovery adopt + observe-without-relaunch.
- `ephemeral_quarantined_classification_never_launches` - pins one-shot reconcile on ambiguous evidence is terminal, never launches.
- `ephemeral_delete_stops_the_exact_identity_and_finalizes` - pins one-shot delete adopt→stop-ephemeral(fixed 30/30)→finalize order.
- `ephemeral_delete_converges_absent_and_stops_a_stale_candidate` - pins one-shot delete converges absent, stop-stale on stale candidate.
- `ephemeral_delete_refuses_an_ambiguous_identity` - pins one-shot delete refuses ambiguous, no destructive action.
- `ephemeral_unmintable_ticket_converges_on_delete_and_is_terminal_on_reconcile` - pins Guest-owned one-shot unmintable ticket: terminal reconcile, effect-free delete.
- `launch_reaches_ready_with_expected_ticket_inputs` - pins durable launch ticket inputs (ref, uid, generation, zone, zone_uid, policy_revision, provider, template, execution) and post-launch adopt → Ready.
- `never_adopt_row_observes_the_identity_it_launched_instead_of_stopping_it` - pins the never-adopt observe-don't-stop loop fix.
- `recover_adopts_a_live_matching_process_without_launching` - pins durable recovery adopt, no launch, Ready status.
- `recover_classifies_drifted_and_ambiguous_processes_as_quarantined` - pins recover stale/quarantined → Quarantined, no adopt.
- `finalize_finalizes_owned_children_before_the_process_teardown` - pins children-draining NotYet boundary and convergence.
- `delete_stops_term_then_kill_and_finalizes` - pins durable delete term(drainTimeout)/kill escalation, finalize, call order.
- `delete_without_a_live_process_is_a_noop` - pins durable delete absent converges without effects.
- `delete_stops_an_exact_stale_candidate_after_restart` - pins durable delete stop-stale on uniquely identified candidate.
- `delete_refuses_an_ambiguous_identity` - pins durable delete refuses ambiguous.
- `delete_stops_a_device_worker_even_when_the_launch_parameters_refuse` - pins delete never derives launch-only parameters.
- `retryable_reconcile_failure_requeues_exactly_once_with_backoff` - pins retryable launch failure → exactly one 1s requeue that fires and relaunches.
- `restart_budget_is_in_memory_only` - pins in-memory budget consumption, AwaitingRestart status, terminal when exhausted, no launch past budget.
- `durable_exit_is_observed_and_restarts_under_the_policy` - pins durable steady-state liveness probe cadence and exit-driven restart.
- `durable_exit_under_a_never_policy_is_terminal_and_never_relaunches` - pins never-policy exit terminal, no relaunch, wire-Failed.
- `durable_liveness_ambiguity_refuses_terminally_and_never_reads_ready` - pins liveness-Unknown terminal identity-ambiguous, never Ready.
- `stopped_lifecycle_row_stops_the_live_process_before_reading_succeeded` - pins stopped lifecycle: stop→finalize→probe, Succeeded only off observed stop.
- `stopped_lifecycle_row_still_live_defers_instead_of_claiming_satisfied` - pins still-live stop defers (Stopping + resync) until observed gone.
- `durable_unresolvable_launch_ticket_refuses_terminally` - pins the closed unresolvable-ticket spellings as terminal, no restart consumed.
- `validate_rejects_an_unsupported_provider` - pins validate refusal for an unsupported Provider.
- `validate_rejects_a_malformed_spec` - pins validate refusal for an undecodable spec.
- `has_active_answers_from_the_runtime_facet` - pins has-active happy path from the runtime facet.
- `has_active_refuses_an_invalid_payload` - pins the has-active invalid-payload closed-code table.
- `has_active_refuses_a_payload_naming_another_zone` - pins zone-mismatch refusal by name.
- `controller_provider_identity_binds_the_committed_provider_row` - pins committed Provider uid/generation binding into the controller context.
- `controller_provider_identity_stays_unbound_without_a_committed_row` - pins unbound slot (empty and unwired sources) stays closed.
- `guest_vmm_ticket_carries_the_old_descriptor_owner_identity` - pins guest VMM ticket owner_ref/owner_uid/target_ref from the pre-v3 plane.
- `guest_owned_row_binds_the_catalog_guest_descriptor_digest` - pins guest-owned row resolving its descriptor digest from the bundle.
- `non_guest_rows_keep_the_guest_descriptor_digest_unbound` - pins non-guest rows never consult the descriptor source.
- `missing_catalog_descriptor_keeps_the_guest_digest_unbound` - pins retained row without a digest stays unbound (refuses closed).
- `declared_target_and_guest_runtime_rows_resolve_through_the_one_resolver` - pins declared-target ownership rule + guest-runtime (VMM/qemu) target binding + incomplete binding worker error.
- `launch_identity_table_covers_owner_shapes` - pins the full owner-shape table (host/guest/binding/serving-probe/controller/incomplete).
- `launch_request_rejects_inherited_fd_count_mismatch` - pins fd-count-mismatch → InvalidTicket.
- `launch_request_debug_redacts_owned_descriptors` - pins ProcessLaunchRequest Debug redaction.
- `diagnostics_are_value_free` - pins Debug redaction of request/observation/launch diagnostics + error-code Display table.
