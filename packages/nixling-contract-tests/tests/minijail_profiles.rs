use nixling_contract_tests::load_bundle_resolver_from_env;

// W3 contract test: the rendered fixture-smoke bundle must pass EVERY
// minijail profile invariant (BundleResolver::validate_minijail_profiles):
// non-empty profile ids, no uid/gid 0 without an ADR carve-out, /nix/store
// read-only without a carve-out, and cgroup subtrees confined to nixling/.
//
// This is the POSITIVE half of retiring the minijail-validator /
// static-invariant-uid0 bash gates — it proves the real rendered output is
// policy-compliant end-to-end. The NEGATIVE half (synthetic rejection cases)
// belongs in nixling-core unit tests over validate_minijail_profiles, which
// are blocked until the build_personal_dev_bundle test helper's stale
// manifest fixture (manifest_version 4 vs MANIFEST_VERSION_CURRENT 5) is
// fixed; see plan.md.
#[test]
fn rendered_bundle_passes_all_minijail_profile_invariants() {
    let resolver = load_bundle_resolver_from_env();
    let validated = resolver.validate_minijail_profiles().unwrap_or_else(|violation| {
        panic!("rendered fixture bundle violates a minijail profile invariant: {violation:?}")
    });
    assert!(
        validated > 0,
        "expected the fixture bundle to contain at least one minijail profile to validate; \
         a zero count means the fixture emitter produced no per-role profiles (regression)"
    );
}
