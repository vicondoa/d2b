# d2b-provider-guest — unit-test audit
tests: 37 · src files: 9
net: -1 tests, -25 lines

## Findings (biggest net first)
- duplicate: `reconcile_refuses_a_qemu_guest_without_its_provider_row` (src/driver.rs:2176) — covered by `classified_provider_row_read_defers_absence_and_requires_terminal_evidence` (src/driver.rs:2204). Both pin: an absent qemu Provider row makes reconcile defer retryable with the closed `guest-provider-unavailable` code and never reach the effect. The keeper's first segment runs the identical setup (empty manager, qemu guest row) and asserts strictly more: absent defers, unanswerable manager defers, undecodable committed row is terminal `guest-spec-invalid`. The only unshared assert (manager `ensure_order()` empty) is the same refusal-before-side-effects property on the manager port.

## Keep
- `factory_registers_the_guest_type` — factory exposes exactly the one Guest type.
- `driver_recreation_shares_the_factorys_controller_state` — recreated driver from same factory resumes controller state (Ready, not Pending restart); U10 lifetime pin.
- `registrations_are_closed_and_provider_scoped` — GUEST_REGISTRATIONS closed at 4, one kind each, provider-scoped; `from_type_and_provider` refuses wrong type / no provider / unknown provider.
- `validate_admits_every_registered_provider` — every registered provider passes validate.
- `validate_refuses_an_unregistered_provider_as_terminal` — unregistered provider is terminal `guest-spec-invalid`.
- `reconcile_ensures_the_qemu_child_graph_and_publishes_the_status` — qemu child graph (Volume then Process), effect call with provider spec + children, Ready status with projection published and taken.
- `reconcile_pends_the_guest_until_its_children_converge` — Provider Ready ahead of children keeps Guest Pending.
- `classified_provider_row_read_defers_absence_and_requires_terminal_evidence` — absent/unanswerable provider row defers retryable; undecodable committed row is terminal (issue #511).
- `reconcile_watches_declared_dependencies_once` — dependency + desired-child watches registered exactly once across passes (R12).
- `cloud_hypervisor_guests_commit_no_children_through_the_driver` — CH kind commits no children via driver, reports Ready.
- `recover_adopts_a_cloud_hypervisor_guest_with_a_live_vmm_child` — CH recover Missing → Adopted on committed VMM child.
- `recover_adopts_a_qemu_guest_with_its_complete_child_set` — qemu recover Missing → Adopted on full child set.
- `finalize_blocks_while_an_owned_child_is_live` — finalize retryable `guest-finalize-pending` while child live, completes after child dropped.
- `delete_runs_the_provider_stage_before_retiring_the_children` — R10 order: provider finalize precedes child retirement.
- `delete_refuses_to_retire_children_while_the_provider_stage_pends` — pending provider stage blocks retirement, no deletes issued.
- `status_projection_is_sticky_across_passes` — projection published each pass and sticky when a later pass carries none.
- `view_phase_delegates_to_the_canonical_wire_phase` — `view_phase` equals `ResourceView::wire_status` phase across the closed status vocabulary × deleting × generation grid (issue #515).
- `qemu_controller_contract_invokes_controller_and_finalizes` — qemu framework controller reaches PausedAtBoot and converges finalizer.
- `aca_controller_contract_invokes_controller_and_finalizes` — ACA framework ports drive Progressing → Converged → Ready and converge finalizer.
- `azure_vm_controller_contract_invokes_controller_and_finalizes` — AzureVM framework drives Ready and converges finalizer through bounded poll loop.
- `guest_phase_refuses_a_payload_that_names_another_zone` — wrong zone refused `guest-phase-zone-mismatch`.
- `guest_phase_refuses_a_payload_without_zone` — missing zone refused `guest-phase-zone-missing`.
- `guest_phase_refuses_a_payload_that_asserts_a_zone_uid` — asserted zoneUid refused `guest-phase-zone-uid-unsupplied`.
- `guest_phase_refuses_a_payload_without_a_resource_ref` — missing resourceRef refused `guest-phase-resource-ref-missing`.
- `guest_phase_refuses_an_invalid_resource_ref` — unparseable ref refused `guest-phase-resource-ref-invalid`.
- `guest_phase_refuses_a_non_guest_resource` — non-Guest type refused `guest-phase-not-a-guest`.
- `guest_phase_refuses_when_the_manager_cannot_answer` — unanswerable manager refused `guest-phase-manager-unavailable`.
- `guest_phase_answers_the_live_phase_of_a_held_row` — held row answers its live phase in the canonical report shape.
- `cloud_hypervisor_reconcile_drives_the_controller_session_facets` — CH reconcile rides the controller-session facets with the full fence read order (row → committed → session-generation → row → ensure-session → reconcile-ch).
- `cloud_hypervisor_finalize_completes_through_the_controller_session` — CH finalize completes via the controller-session reconcile entry alone.
- `schema_vector_pins_the_minimal_guest_base_spec` — canonical JSON vector of the minimal spec, roundtrip-stable.
- `guest_and_host_share_one_execution_policy_definition` — Guest reuses Host execution policy; missing defaultUserRef refused.
- `system_artifact_id_is_a_bounded_token` — systemArtifactId is a bounded token; path-like values refused.
- `base_object_never_carries_a_universal_or_layer_three_field` — base object excludes providerRef/updatePolicy/provider; unknown fields refused.
- `diagnostics_stay_redacted` — Debug output redacted.
- `cloud_hypervisor_provider_fails_closed_without_api_socket` — no API socket: shutdown Unavailable, poll Unknown, vmm exit NotSupported.

## Gaps
- gap: target-control service refusals — foreign-zone source, unregistered resource type, and specDigest mismatch all refuse `SessionUnavailable` with no state (src/target_service.rs, the U13 guest half) — the whole module has no test in this crate (src or tests/); the fail-closed rules are the module's documented core.
- gap: `CloudHypervisorShutdown::poll_state` state classification — Created/Shutdown → GuestStopped, Running/Paused → Running, unknown/error → Unknown (src/shutdown.rs) — only the no-socket path is tested; the wire-state mapping is unpinned.
- gap: ACA kind child graph through the driver — `aca_child_ensures` commits the sandbox-agent Endpoint (src/driver.rs) but no unit test asserts the ACA reconcile ensure order (qemu graph is pinned; ACA only appears with scripted effects).