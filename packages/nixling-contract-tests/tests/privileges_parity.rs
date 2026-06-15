use std::collections::BTreeMap;

use nixling_contract_tests::load_privileges_fixture_from_env;
use nixling_core::privileges::{OperationAuthz, PrivilegesJson};

// W3 contract test migrated from tests/privileges-json-rust-vs-nix-eval.sh.
//
// The bash gate perl-parsed packages/nixling-core/src/privileges.rs to
// recover the Rust authorization matrix, then diffed it against the
// Nix-rendered privileges.json (operation set + allowedGroups). In Rust we
// don't need to re-parse the source: `PrivilegesJson::w1` IS the canonical
// matrix built from the same const rows, so the contract test compares the
// rendered fixture directly against it.
fn operation_groups(ops: &[OperationAuthz]) -> BTreeMap<String, Vec<String>> {
    ops.iter()
        .map(|op| {
            let mut groups = op.allowed_groups.clone();
            groups.sort();
            (op.operation.clone(), groups)
        })
        .collect()
}

#[test]
fn rendered_privileges_matches_rust_matrix() {
    let rendered = load_privileges_fixture_from_env();
    let rust = PrivilegesJson::w1(rendered.schema_version.clone());

    // Operation set + allowedGroups parity, public and broker, in one
    // BTreeMap equality each (the map diff localizes any drift to the
    // offending operation, mirroring the bash gate's rust-only/nix-only +
    // per-op mismatch reporting).
    assert_eq!(
        operation_groups(&rendered.public_operations),
        operation_groups(&rust.public_operations),
        "rendered privileges.json publicOperations drifted from the Rust matrix \
         (PUBLIC_OPERATION_AUTHZ in nixling-core::privileges)"
    );
    assert_eq!(
        operation_groups(&rendered.broker_operations),
        operation_groups(&rust.broker_operations),
        "rendered privileges.json brokerOperations drifted from the Rust matrix \
         (BROKER_OPERATION_AUTHZ in nixling-core::privileges)"
    );
}
