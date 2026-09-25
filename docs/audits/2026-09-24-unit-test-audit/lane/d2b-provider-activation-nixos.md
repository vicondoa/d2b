# d2b-provider-activation-nixos — unit-test audit
tests: 23 · src files: 7
net: -2 tests, -54 lines

## Findings (biggest net first)
- duplicate: `offline_verification_refuses_before_the_handoff_is_dispatched` (src/driver.rs:1501) — covered by `a_production_factory_created_driver_refuses_before_dispatch` (src/driver.rs:1390). Both pin: with a fail-closed verifier and the prior generation row present, a Host-target reconcile projects HelperRefused and dispatches nothing; the covering test additionally pins that the production factory binds that verifier (the same `FailClosedActivationVerifier` type, same seam, same row shape). ~42 lines.
- duplicate: `factory_registers_only_the_generation_resource_type` (src/driver.rs:1216) — covered by `descriptor_declares_and_registers_the_generation_type` (tests/registration.rs:26). Both pin: the activation factory serves exactly one resource type, `ACTIVATION_TYPE_NAME` (`resource_types().len() == 1`, `[0] == ACTIVATION_TYPE_NAME`); the integration test asserts it on the descriptor-registered factory, which is built from the same `ActivationDriverFactory::new`. ~12 lines.
- gap: `recover`'s Adopted branch (src/driver.rs, `recover()`) — a Guest target with an existing owned runner must return `RecoveryOutcome::Adopted` and project `staged`; the test `recover_adopts_an_existing_runner_and_reports_missing_without_one` (src/driver.rs:1644) promises it in its name but only exercises the Missing branch (no runner, and Host target). No test anywhere in the crate pins the Adopted path.
- gap: StaleGeneration outcome (src/driver.rs, `host_handoff_outcome` Completed-with-equal-generations branch and `execute_host_runner`'s `source_generation == 0 || observed.ordinal() <= source_generation` guard) — no test pins the stale-generation projection; unit tests only script `Completed { 1, 2 }` → Succeeded.
- gap: `activation_detail` Superseded (Ready phase + non-success outcome) and Adopted branches (src/driver.rs) — `boot_success_projects_the_default_and_switch_success_projects_applied` (src/driver.rs:1784) covers 4 of the 7 branches only.

## Keep
- `validate_rejects_a_spec_outside_the_closed_generation_contract` — valid contract passes, malformed spec (foreign providerRef) fails the gate.
- `host_target_dispatches_the_preserved_handoff_intent_and_projects_success` — Host reconcile dispatches one handoff with preserved intent (source/target generation, mode, artifact, compatibility floor), projects Ready/Applied/Succeeded, mints no runner.
- `the_factory_wires_the_facets_into_the_created_driver_end_to_end` — factory-created driver dispatches through the facets' scripted broker and reduces the response end to end.
- `a_production_factory_created_driver_refuses_before_dispatch` — production factory binds the fail-closed verifier; refuses before any dispatch, projects HelperRefused.
- `host_refusal_projects_helper_refused_without_minting_a_runner` — Refused handoff projects Failed/Planning/HelperRefused, no runner child.
- `host_target_without_a_prior_row_fails_closed_before_dispatch` — missing or cross-execution prior row errors before dispatch.
- `guest_target_mints_the_runner_as_an_owned_process_resource` — Guest reconcile mints the EphemeralProcess runner via the manager: persist-before-spawn (F1), owner_uid, argv-free typed spec, Staged projection, no host handoff.
- `guest_rejoin_waits_on_the_settle_edge_without_duplicating_the_runner` — rejoin keeps one ensure + one settle watch, projects Applying.
- `recover_adopts_an_existing_runner_and_reports_missing_without_one` — recover reports Missing for runner-less Guest and Host targets (see gap for the untested Adopted half).
- `finalize_finalizes_the_owned_runner_before_the_generation_teardown` — live owned runner → Retryable failure + delete nudge; converges once the runner retires.
- `delete_retires_only_the_owned_runner_child_and_is_idempotent` — delete retires exactly the owned runner child, once per pass, idempotent under retry.
- `generation_ordinals_are_taken_from_bounded_names` — ordinal_from_name boundaries (suffixed names, gen-0 and bare "gen" rejected).
- `boot_success_projects_the_default_and_switch_success_projects_applied` — activation_detail projections for Boot/Switch success, RolledBack and HelperFailed.
- `a_completed_handoff_reduces_to_the_closed_completed_result` — Completed state reduces to Completed with the coordinator's generations.
- `a_refused_handoff_projects_helper_refused_without_a_completion` — Refused state reduces to Refused.
- `a_rolled_back_handoff_projects_rolled_back` — RolledBack state reduces to RolledBack.
- `a_non_terminal_or_failed_dispatch_projects_incomplete` — Recorded state and dispatch error both reduce to Incomplete.
- `the_dispatched_request_carries_the_lifecycle_caller_role` — dispatched request carries HandoffCallerRole::Lifecycle, target and intent verbatim.
- `the_hosted_method_answers_the_family_committed_surface` — hosted inspect-activation serves the committed family payload through the factory.
- `declared_steps_carry_the_wire_labels_and_generation_suffixes` — the 3 declared runner steps with wire labels and generation suffixes; mode→step mapping for Switch/Boot/Test.
- `adopt_is_outside_the_declared_runner_step_set` — Adopt mode maps to None; is_declared_runner_step rejects adopt/rollback, accepts switch/boot/test.