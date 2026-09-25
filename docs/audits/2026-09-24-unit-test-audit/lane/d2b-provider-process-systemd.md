# d2b-provider-process-systemd — unit-test audit
tests: 12 · src files:  ̄11
net: -2 tests, -66 lines

## Findings (biggest net first)
- duplicate: `a_guest_execution_binding_with_a_wrong_digest_refuses_through_the_hosted_service` (src/effects_service.rs:351) — covered by `a_guest_execution_binding_with_a_wrong_boot_identity_digest_is_refused` (src/operations.rs:1368). Both pin: a guest execution binding with a zero boot-identity digest refuses with `UNIT_INVALID_REQUEST`. The direct `validate_request` test also pins the zero-digest refusal, wrong-digest refusal, and matching kernel boot-id digest admission — strictly more behavior. The only extra the hosted test carries, that the refusal survives the service dispatch path, is pinned by `the_operation_methods_resolve_to_the_committed_handler_table` (src/effects_service.rs:289), which loops every non-inspect method (incl. `start-systemd-unit`) asserting the committed handler's closed refusal, never a synthesized result. Nothing unique is lost.
- trivial: `the_factory_builds_a_new_service_value` (src/effects_service.rs:336) — asserts two `Factory::build()` calls return non-`Arc::ptr_eq` instances. `build()` is `Arc::new(SystemdEffectsService::new())`, no caching, so a fresh allocation is guaranteed by construction — an allocation identity detail, not an observable contract. Nothing lost.

## Keep
- `the_declared_service_answers_its_operation_inventory` — inspect endpoint returns `operations` array of exactly 5 + `family: process-systemd` fest.
- `the_operation_methods_resolve_to_the_committed_handler_table` — every declared non-inspect method dispatches to the committed handler and refuses closed(never synthesized, with `kernel-seam-unwired`/`unit-invalid-request`..
- `unit_names_are_deterministic_and_path_safe` — `unit_name` deterministic, prefix/suffix pin (`d2b-process-` / `.service`), charset restricted to ASCII alnum + `-`/`.`..
- `generic_resource_identity_changes_unit_name` — resource ref and resource uid each influence the unit name.

- `cgroup_identity_rejects_foreign_unit_leaves` — System domain: matching leaf accepted, foreign leaf rejected `UNIT_IDENTITY_MISMATCH`.
- `cgroup_identity_binds_user_manager_and_slice` — User domain: `user-<uid>.slice` template accepted, wrong-uid slice rejected — identity is bound tothe manager uid..
- `a_guest_execution_binding_with_a_wrong_boot_identity_digest_is_refused` — zero/wrong digest refused `UNIT_INVALID_REQUEST`; digest of `/proc/sys/kernel/random/boot_id` (domain-tagged, admitted — gate compares, never a constant..
- `a_guest_targeting_request_without_a_binding_is_refused` — Guest execution target mandates a guest binding (never optional..
- `the_family_registers_exactly_the_five_committed_operations` — family registry pins exactly the five committed operation refs in order（start/check/observe/open/stop）..
- `identity_reads_select_the_service_interface_for_service_owned_properties` — `ControlGroup`/`MainPID` map to `Service` interface, `ActiveState`/`InvocationID` stay on `Unit` interface (issue #587 hermetic pin..