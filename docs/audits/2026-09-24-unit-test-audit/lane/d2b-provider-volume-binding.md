# d2b-provider-volume-binding - unit-test audit
tests: 18 · src files: 6
net: -2 tests, -24 lines

## Findings (biggest net first)
- duplicate: `factory_registers_only_the_binding_resource_type` (src/driver.rs:1440) - covered by `descriptor_declares_and_registers_the_binding_type` (tests/registration.rs:27). Both pin the `BindingDriverFactory` registering exactly one resource type, "VolumeBinding" (`resource_types().len() == 1`, `[0] == "VolumeBinding"`); the integration test pins strictly more (descriptor fields, registry registration, decoder lookup, factory create).
- duplicate: `child_cannot_silently_change_owner` (src/driver.rs:1952) - covered by `absent_parent_row_defers_retryably_while_owner_mismatch_stays_terminal` (src/driver.rs:1970). Both pin the same `parent_volume` OwnerMismatch guard failing `FailureClass::Terminal` when the present parent row's owner uid differs from the binding's; the keeper pins strictly more (absent parent row → retryable, unanswerable manager → retryable, owner mismatch → terminal). The deleted test's only extra is the `validate()` entry point, which is the same guard function (`decoded_binding` + `parent_volume`).
- gap: driver spec refusal - `SpecInvalid`/`ProviderUnsupported` terminal paths at `decoded_binding` (src/driver.rs:437-458) - no test feeds the driver an undecodable binding spec or a non-serving `providerRef`; the terminal-refusal surface is only pinned for view-not-found and owner-mismatch.
- gap: `ParentSpecInvalid` (src/driver.rs:540) - a present parent Volume row whose uid/spec does not decode (terminal) is untested; the parent-read matrix only pins absent (retryable) and owner-mismatch (terminal).
- gap: retryable effect/mutation failures - `ServingEffect` on socket-removal failure (src/driver.rs:1015-1021) and `ChildMutation` on manager ensure/delete failure (src/driver.rs:649, 664, 796, 1012, 1032) - the fakes never fail, so these error paths are unpinned.

## Keep
- `ensure_derives_worker_and_endpoint_children_persisted_before_spawn` - pins persist-before-spawn (F1) ordering, endpoint-after-worker dependency, ServingChildren plan fields, requeue cadence, convergence when the socket serves.
- `the_concluding_pass_publishes_the_fenced_status_projection` - pins the fenced projection naming the row's own uid/generation/revision; current for exactly that row, never for an unfenced revision.
- `stale_or_not_serving_fences_never_report_ready` - pins ready:false + `binding-not-ready` reason when the socket isn't serving; older-generation, ahead-revision, and foreign-uid fences never current.
- `worker_child_carries_the_signed_template_and_no_argv` - pins worker child spec: signed template, Host executionRef, processClass worker, no argv/command fields; ProcessSpec/EndpointSpec parse.
- `recover_rederives_the_launch_plan_matching_the_pre_restart_incarnation` - pins recover: re-derived plan equals pre-restart incarnation, RecoveredPlan status, Adopted vs Missing outcomes.
- `finalize_finalizes_owned_children_before_the_binding_teardown` - pins finalize: retryable while an owned child is live, child nudged through its delete pass, converges after retirement.
- `delete_drains_the_endpoint_before_the_worker_and_behind_the_mount_gate` - pins delete ordering: mount observed first, endpoint row, socket removal, worker last.
- `mounted_share_blocks_the_drain_before_any_child_is_removed` - pins a mounted share blocking delete with a retryable failure and zero teardown.
- `obsolete_owned_child_is_retired_endpoint_first` - pins reconcile retiring stale owned children endpoint-first/process-last.
- `dependency_watches_are_registered_once_per_target` - pins watch dedup across passes and the parent+children target set.
- `rejected_view_keeps_its_stable_reason_in_status` - pins terminal view rejection with the stable reason in status and no children minted.
- `absent_parent_row_defers_retryably_while_owner_mismatch_stays_terminal` - pins absent/unanswerable parent → retryable defer, present owner mismatch → terminal.
- `same_parent_and_binding_derive_the_same_child_keys` - pins deterministic child keys across passes (no churn).
- `the_service_delegates_onto_the_facets` - pins BindingEffectsService delegating socket_ready/remove_socket/guest_mount_ready onto the facets with the ordered call log.
- `binding_child_readiness_requires_current_fenced_projection` - pins `binding_readiness_current` fail-closed on unfenced/foreign-uid/stale-generation rows and current on a matching fence (consumer read path; overlapping currency matrix with the two driver fence tests, but each pins its own seam - publishing identity / not-ready reason here vs fail-closed parse there - so both stay).
- `parsed_binding_spec_strips_reserved_fields_and_rejects_broken_specs` - pins `parsed_binding_spec` stripping reserved envelope fields and returning None for broken specs.
