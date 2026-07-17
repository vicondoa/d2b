use d2b_contract_tests::load_privileges_fixture_from_env;

#[test]
fn privileges_fixture_contains_broker_operation_matrix() {
    let privileges = load_privileges_fixture_from_env();

    assert!(
        !privileges.broker_operations.is_empty(),
        "privileges.json brokerOperations must not be empty"
    );
    assert!(
        privileges
            .broker_operations
            .iter()
            .any(|row| row.operation == "DelegateCgroupV2"),
        "privileges.json must include the DelegateCgroupV2 broker operation"
    );
}
