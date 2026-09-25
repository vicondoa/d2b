# d2b-provider-wayland-policy — unit-test audit
tests: 9 · src files: 8
net: -1 tests, -11 lines

## Findings (biggest net first)
- trivial: `projection_status_never_claims_host_readiness` (src/audio_registry.rs:677) — asserts the private `unavailable_status` builder stores its three arguments into the struct fields (constructor-stores-args echo). Nothing lost: the same builder's observable JSON output is pinned by `audio_status_projection_is_stable_and_separates_readiness` (src/audio_registry.rs:689, host=Ready branch) and `audio_resource_projection_matches_the_frozen_status_schema` (src/audio_registry.rs:707, host=Unavailable branch); the builder stays exercised transitively.

## Keep
- `audio_lease_identity_is_stable_and_nonzero` — pins `lease_for`: deterministic per resource, distinct across resources (lease uniqueness invariant).
- `audio_status_projection_is_stable_and_separates_readiness` — pins the `audio_binding_status_value` wire shape: phase/readiness strings, null microphone, off grants, inactive arbitration, None posture, OfflineOnly last-set.
- `audio_resource_projection_matches_the_frozen_status_schema` — pins `audio_binding_projection` against the frozen `SemanticFamily::Audio` binding-status schema (validate_names) plus observedServiceRef/realizationRefs/channel grants.
- `relationship_validation_rejects_missing_guest_and_cross_service` — pins the test-local relationship validator: missing guest refused, present guest accepted, deleting binding exempt (mirrors `reconcile_binding_resource`'s relationship checks; note the deleting-binding exemption diverges from the product path, which refuses deleting bindings — see gap 1).
- `audio_decoder_reads_reserved_provider_ref_from_resource_spec` — pins spec decode re-inserting `providerRef` from the envelope (via test-local `decode_services` mirror).
- `audio_decoder_ignores_a_foreign_provider_resource` — pins foreign-provider rows being skipped by the audio-resource filter (via test-local `decode_services` mirror).
- `absent_audio_dependency_defers_until_the_row_appears` — pins the real `audio_dependency_row`: absent row defers (Ok(None), never terminal), committed matching row progresses, foreign name/zone/type stays terminal InvalidResource. Regression for the P2 defer-vs-refuse bug.
- `view_phase_delegates_to_the_canonical_wire_phase` — pins `view_phase` equal to `ResourceView::wire_status()["phase"]` across the full status vocabulary × deleting × status-generation matrix (issue #515).

## Gaps
- gap: `AudioResourceRuntime` reconcile/finalize paths (`reconcile_service_resource`, `reconcile_binding_resource`, `finalize_binding_resource` in src/audio_registry.rs) — the crate's biggest product module is untested: real relationship validation, deletion refusal, controller-error mapping, and service-deletion-with-dangling-binding refusal have no test in src or tests/ (engine.rs drives the shared verbs; registration.rs pins the descriptor only).
- gap: product `is_audio_resource`/`decode_spec` (src/audio_registry.rs) — only test-local mirrors (`decode_services`, `validate_relationships`) are exercised, and the mirror already drifts from product on deleting-bindings; the real functions have no direct test.
- gap: hosted `audio-binding-statuses` method (`serve_audio_binding_statuses`/`binding_status_value` in src/effects_service.rs) — the family's declared service surface is untested anywhere in the crate.