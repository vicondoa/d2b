use std::collections::BTreeMap;

use d2b_contracts::{
    BundleMetadata, BundleResource, ZoneBundle,
    v3::{
        CanonicalJsonObject, ResourceName, ResourceTypeName, SchemaFingerprint, Timestamp, ZoneId,
    },
};
use d2b_core_controller::configuration::{
    BundleActivation, CanonicalSpec, DiffKind, ManagementAgent, ResourceKey, RetainedGenerations,
    StoredResource, ZoneConfigController,
};

fn digest(value: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", value.to_string().repeat(64))).unwrap()
}

fn now() -> Timestamp {
    Timestamp::parse("2026-08-01T00:00:00.000Z").unwrap()
}

fn key(name: &str) -> ResourceKey {
    ResourceKey::new(
        ResourceTypeName::parse("Device").unwrap(),
        ResourceName::parse(name).unwrap(),
    )
}

fn bundle() -> ZoneBundle {
    ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest('c'),
        vec![
            BundleResource::new(
                ResourceTypeName::parse("Device").unwrap(),
                BundleMetadata::new(
                    ResourceName::parse("device-child-owner").unwrap(),
                    ZoneId::parse("work").unwrap(),
                    None,
                    BTreeMap::new(),
                    BTreeMap::new(),
                )
                .unwrap(),
                CanonicalJsonObject::parse(br#"{"value":"desired"}"#).unwrap(),
            )
            .unwrap(),
        ],
        BTreeMap::new(),
    )
    .unwrap()
}

#[test]
fn controller_owned_resource_is_protected_and_other_items_can_activate() {
    let mut controller = ZoneConfigController::new(
        ZoneId::parse("work").unwrap(),
        RetainedGenerations::default_value(),
    );
    let stored = [StoredResource::new(
        key("device-child-owner"),
        ManagementAgent::Controller,
        None,
        CanonicalSpec::from_fields([("spec", r#"{"value":"old"}"#)]).unwrap(),
    )];
    let result = controller
        .activate(BundleActivation::new(bundle()), &stored, &now())
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
