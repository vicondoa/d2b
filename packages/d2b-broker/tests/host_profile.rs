use d2b_contracts_broker::broker_wire::BrokerProfile;

#[test]
fn host_profile_keeps_the_complete_closed_operation_catalog() {
    let operations = BrokerProfile::Host.operations();

    for operation in [
        "ApplyNftables",
        "OpenZoneStore",
        "SpawnRunner",
        "ApplyHostGenerationHandoff",
        "ExportBrokerAudit",
        "ValidateBundle",
        "ConsumeLifecycleLease",
    ] {
        assert!(
            operations.contains(&operation),
            "host profile lost the existing operation {operation}"
        );
        assert!(
            BrokerProfile::Host.allows_operation(operation),
            "host profile must admit {operation}"
        );
    }
}

#[test]
fn host_profile_is_not_an_open_ended_default() {
    assert!(!BrokerProfile::Host.allows_operation("SelectProfile"));
    assert!(!BrokerProfile::Host.allows_operation("UnknownFutureOperation"));
}
