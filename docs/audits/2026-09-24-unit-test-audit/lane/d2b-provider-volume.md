# d2b-provider-volume - unit-test audit

tests: 14 · src files: 5

net: -2 tests, -23 lines

## Findings (biggest net first)

- duplicate: `factory_registers_only_the_volume_resource_type` (src/driver.rs:1012) - covered by `descriptor_declares_and_registers_the_volume_type` (tests/registration.rs:35 - its `lookup(...).resource_types()` asserts the same one-type list (len==1,[0]==VOLUME_TYPE_NAME, plus create()). The factory's resource-type surface is already pinned end-to-end through the registry. Behaviour both pin: the Volume driver factory exposes exactly one resource type, "Volume".
- trivial: `the_has_layout_wire_payloads_are_canonical` (.src/effects_service.rs:334) - asserts canonical_json_bytes roundtrip identity for two hardcoded literals (`{"hasLayout":true}`/`false`), referencing no product code (the literals are duplicated in the test, not read from `has_layout_response`). This pins the shared contracts serializer's plumbing, already roundtrip-pinned by d2b-contracts-resource's own spec tests (e.g. v3/volume.rs:1536 minimal_volume canon→parse roundtrip). Nothing of this crate's behavior is lost.
.
 gap: successful false report for the has-layout probe (has_layout_response(false) called via UIT serve_has_layout when the runtime facade reports an uninitialized layout, effects_service.rs:59) - the service contract promises `{"hasLayout":false}`, but every test that uses a false-reporting runtime refuses before reaching the response path; only the true literal is exercised through the service (and the deleted canonical-wire test pinned the false form only as serializer plumbing, not through service code).

## Keep
- `ensure_creates_binding_children_after_the_layout_effect` - pins fresh-host flow: recover→Missing, has-layout probe then exactly one ensure-layout, binding child ensured through ctx, F1 commit-before-spawn ordering, ServingChildren{desired:1,converged:true} status.
 remaining 11 tests pin the driver's other lifecycle states (adoption+re-validation+idempotent re-attach, degraded-layout retryable-failure one-effect-per-pass, deterministic child identity/no-churn, spec-grow shrink retire/retain, finalize F3 ordering, delete+idempotent retry, provider-mismatch terminal at validate, malformed-spec terminal at reconcile), and the effects service's uid-contract refusals (missing volumeUid→Declined "has-layout-volume-uid-missing", invalid uid→Declined "has-layout-volume-uid-invalid"),and the true-literal success report.


Nothing further to cut. Ship - checked all 14 `#[cfg(test)]` fns against same-crate integration coverage (tests/registration.rs) and the shared contracts serializers; only the factory-registration pin and the canonical-serializer plumbing echo were redundant.

route-out: `has_layout_response`'s "has-layout-response-invalid" declined branch is unreachable by construction (both literals parse cleanly) - dead defensive code, or a refactor toward response-from-value would make the canonical-wire test meaningful again.