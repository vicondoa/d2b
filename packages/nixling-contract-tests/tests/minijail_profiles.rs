use nixling_contract_tests::load_bundle_resolver_from_env;

// W3 contract test: the rendered fixture-smoke bundle must pass EVERY
// minijail profile invariant (BundleResolver::validate_minijail_profiles):
// non-empty profile ids, no uid/gid 0 without an ADR carve-out, /nix/store
// read-only without a carve-out, and cgroup subtrees confined to nixling/.
//
// This is the POSITIVE half of retiring the minijail-validator /
// static-invariant-uid0 bash gates — it proves the real rendered output is
// policy-compliant end-to-end. The NEGATIVE half (synthetic rejection cases)
// lives in nixling-core unit tests over validate_minijail_profiles.
//
// RETIREMENT PREREQUISITES (W3 security panel findings) before retiring
// static-invariant-uid0 / minijail-validator-*:
//   - validate_minijail_profiles treats `adr_carve_out: Some(_)` as
//     sufficient and accepts Some(""); the bash gate required an ADR-like
//     reference. Either reject empty/malformed carve-outs in the validator
//     or document Some(_) as intentionally sufficient.
//   - RoleProfile has no requires_start_root; the bash gate coupled uid0 +
//     long-lived with requiresStartRoot=true. Decide whether to model that
//     in Rust or document the simplification.
//   - Replace the bash gate's schema-shape check (uid/requiresStartRoot
//     shapes must carry an ADR carve-out field) with a Rust/xtask assertion
//     over the current v2 schemas/DTOs (bundle-drift proves schema==DTO but
//     would not catch a future DTO dropping the field).
#[test]
fn rendered_bundle_passes_all_minijail_profile_invariants() {
    let resolver = load_bundle_resolver_from_env();
    let validated = resolver
        .validate_minijail_profiles()
        .unwrap_or_else(|violation| {
            panic!("rendered fixture bundle violates a minijail profile invariant: {violation:?}")
        });
    assert!(
        validated > 0,
        "expected the fixture bundle to contain at least one minijail profile to validate; \
         a zero count means the fixture emitter produced no per-role profiles (regression)"
    );
}
