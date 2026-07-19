mod common;

use d2b_contracts::{v2_component_session::ServicePackage, v2_services::SERVICE_INVENTORY};

#[test]
fn audit_export_is_a_generated_authenticated_broker_method() {
    let policy = common::local_root_policy();
    assert_eq!(policy.service, ServicePackage::BrokerV2);
    let service = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2")
        .expect("broker service");
    assert!(
        service
            .methods
            .iter()
            .any(|method| method.name == "ExportAudit")
    );
}
