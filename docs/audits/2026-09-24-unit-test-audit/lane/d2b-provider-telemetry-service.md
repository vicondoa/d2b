# d2b-provider-telemetry-service - unit-test audit
tests: 9 · src files: 2
net: -2 tests, -20 lines

## Findings (biggest net first)
- duplicate: `factory_registers_only_the_service_type` (src/driver.rs:698) - covered by `descriptor_declares_and_registers_the_service_type` (tests/registration.rs:21). Both pin the factory serving exactly the one `TELEMETRY_SERVICE_TYPE` (`resource_types()` == `[TELEMETRY_SERVICE_TYPE]`); the integration test also proves registration by the descriptor which registers and serves that factory
- duplicate: `validate_accepts_a_provider_declared_spec` (src/driver.rs:729) - covered by `the_declared_decoder_reads_a_stored_service_spec` (tests/registration.rs:69). Both decode the same provider-declared stored Service spec envelope successfully;the unit test's only extra assertion is driver-wiring plumbing (`driver.validate()` → `Ok`) over the same decoder
- gap: recover/reconcile on a corrupt stored spec return `InvalidResource` + Retryable (envelope error path, src/driver.rs:241) - only validate's malformed path is pinned (`validate_rejects_a_malformed_spec`);the op tagging for Recover/Reconcile and failure classification on those verbs is untested
- gap: reconcile propagates a manager row-read failure as `Reconcile`-class error ( src/driver.rs:304) -the `RecordingManager` fixture never returns an error, so this explicit error path has no test anywhere in the crate

## Keep
- `validate_rejects_a_malformed_spec` - pins malformed stored spec fails validate with op `Validate`, class `Retryable`
- `recover_adopts_a_service_without_a_target_realization` - pins recover returns `Adopted` and touches the manager not at all
- `service_pending_until_declared_endpoints_exist_then_fail_closed` - pins authority Service: no endpoints → `Pending`, empty `present_endpoints`, resync requeue once; seeded endpoint → present endpoint, projection `authority`/`Pending` (fail-closed readiness term), no reschedule while endpoint present
- `service_projection_role_reports_ready_without_ingest_evidence` - pins projection role → `Ready` projection `{serviceRole: projection, serviceReadiness: Ready}`, no manager effects, no requeue
- `service_reconcile_reports_degraded_for_an_unadmitted_role` - pins unadmitted role → `Degraded`, empty projection, `Satisfied`
- `dependency_watches_are_registered_once_per_target` - pins two reconciles register the ingest Endpoint watch exactly once per target (R12 dedup)
- `delete_runs_no_effect_past_the_manager_cascade` - pins delete adds no manager log entries after a reconcile (Service owns no children, realizes nothing to teardown)
