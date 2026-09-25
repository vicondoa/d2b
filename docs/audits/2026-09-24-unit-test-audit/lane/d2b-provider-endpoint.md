# d2b-provider-endpoint — unit-test audit
tests: 16 · src files: 6
net: -0 tests, -0 lines

## Findings (biggest net first)
No duplicate or trivial tests: all 16 unit tests pin distinct behavior (see Keep). Three genuine product error paths have no test anywhere in the crate:
- gap: socket-effect failure → Retryable `ENDPOINT_SOCKET_EFFECT_FAILED` (driver.rs delete `remove_socket` error map; reconcile spawned-effect `Failed` branch) — `FakeSocketEffects` never fails, so neither the delete retryable classification nor the long-effect failure result is exercised.
- gap: evidence realize-budget timeout (effects_service.rs `ensure_socket`, `SOCKET_REALIZE_BUDGET` 5 s deadline → retryable "not realized within its realize budget") — no test scripts never-present evidence.
- gap: delete on an undecodable stored spec converges with `Ok` and no effects (driver.rs `let Ok(spec) = self.decoded_spec(...) else { return Ok(()) }`) — no test.

cross-check: d2bd `resource_runtime/plane_controller_bridge.rs:961-1047` asserts the same committed ch-api/guest-control → `GuestControl` realization admission as `provider_committed_control_shapes_are_admitted` / `guest_control_shapes_realize_through_the_endpoint_verbs`. C2 to resolve.
cross-check: d2bd `resource_plane_v3.rs:5053` asserts the hosted `inspect-endpoint` payload carries the swtpm purposes/realizations — overlaps the derivations pinned by `the_derived_families_follow_the_declaring_providers`. C2 to resolve.

## Keep
- `reconcile_realizes_the_socket_through_a_long_effect` (driver.rs:806) — virtiofsd validate → recover Missing → two-pass reconcile: `ensure-socket` spawns as a long effect (InProgress, mailbox never blocks), second pass Satisfied + status Realized.
- `recover_adopts_a_realized_socket` (driver.rs:862) — socket present at recover → Adopted.
- `finalize_finalizes_owned_children_before_the_socket_teardown` (driver.rs:877) — live owned child → Retryable drain-pending + child nudge with no socket effect; converged once child retires, finalize runs no socket teardown.
- `delete_removes_the_socket_before_any_worker_teardown` (driver.rs:914) — delete removes the socket first (endpoint-first ordering befores the worker Process child), retry idempotent.
- `non_virtiofsd_shapes_are_rejected_at_validate` (driver.rs:938) — virtiofsd purpose on a non-unix transport → Terminal shape-unsupported at validate.
- `malformed_spec_decodes_to_a_terminal_failure` (driver.rs:963) — non-Endpoint spec bytes → Terminal spec-invalid at reconcile.
- `provider_committed_control_shapes_are_admitted` (driver.rs:1025) — `endpoint_realization` admits exactly the committed ch-api/guest-control shapes as `GuestControl` (exact-variant assertion the driver-path tests do not pin).
- `provider_committed_device_worker_shapes_are_admitted` (driver.rs:1084) — `endpoint_realization` admits the two TPM worker purposes as `DeviceWorkerSocket`.
- `device_worker_look_alikes_stay_refused` (driver.rs:1103) — 4 device-worker look-alikes (wrong class, Guest producer, cross-domain locality, provider visibility) → Terminal at validate.
- `look_alike_control_endpoints_stay_refused` (driver.rs:1151) — 6 control look-alikes (undeclared purpose, non-carriage transport, purpose/producer/locality cross pairings) → Terminal at validate.
- `guest_control_shapes_realize_through_the_endpoint_verbs` (driver.rs:1225) — ch-api/guest-control full verb cycle: validate admits, recover Missing, reconcile realizes through the evidence facet (never `ensure-socket`), Satisfied, delete converges.
- `the_derived_families_follow_the_declaring_providers` (effects_service.rs:286) — purpose derivations equal the declaring providers' vocabularies (ChildRole purposes, TPM constants), incl. negative cases.
- `the_service_dispatches_onto_the_facets_by_purpose` (effects_service.rs:324) — facet dispatch: evidence purposes never reach the socket facet (no calls recorded), virtiofsd reaches it (`socket-present`).
- `minimal_endpoint_vector_is_strict_and_canonical` (endpoint.rs:479) — canonical JSON bytes byte-exact + strict roundtrip decode equals.
- `visibility_aliases_and_scalar_consumer_policy_are_rejected` (endpoint.rs:493) — closed visibility enum (no `private`/`provider-internal`/`authorized-consumers` aliases); consumerPolicy must be an object, never scalar/array.
- `producer_and_provider_references_are_type_checked` (endpoint.rs:509) — providerRef must name a Provider type; producerRef type-checked against the admitted producer set.