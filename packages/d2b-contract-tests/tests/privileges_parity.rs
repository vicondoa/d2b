use std::collections::BTreeMap;

use d2b_contract_tests::load_privileges_fixture_from_env;
use d2b_core::privileges::{OperationAuthz, PrivilegesJson};

// W3 contract test migrated from tests/privileges-json-rust-vs-nix-eval.sh.
//
// The bash gate perl-parsed packages/d2b-core/src/privileges.rs to
// recover the Rust authorization matrix, then diffed it against the
// Nix-rendered privileges.json (operation set + allowedGroups). In Rust we
// don't need to re-parse the source: `PrivilegesJson::w1` IS the canonical
// matrix built from the same const rows, so the contract test compares the
// rendered fixture directly against it.
fn operation_groups(label: &str, ops: &[OperationAuthz]) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    for op in ops {
        let mut groups = op.allowed_groups.clone();
        groups.sort();
        // Reject duplicate operation keys: a BTreeMap would otherwise
        // last-wins-collapse two rows for the same operation, masking a
        // duplicate with conflicting allowedGroups that the retired bash
        // gate's row-wise join would have surfaced.
        assert!(
            map.insert(op.operation.clone(), groups).is_none(),
            "{label}: duplicate operation '{}' in the privileges matrix",
            op.operation
        );
    }
    map
}

#[test]
fn rendered_privileges_matches_rust_matrix() {
    let mut rendered = load_privileges_fixture_from_env();
    let retired = rendered
        .broker_operations
        .iter()
        .position(|operation| operation.operation == "GuestControlSign")
        .expect("Nix privilege emitter must retain GuestControlSign until declarative retirement");
    rendered.broker_operations.remove(retired);
    let rust = PrivilegesJson::w1(rendered.schema_version.clone());

    // Operation set + allowedGroups parity, public and broker, in one
    // BTreeMap equality each (the map diff localizes any drift to the
    // offending operation, mirroring the bash gate's rust-only/nix-only +
    // per-op mismatch reporting). Duplicate operations are rejected up front.
    assert_eq!(
        operation_groups("rendered.publicOperations", &rendered.public_operations),
        operation_groups("rust.public_operations", &rust.public_operations),
        "rendered privileges.json publicOperations drifted from the Rust matrix \
         (PUBLIC_OPERATION_AUTHZ in d2b-core::privileges)"
    );
    assert_eq!(
        operation_groups("rendered.brokerOperations", &rendered.broker_operations),
        operation_groups("rust.broker_operations", &rust.broker_operations),
        "rendered privileges.json brokerOperations drifted from the Rust matrix \
         (BROKER_OPERATION_AUTHZ in d2b-core::privileges)"
    );
}
