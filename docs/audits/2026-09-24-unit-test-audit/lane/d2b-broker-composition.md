# d2b-broker-composition — unit-test audit
tests: 31 · src files: 5
net: -3 tests, -36 lines

## Findings (biggest net first)
- duplicate: `an_unregistered_admitted_operation_fails_the_startup_invariant` (src/seam.rs:720) — covered by `the_startup_routing_invariant_holds_for_the_committed_catalog` (src/seam.rs:649). Both pin `verify_startup_routing(&[])` passes with nothing registered; the test's second assertion (fixture declaration registers) is covered by `injected_fixture_handler_answers_through_call` (src/seam.rs:413). Its own comment admits the admitted-without-handler leg is unreachable this pass, so every assertion it makes is pinned elsewhere.
- duplicate: `production_registration_accepts_only_the_catalog_row` (src/seam.rs:600) — covered by `registration_of_an_uncommitted_row_is_refused` (src/seam.rs:585). Both pin production registration refuses a non-committed row as `RoutingRefusal::Uncommitted`. The fixture operation `d2b.fixture.pure.echo` is not in the committed catalog (verified: absent from `generated/broker_operation_catalog.rs`), so `BrokerOperationRow::find` fails before the `ptr::eq` identity check the test's comment claims to pin — it exercises exactly the same branch as the covering test.
- duplicate: `the_pure_fixture_crate_passes_the_source_surface_probe` (src/dependency_surface.rs:426) — covered by `the_pure_fixture_crate_passes_the_full_audit` (src/dependency_surface.rs:457). Both pin the pure fixture crate's source surface is clean: `audit_crate` runs `probe_sources_in` and `SurfaceReport::is_clean()` requires `source_violations` empty, so the full-audit test subsumes the probe-only assertion.
- gap: `register_production_handlers` row-identity `ptr::eq` refusal branch (src/seam.rs:141-150) — no test reaches it: `find()` fails first for every non-catalog operation (the fixture operation is not committed), so the guarantee that a same-operation non-catalog row copy is refused is never exercised. The deleted `production_registration_accepts_only_the_catalog_row` claimed to pin it but could not reach it.
- gap: `verify_startup_routing` admitted-without-handler leg, the "committed operation(s) route to the in-broker leg with no registered handler" error (src/seam.rs:157-166) — never exercised: no committed row is admitted this pass and the fixture row is not committed, so the leg is unreachable by any test. The invariant's main failure mode (a wiring gap failing the broker closed at startup) is unpinned until a committed row is admitted.

## Keep
- `the_syscall_surface_fixture_crate_fails_the_source_surface_probe` (src/dependency_surface.rs:436) — pins the lexical probe fires the full violation set (raw-asm, used-static, link-section-entry-point, panic-hook-registration) on the hostile fixture crate.
- `the_pure_fixture_crate_passes_the_full_audit` (src/dependency_surface.rs:457) — pins the full audit (dep-tree delta + source surface) is clean for the pure fixture crate.
- `the_syscall_surface_fixture_crate_fails_the_full_audit` (src/dependency_surface.rs:466) — pins the full audit catches both halves: `libc` in forbidden deps and raw-asm in source violations.
- `an_unknown_crate_fails_the_audit_closed` (src/dependency_surface.rs:494) — pins the audit fails closed on a non-workspace crate name.
- `no_census_operation_maps_to_the_in_broker_leg_this_pass` (src/routing.rs:181) — pins the committed catalog admits zero operations to the in-broker leg (both via `catalog_admitted_operations` and a direct sweep).
- `a_family_owned_row_is_refused_and_routes_to_the_forward_carrier` (src/routing.rs:197) — pins a family-owned row with otherwise-admissible facets is refused by owner class alone.
- `the_wire_typed_broker_generic_rows_are_refused` (src/routing.rs:209) — pins every committed BrokerGeneric row is refused as NotProviderDeclared.
- `a_destructive_row_is_refused_as_effectful` (src/routing.rs:225) — pins the destructive facet → Effectful.
- `a_secret_exposing_row_is_refused_as_effectful` (src/routing.rs:235) — pins the secret-access facet → Effectful.
- `an_fd_minting_row_is_refused_as_effectful` (src/routing.rs:245) — pins the max_fds facet → Effectful.
- `a_state_cell_row_is_refused_this_pass` (src/routing.rs:255) — pins state-cell + durability → TouchesPrivilegedMachinery.
- `an_unowned_generic_row_is_refused` (src/routing.rs:268) — pins no declaring provider → NotProviderDeclared.
- `a_transport_excluded_row_is_refused` (src/routing.rs:278) — pins TransportExcluded owner → NotGeneric.
- `a_pure_generic_provider_row_is_admitted` (src/routing.rs:289) — pins the happy path: an otherwise-pure row is admitted InBroker.
- `the_committed_family_rows_are_all_refused` (src/routing.rs:294) — pins every committed family row (66) routes Forward(FamilyOwned).
- `injected_fixture_handler_answers_through_call` (src/seam.rs:413) — pins end-to-end injection: registration admits, envelope call answers, payload echoed, invocation id minted, audit-join identity carried.
- `a_fixture_handler_reaches_only_its_declared_context` (src/seam.rs:450) — pins the capability object carries exactly the declared context; a handler reaching beyond it refuses under its own code.
- `a_handler_touching_an_undeclared_state_cell_refuses_at_the_capability_object` (src/seam.rs:473) — pins AE3's runtime half: no cell handle is carried, the handler refuses with HANDLER_REFUSED + detail, never an envelope-side refusal.
- `an_effectful_handler_offered_to_the_in_broker_table_is_refused_by_the_routing_rule` (src/seam.rs:504) — pins the seam applies the routing rule at registration, refuses the whole registration fail-closed, and the Display message names the forward carrier.
- `a_family_owned_handler_is_refused_and_routes_forward` (src/seam.rs:527) — pins the committed pilot row (inspect-process-family) is refused Forwarded(FamilyOwned) at registration.
- `a_state_cell_operation_is_refused_this_pass` (src/seam.rs:554) — pins a state-cell declaration is refused Forwarded(TouchesPrivilegedMachinery) at registration.
- `registration_without_the_pure_certificate_is_refused` (src/seam.rs:574) — pins a mismatched pure certificate → ClaimMismatch.
- `registration_of_an_uncommitted_row_is_refused` (src/seam.rs:585) — pins production registration refuses an operation with no committed row as Uncommitted.
- `a_handler_crate_that_is_not_the_declaring_provider_is_refused` (src/seam.rs:611) — pins a foreign source crate → SourceCrateMismatch.
- `a_handler_crate_with_a_syscall_surface_is_refused_at_registration` (src/seam.rs:622) — pins the dependency-surface gate at registration: a syscall-surface crate is refused SurfaceViolation with the raw-asm violation named.
- `the_startup_routing_invariant_holds_for_the_committed_catalog` (src/seam.rs:649) — pins the invariant passes with nothing registered and fails for a handler registered for a refused operation.
- `the_startup_invariant_cross_checks_registered_handlers` (src/seam.rs:661) — pins the invariant fails for a refused operation and for a non-committed operation name.
- `two_admitted_fixture_handlers_both_answer` (src/seam.rs:676) — pins fail-closed registration admits every declared handler: two fixture declarations both answer with distinct invocation ids.

No `#[ignore]`d tests in this crate. No `tests/` integration dir (no cross-layer overlap to flag).