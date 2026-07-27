const STORE_LIB: &str = include_str!("../src/lib.rs");
const STORE_MANIFEST: &str = include_str!("../Cargo.toml");
const REDB_LIB: &str = include_str!("../../d2b-resource-store-redb/src/lib.rs");
const REDB_OWNERSHIP: &str = include_str!("../../d2b-resource-store-redb/src/ownership.rs");
const REDB_MANIFEST: &str = include_str!("../../d2b-resource-store-redb/Cargo.toml");
const API_AUTHZ: &str = include_str!("../../d2b-resource-api/src/authz.rs");
const API_SERVICE: &str = include_str!("../../d2b-resource-api/src/service.rs");

#[test]
fn admission_capability_has_exactly_one_native_evaluator_call_site() {
    assert_eq!(API_AUTHZ.matches("admission.record_allow(").count(), 1);
    assert!(!API_SERVICE.contains(".record_allow("));
}

#[test]
fn store_crates_do_not_depend_on_api_or_embed_rbac() {
    for manifest in [STORE_MANIFEST, REDB_MANIFEST] {
        assert!(!manifest.contains("d2b-resource-api"));
    }
    for source in [STORE_LIB, REDB_LIB, REDB_OWNERSHIP] {
        assert!(!source.contains("RoleBinding"));
        assert!(!source.contains("Role::"));
    }
}
