use d2b_contracts_resource::v3::ZoneId;
use d2b_core_controller::configuration::{
    BundleActivation, CanonicalSpec, DiffKind, ManagementAgent, RetainedGenerations,
    StoredResource, ZoneConfigController,
};

mod common;

use common::{input, key, now};

#[test]
fn controller_owned_resource_is_protected_and_other_items_can_activate() {
    let mut controller = ZoneConfigController::new(
        ZoneId::parse("work").unwrap(),
        RetainedGenerations::default_value(),
    );
    let stored = [StoredResource::new(
        key("Device", "device-child-owner"),
        ManagementAgent::Controller,
        None,
        CanonicalSpec::from_fields([("spec", r#"{"value":"old"}"#)]).unwrap(),
    )];
    let result = controller
        .activate(
            BundleActivation::new(common::bundle(
                'c',
                vec![input("Device", "device-child-owner", "desired")],
            )),
            &stored,
            &now(),
        )
        .unwrap();
    assert_eq!(result.diff().by_kind(DiffKind::Collision).len(), 1);
    assert_eq!(result.state().pending_cleanup_count(), 0);
    assert!(result.audits().iter().any(|event| {
        event.kind() == d2b_core_controller::audit::AuditEventKind::ConfigurationCollision
    }));
    assert_eq!(
        controller.service().pending_cleanup().len(),
        0,
        "foreign-owned rows are never generation cleanup candidates"
    );
}
