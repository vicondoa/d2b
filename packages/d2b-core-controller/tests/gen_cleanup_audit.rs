use d2b_contracts_resource::v3::{ZoneId, ZoneRevision};
use d2b_contracts_zone_session::v3::ZoneBundle;
use d2b_core_controller::{
    audit::{AuditError, AuditEventKind},
    configuration::{
        BundleActivation, CanonicalSpec, ManagementAgent, RetainedGenerations,
        StoredResource, ZoneConfigController,
    },
};

mod common;

use common::{input, key, now};

fn bundle(value: char, include: bool) -> ZoneBundle {
    let resources: Vec<_> = include
        .then(|| input("Device", "sensitive-device-name", "desired"))
        .into_iter()
        .collect();
    common::bundle(value, resources)
}

fn stored() -> StoredResource {
    StoredResource::new(
        key("Device", "sensitive-device-name"),
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
        .complete_intent(&key("Device", "sensitive-device-name"))
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
        .observe_deleted(&key("Device", "sensitive-device-name"), ZoneRevision::new(17), &now())
        .unwrap();
    assert_eq!(
        controller
            .observe_deleted(&key("Device", "sensitive-device-name"), ZoneRevision::new(17), &now())
            .unwrap(),
        d2b_core_controller::configuration::CleanupOutcome::Deleted
    );
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
