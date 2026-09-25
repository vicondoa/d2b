# d2b-provider-provider - unit-test audit
tests: 14 · src files: 4
net: -0 tests, -0 lines

## Findings (biggest net first)
Nothing to cut. Ship.

14 tests audited across the driver lifecycle (validate/recover/delete/reconcile/finalize) and the pure planner (plan_external/plan_observed/plan_system_core); none duplicates or trivial - each pins a distinct transition, error class, or boundary.

- gap: system-minijail fixed-host readiness gate (`fixed_provider_host_ready`, src/providers.rs:362) - no test anywhere in crate. Only system-core's Zone projection is covered (driver.rs `system_core_provider_phase_follows_the_zone_projection`); the minijail Provider's Host-row gate (providerRef + Ready + observedGeneration) could regress unnoticed.
 cross-check: `d2bd` plane tests register the same descriptor with `FailClosedProviderDriverEffects`, so Provider reconcile may be re-pinned end-to-end there - resolve in C2.

- gap: `provider_observation` canonical-JSON decode failure → `CoreReconcileError` (src/providers.rs:406,415,466) - no test.; the pure observation policy is the shared home consumed by d2bd's G5 bridge, and its error path is unpinned anywhere in crate.

## Keep
- `validate_refuses_a_spec_that_is_not_an_object` - validate() refuses non-object spec terminal `core-spec-invalid`, op=Validate, no manager touch.
 
- `recover_adopts_without_effects` - recover() returns Adopted and touches nothing.
 
- `delete_converges_without_effects` - delete() converges with no manager calls.
 

- `provider_reconcile_publishes_the_observed_status` - full happy path: session evidence → Satisfied, Ready status, all observation fields true, volume_refs, exact call order, session read once. (Richest keeper.) 
 
- `provider_reconcile_pends_without_session_evidence` - absent session → Pending; controller component not ready, but Process row itself Ready (dependencies stay ready, conformance fails).
 

- `provider_reconcile_fails_a_declared_volume_that_disappeared` - declared Volume drifts away → Pending, `required_dependencies_ready` false (volume-refs fence).
 

- `provider_reconcile_reports_manager_failures_as_retryable` - manager read failure → Retryable, op=Reconcile.

 
- `system_core_provider_phase_follows_the_zone_projection` - system-core skips children; Zone mandatory-handler projection drives phase (absent → Pending, and the fixed dependency is read from manager)..
 

- `finalize_converges_once_the_owned_children_are_gone` - finalize() errors while children live, succeeds after both dropped (full drain pass plus gate).
 

- `provider_drain_gate_tracks_the_controller_process_child` - finalize_pass: controller Process blocks, Volume child does not (per-type selectivity the full-finalize test cannot distinguish).
 

- `ready_external_provider_publishes_only_after_children_are_ready` - plan_external with all-ready observation → Ready, exports published, EnsureComponent(Service( action planned..
 

- `missing_dependency_keeps_exports_withdrawn` - `required_dependencies_ready=false` → Pending, exports withdrawn (complementary transition)..
 

- `untrusted_provider_is_rejected_before_child_planning` - untrusted manifest → `TrustOrCompatibilityDenied` before any observation-driven planning...
 

- `fixed_system_core_never_plans_a_process_child` - plan_system_core(true) → Ready, zero actions (bootstrap exception has no children)..
