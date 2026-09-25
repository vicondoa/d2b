# d2b-provider-user - unit-test audit
tests: 21 · src files: 6
net: -1 tests, -15 lines

## Findings (biggest net first)
- duplicate: `factory_registers_exactly_the_user_resource_type` (src/driver.rs:667) - covered by `descriptor_declares_and_registers_the_user_type` (tests/registration.rs:25). Both assert the User factory's `resource_types()` is exactly `["User"]` and `create()` on a `User/alice` key succeeds; the integration keeper pins those same factory claims via the descriptor + registry and strictly more (descriptor flags, registry lookup, the declared decoder).
- gap: production NSS probe `UserProbe` (src/probe.rs:20, `discover_local_user`) is never exercised by any test in the crate - every unit and integration test scripts `ScriptedProbe`, so the real NSS error mapping (`getpwnam`/`getgrnam` failure → `SystemCoreError::DiscoveryUnavailable`), the account-absent `Ok(None)` boundary, and the digest/binding derivation are all unpinned. One real-machine boundary with zero coverage.

## Keep
- `the_registry_serves_the_declared_factory_for_a_user_row` - a registry-served driver reconciles a stored User row to Satisfied with Ready/Discovered status; the only reconcile-through-registry test.
- `validate_accepts_the_bootstrap_user_row` / `validate_rejects_a_malformed_user_spec` - typed User spec decode: bootstrap envelope validates, non-User JSON fails Terminal (system-core-spec-invalid).
- `recover_adopts_without_touching_the_target` - recover converges with no effects and no status (old observe was converged).
- `reconcile_discovers_once_per_desired_generation` - exactly one observe per generation; a realized current-gen status short-circuits the next pass; no requeue, no manager mutation.
- `reconcile_publishes_the_user_discovery_projection` - Pending/Degraded/Unknown publish the honest phase + Discovered and schedule exactly one re-check at `USER_REDISCOVER` cadence.
- `reconcile_rediscovers_a_cached_unrealized_phase` - a current-gen but unrealized cached status is re-observed; only a realized status short-circuits (distinct from the two tests above).
- `reconcile_maps_a_discovery_failure_to_a_retryable_failure` - discovery failure surfaces Retryable (system-core-user-discovery-failed) with no status published.
- `finalize_finalizes_owned_children_before_the_delete_noop` - a live owned child requeues the pass (Retryable) and nudges the child delete; converges once the child row retires (F3).
- `delete_converges_without_effects_or_child_mutation` - delete is a no-op on effects and manager.
- `driver_operations_stay_off_every_spawn_surface` - all four ops in sequence leave manager/requeue untouched (KTD13 no-spawn invariant; validate/recover manager-emptiness only pinned here).
- 3 `observe_user_*` seam tests (discovered Pending/Absent/error-parity) and 4 `inspect_user_*` hosted tests (bounded discoverable answer, absent, discovery-failure refusal, 7 malformed-request shapes) pin the U5 parity contract across the two public surfaces; `inspect_user_answers_the_declared_groups_observation` (declared groups feed the probe → Drifted/Degraded with group-carrying digest) and `the_factory_builds_the_service_over_the_facets` (composition-root factory serves the same observation) pin the remaining distinct claims.
