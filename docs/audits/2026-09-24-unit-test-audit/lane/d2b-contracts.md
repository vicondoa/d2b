# d2b-contracts - unit-test audit
tests: 118 · src files: 31
net: -8 tests, -38 lines

## Findings (biggest net first)
- duplicate: `workload_target_parse_canonical` (src/workload_identity.rs:209) - covered by `canonical_examples_parse_and_render` (src/target.rs:604). `WorkloadTarget` is a type alias for `RealmTarget`; both pin parse("builder.dev.d2b") → workload "builder", canonical "builder.dev.d2b".
- duplicate: `workload_target_parse_nested_realm` (src/workload_identity.rs:216) - covered by `canonical_examples_parse_and_render` (src/target.rs:604). Both pin parse("api.payments.work.d2b") → workload "api", realm "payments.work", canonical form.
- duplicate: `workload_target_rejects_no_dot` (src/workload_identity.rs:224) - covered by `bare_alias_requires_context_by_default` (src/target.rs:654). Both pin `RealmTarget::parse("builder")` failing.
- duplicate: `workload_target_rejects_missing_d2b_suffix` (src/workload_identity.rs:229) - covered by `multi_label_without_suffix_is_rejected` (src/target.rs:737). Both pin a missing ".d2b" suffix → `MissingSuffix`.
- duplicate: `stable_fingerprint_orders_by_capability_code` (src/capability.rs:478) - covered by `capability_fingerprint_is_stable_and_order_independent` (src/capability.rs:485). Both pin order-independent `stable_fingerprint`; the keeper also pins distinct-set fingerprints, negotiation-envelope fields, and the fingerprint length bound.
- duplicate: `frame_too_large_is_rejected` (src/lib.rs:328) - covered by `encode_frame_public_sock_cap_boundary_is_exact` (src/lib.rs:335). Both pin `encode_frame` rejecting an over-cap body with `wire-frame-too-large`; the keeper also pins cap-1/cap success and exact encoded lengths.
- trivial: `workload_identity_new_has_none_optional_fields` (src/workload_identity.rs:234) - constructor stores its arguments and leaves optionals `None`. Nothing lost.
- trivial: `workload_identity_target_accessor` (src/workload_identity.rs:244) - accessor echoes the stored `canonical_target`. Nothing lost.

- gap: configured-argv argc cap (128) and total-byte cap (16 KiB) never exercised (src/configured_argv.rs:31-53) - the only test pins empty/NUL/per-arg-length rejection.
- gap: `decode_frame` short-frame (<4 bytes → `frame-too-short`) and invalid-JSON body → `wire-malformed-json` untested (src/lib.rs:252-275) - only unknown-field and prefix-boundary paths are pinned.
- gap: `SemverRange::new` / `Version::new` invalid-input rejection untested (src/error.rs) - only the valid-range match path is pinned.

- cross-check: serde roundtrip families (`SecurityKeyStatusResponse`/`SecurityKeySessionResult` in security_key.rs, `WorkloadIdentity`/`WorkloadBackend`/`WorkloadRuntimeIntent` in workload_identity.rs, transparent-string ids in ids.rs/types.rs) - likely re-pinned by sibling contract crates' contract tests or generated-schema/golden tests; C2 to resolve.

## Keep
- capability.rs: 9 tests pin `has()` semantics, unknown-token preservation + fingerprint anti-downgrade, stable code mapping, decode caps (token length, set size), negotiation deny-unknown (serde + schema), fingerprint bound, fingerprint stability + negotiation fields.
- configured_argv.rs: `configured_argv_is_bounded_and_debug_redacted` - debug redaction + empty/NUL/overlong rejection.
- constellation_error.rs: 6 tests pin constructor fields (capability, fingerprint), correlation-id roundtrip, message bound at decode, deny-unknown, capability-denied requires capability.
- contract_id.rs: 3 tests pin ContractId bounds/shape, ReasonSlug lowercase, PathTemplate absolute.
- error.rs: 5 tests pin operator envelope, leaf discriminant, unique operator-visible kind records (37), semver range match, no host paths in messages.
- identity.rs: 2 tests pin converted-registry plane mapping and wrong-plane refusal naming/caller/terminality.
- identity_config.rs: 3 tests pin metadata-only parse/summary, secret-material rejection, invariant enforcement.
- ids.rs: 5 tests pin label/opaque-token shapes, fail-closed decode, transparent serialize, debug redaction classes.
- launcher.rs: 2 tests pin no-argv schema, version + true invariants required.
- lib.rs: remaining 5 tests pin unknown-field fail-closed, retired feature not negotiated, encode/decode cap boundaries, length-prefix roundtrip.
- opaque_payload.rs: 4 tests pin constructor bound, debug redaction, positive decode, oversized decode rejection.
- privileges_w3.rs: `wire_tags_are_unique_pascalcase` - unique W3 wire tags.
- realm.rs: 6 tests pin path forms, empty/malformed rejection, label cap, byte cap, descendant relationships, placement deny-unknown.
- security_key.rs: 4 tests pin status roundtrip, terminal-state enum coverage, deny-unknown, cancel-current default.
- target.rs: 17 tests pin parse/render, scheme stripping, serde/schema string shape, alias resolution/ambiguity/dedupe, legacy-node diagnostics, selector/reserved/empty/suffix/label/bounds rejections.
- token.rs: `parse_and_decode_are_fail_closed` - token parse + decode bounds.
- types.rs: 3 tests pin transparent ids, kebab-case path class, media-ref/usb-busid validation.
- unsafe_local_workloads.rs: 11 tests pin artifact validation family (defaults, redaction, schema version, workload/item bounds, shell quota, runtime-identity match, missing default item, local-vm sharing).
- usbip.rs: 3 tests pin canonical/rejected bus ids, hex-id sanitization.
- v3/ifname.rs: 6 tests pin derivation contract, role/guest hash inputs, zone/network binding, Linux limits, collision vs inconsistency, diagnostic redaction.
- workload.rs: 5 tests pin summary fields, deny-unknown, posture roundtrip, provider-family fixture coverage, no-argv summary.
- workload_identity.rs: remaining 8 tests pin identity roundtrips (minimal + optional), skip-none serialization, deny-unknown (identity + intent), backend roundtrips (local-vm, qemu-media), intent roundtrip.
