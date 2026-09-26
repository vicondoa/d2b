# d2b-provider-guest-cloud-hypervisor - unit-test audit
tests: 16 · src files: 12
net: -1 tests, -23 lines

## Findings (biggest net first)
- trivial: `bound_evidence_exposes_only_exact_bounded_commitments` (src/health.rs:541) - constructs a `GuestSessionEvidence` from a fixed binding and echoes every value back through the accessors (`guest_uid`, `descriptor_digest`, `schema_digest`, provider=7, controller/session/reconnect/endpoint/seed=3, `seed_ready()`). This is a constructor-stores-its-arguments getter echo; the only extra it adds is the 9-slot argument mapping, which the constructors' typed parsing (every field parsed/validated in `binding()`) already proves. Nothing lost.
- cross-check: name-addressed/UID-free/redaction aspects of `guest_child_graph_is_deterministic_name_addressed_and_redacted` (src/bootstrap_graph.rs:297) are also pinned by `tests/guest_spec_validation_test.rs::fixed_guest_child_batch_is_name_addressed_and_uid_free`, `tests/redaction_test.rs::child_batch_debug_and_wire_bytes_do_not_include_descriptor_payload`, and `tests/controller.rs::one_uid_free_batch_contains_the_complete_guest_owned_child_graph`. Unit test additionally pins creation/deletion order determinism and `plan_children ≡ from_descriptor` - C2 to confirm overlap.
- gap: `GuestSessionEvidence::current` rejection branch - non-Guest resource ref, empty name, reconnect generation 0, or malformed boot-identity digest → `GuestSessionError::AuthenticationFailed` (src/health.rs:208-209). No test in src or tests/ drives this error path; every reachable constructor (current/current_bound/stale) funnels through it.

## Keep
- `guest_child_graph_is_deterministic_name_addressed_and_redacted` - pins creation/deletion order determinism, `plan_children` ≡ `from_descriptor`, UID-free 4-child device/process/endpoint batch with guest zone/owner, and Debug + canonical-bytes redaction of descriptor secrets.
- `vmm_lifecycle_stays_stopped_until_every_dependency_is_ready` - all five deps (device/network/volume/binding/setup) gate Running/Ready; each single missing dep keeps Stopped/Pending; all true → Running/Ready.
- `child_planning_rejects_invalid_resource_references_before_returning_a_graph` - non-Guest guest ref and non-Host execution ref both make `from_descriptor` return `Err`.
- `legacy_three_dependency_readiness_remains_a_strict_subset` - 3-arg `readiness()` wrapper forwards to 5-arg `vmm_readiness` with bindings+setup forced true (strict subset).
- `controller_sends_bootstrap_endpoint_before_establishing_resource_session` - `run_controller_session` sends the bootstrap marker+resource-endpoint fd with matching peer credentials before the ResourceV3 handshake succeeds and the controller stays alive.
- `controller_session_policy_is_resource_v3_and_inherited_socketpair_only` - pins the endpoint policy's ServicePackage=ResourceV3, TransportClass=InheritedSocketpair, Provider initiator role.
- `cloud_hypervisor_assignment_contract_is_fixed_to_host_target` - manifest-derived `ControllerRoleContract` is `FixedExecutionTarget` scoped to Guest only.
- `provider_manifest_is_the_packaged_canonical_contract` - parsed `provider-manifest.json` re-canonicalizes byte-identical to the packaged file (config-drift guard).
- `cloud_assignment_expectation_separates_primary_and_owner_child_verbs` - primary verbs exclude UpdateSpec/Delete; owner-child process verbs are exactly {Create, UpdateSpec, Delete}.
- `controller_receives_idempotent_assignment_over_authenticated_session` - duplicate assignment frames survive without terminating, stream-reset reopen keeps serving, revocation drives exit with `ControllerSessionError::Assignment`.
- `bound_evidence_requires_endpoint_and_seed_readiness_for_ready` - Ready only with controller+endpoint+seed; Degraded and not `ready_for` otherwise.
- `binding_rejects_zero_and_malformed_generations` - zero provider generation, malformed uid/descriptor/schema digests all → `Protocol`; controller-generation mismatch → `WrongIdentity`.
- `stale_or_mismatched_evidence_fails_closed_against_expected_binding` - stale generation → `Disconnected`; schema-digest and guest-uid mismatch → `WrongIdentity`; evidence fails closed.
- `capabilities_remain_bounded_and_legacy_constructors_stay_compatible` - >64 or >128-char capabilities → `Protocol`; legacy `current`/`stale` constructors keep Ready/Degraded.
- `debug_output_redacts_all_identity_payloads` - Debug of evidence omits guest uid, boot/descriptor/schema digests, `/run/d2b` paths, and "credential".
