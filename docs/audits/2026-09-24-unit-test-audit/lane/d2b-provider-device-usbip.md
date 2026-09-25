# d2b-provider-device-usbip - unit-test audit
tests: 20 · src files: 18
net: -3 tests, -60 lines

## Findings (biggest net first)
- duplicate: `explicit_plan_preserves_step_stop_and_execution_order` (src/state_machine.rs:803) - covered by `explicit_plan_does_not_require_bundle_intents` (src/state_machine.rs:719) for the canonical-steps assertion, `stop_order_preserves_per_env_sidecars` (src/state_machine.rs:572) for stop-order, and `execute_happy_path_calls_every_step_in_order` (src/state_machine.rs:675) for happy-path execution. All three asserts are already pinned on identically-shaped plans; `execute_usbip_plan`/`stop_order` never branch on `claim_source`, so an explicit plan flows through exactly the code the other tests already pin.
- duplicate: `bind_failure_rollback_preserves_started_backend` (src/state_machine.rs:601) - covered by `proxy_failure_rollback_preserves_per_env_sidecars` (src/state_machine.rs:631) and `each_step_failure_surfaces_typed_error` (src/state_machine.rs:690). Rollback-order filtering (per-env sidecars excluded, per-busid steps undone) is pinned strictly more strongly by the proxy-failure test, whose rollback list adds Bind-included and Proxy-excluded; the Bind-failure `completed` prefix is exactly `CANONICAL_STEPS[..5]`, which the per-step loop test asserts for step Bind.
- duplicate: `explicit_plan_carries_explicit_claim_source` (src/state_machine.rs:767) - covered by `explicit_plan_does_not_require_bundle_intents` (src/state_machine.rs:719). Both pin that an explicit plan carries `UsbipClaimSource::Explicit` (via `is_explicit()` vs a match); the covering test also pins busid/env/vm passthrough and canonical steps.
- gap: `build_usbip_plan` declared-path failure checks (src/state_machine.rs:254-333) - empty busid/env/vm and missing bundle firewall/bind intents (tagged Firewall/Bind steps) have no test in src or tests/; this is the daemon's primary plan path and its fail-fast error surface is entirely unpinned.
- gap: `UsbipBindingContext::new` zero-physical-key / empty-ref rejection (src/broker.rs:41-63) - production-called from d2bd (shared_provider_effects.rs) but only the empty-vm/env paths are reached via the test-only `before_host_effects` wrapper; zero key and empty intent-ref paths are untested.

## Keep
- `context_is_required_before_host_effects` (src/broker.rs:438) - pins empty vm/env rejection and value-equality of the binding context.
- `descriptors_declare_the_usb_types` (src/driver.rs:356) - pins the two USB ResourceType descriptors, exportable flags, and the shared factory's type set.
- `rows_keep_the_preserved_identity` (src/driver.rs:391) - pins registration rows' resource types, components, effect ids, provider ref, resync cadence.
- `bind_admits_the_declared_usb_class` (src/vocabulary.rs:83) - pins admission of the declared `usb` bus class.
- `bind_refuses_a_device_outside_the_declared_class` (src/vocabulary.rs:89) - pins refusal of hidraw/drm/pci/tpm classes and the error code string.
- `canonical_order_is_pinned` (src/state_machine.rs:555) - pins the CANONICAL_STEPS constant order directly.
- `stop_order_preserves_per_env_sidecars` (src/state_machine.rs:572) - pins reversed per-busid stop order excluding Backend/Proxy.
- `rollback_scope_filters_per_env_sidecars` (src/state_machine.rs:590) - pins the per-env-sidecar vs per-busid-rollback predicate classification.
- `proxy_failure_rollback_preserves_per_env_sidecars` (src/state_machine.rs:631) - pins completed prefix, rollback order with Proxy failed, Bind undone, sidecars preserved.
- `step_as_str_is_stable_kebab_case` (src/state_machine.rs:664) - pins stable operator-greppable step identifiers.
- `execute_happy_path_calls_every_step_in_order` (src/state_machine.rs:675) - pins full happy-path dispatch.
- `each_step_failure_surfaces_typed_error` (src/state_machine.rs:690) - pins per-step typed error (busid/step/reason), fail-fast halt, completed prefix.
- `explicit_plan_does_not_require_bundle_intents` (src/state_machine.rs:719) - pins explicit plan succeeds without resolver, field passthrough, canonical steps, Explicit source.
- `explicit_plan_accepts_valid_busid_shapes` (src/state_machine.rs:730) - pins valid busid shapes build a plan.
- `explicit_plan_rejects_invalid_busid_shapes` (src/state_machine.rs:738) - pins invalid shapes rejected at Lock step.
- `explicit_plan_rejects_empty_env_or_vm` (src/state_machine.rs:756) - pins empty env/vm rejection.
- `declared_plan_carries_bundle_refs_in_claim_source` (src/state_machine.rs:777) - pins the Declared variant shape and non-explicit flag the daemon branches on.

cross-check: busid-shape validation is pinned three ways - this crate's `explicit_plan_accepts/rejects_valid_busid_shapes`, tests/conformance.rs::adapted_bus_id_validation_rejects_unsafe_and_noncanonical_values, and d2b-contracts/src/usbip.rs unit tests of `validate_bus_id` - the unit tests' only unique part is the Lock-step error tagging; C2 should judge the layered re-pinning.

route-out: none.