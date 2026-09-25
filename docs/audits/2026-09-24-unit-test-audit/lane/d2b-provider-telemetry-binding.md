# d2b-provider-telemetry-binding — unit-test audit
tests: 10 · src files: 2
net: -1 tests, -12 lines

## Findings (biggest net first)
- duplicate: `factory_registers_only_the_binding_type` (src/driver.rs:950) — covered by `descriptor_declares_and_registers_the_binding_type` (tests/registration.rs:24). Both pin that the binding factory serves exactly one type, `TELEMETRY_BINDING_TYPE` (`resource_types()` len 1 / value equal); the integration test pins strictly more (registry registration, decoder presence, `create`, allowed-sources mask, creation order).
- gap: manager route failure → `Reconcile`-class error (src/driver.rs `derived_binding_children`/`reconcile_binding`/`recover` `map_err` sites) — the recording manager endpoint never fails, so no test anywhere in the crate exercises `TelemetryBindingDriverErrorKind::Reconcile`; only the `Validate` class (malformed spec) is pinned.

## Keep
- `validate_rejects_a_malformed_spec` — pins malformed spec bytes → error with `DriverOp::Validate` and `FailureClass::Retryable` classification.
- `validate_accepts_a_provider_declared_spec` — pins a provider-declared spec passes validate.
- `binding_reconcile_ensures_provider_declared_children_then_converges` — pins Process-then-Endpoint ensure order, exact child specs (template/providerRef/executionRef/processClass/domain; producerRef/purpose/lifecyclePolicy), Pending→Degraded phase transition, resync scheduling then stop, no-op second pass.
- `binding_reconcile_fences_a_dangling_service_dependency` — pins fence on absent Service/target rows: no mutation, Degraded, resync re-evaluates.
- `binding_reconcile_fences_a_foreign_provider` — pins fence on non-serving `providerRef`: never derives children.
- `binding_reconcile_retires_obsolete_children_endpoint_first` — pins endpoint-first / process-last teardown order of owned children the desired set no longer derives.
- `dependency_watches_are_registered_once_per_target` — pins watch dedup per target incl. `TelemetryService/ingest` across two reconcile passes.
- `binding_recover_adopts_only_a_current_owned_child_set` — pins `Missing` → `Adopted` transition once durable child rows are current (R15).
- `delete_runs_no_effect_past_the_manager_cascade` — pins the delete pass issues no manager effects (cascade already retired children).