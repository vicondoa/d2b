use std::collections::BTreeMap;

use d2b_contracts::{
    BundleMetadata, BundleResource, ZoneBundle,
    v3::{
        CanonicalJsonObject, ResourceName, ResourceTypeName, SchemaFingerprint, Timestamp, ZoneId,
        ZoneRevision,
    },
};
use d2b_core_controller::{
    audit::{AuditError, AuditEventKind},
    configuration::{
        BundleActivation, CanonicalSpec, ManagementAgent, ResourceKey, RetainedGenerations,
        StoredResource, ZoneConfigController,
    },
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

fn bundle(value: char, include: bool) -> ZoneBundle {
    let resources = include
        .then(|| {
            BundleResource::new(
                ResourceTypeName::parse("Device").unwrap(),
                BundleMetadata::new(
                    ResourceName::parse("sensitive-device-name").unwrap(),
                    ZoneId::parse("work").unwrap(),
                    None,
                    BTreeMap::new(),
                    BTreeMap::new(),
                )
                .unwrap(),
                CanonicalJsonObject::parse(br#"{"value":"desired"}"#).unwrap(),
            )
            .unwrap()
        })
        .into_iter()
        .collect();
    ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest(value),
        resources,
        BTreeMap::new(),
    )
    .unwrap()
}

fn stored() -> StoredResource {
    StoredResource::new(
        key("sensitive-device-name"),
        ManagementAgent::Configuration,
        Some(bundle('a', true).content_hash().clone()),
        CanonicalSpec::from_fields([("spec", r#"{"value":"desired"}"#)]).unwrap(),
    )
}

#[test]
fn cleanup_audit_is_redacted_and_recovery_append_is_exactly_once() {
    let mut controller = ZoneConfigController::new(
        ZoneId::parse("work").unwrap(),
        RetainedGenerations::default_value(),
    );
    controller
        .activate(BundleActivation::new(bundle('a', true)), &[], &now())
        .unwrap();
    controller
        .complete_intent(&key("sensitive-device-name"))
        .unwrap();
    let result = controller
        .activate(
            BundleActivation::new(bundle('b', false)),
            &[stored()],
            &now(),
        )
        .unwrap();
    assert!(result.audits().iter().any(|event| {
        event.kind() == AuditEventKind::ResourceDeletionRequested
            && event.resource_name_digest().is_some()
    }));
    let rendered = format!("{:?}", result.audits());
    assert!(!rendered.contains("sensitive-device-name"));
    assert!(!rendered.contains("desired"));

    controller
        .observe_deleted(&key("sensitive-device-name"), ZoneRevision::new(17), &now())
        .unwrap();
    assert!(controller.audit().events().iter().any(|event| {
        event.kind() == AuditEventKind::ResourceDeleted
            && event.event() == "deleted"
            && event.trigger() == Some("config-cleanup")
    }));
    let duplicate = controller.audit().events().iter().find_map(|event| {
        (event.kind() == AuditEventKind::ResourceDeleted).then(|| event.recovery_key())
    });
    assert!(duplicate.is_some());
    let last = controller.audit().events().last().unwrap().clone();
    let mut recovered_ledger = controller.audit().clone();
    assert_eq!(
        recovered_ledger.append(last),
        Err(AuditError::AlreadyAppended)
    );
}
