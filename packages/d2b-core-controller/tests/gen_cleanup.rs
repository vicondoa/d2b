use std::collections::BTreeMap;

use d2b_contracts::{
    BundleMetadata, BundleResource, ZoneBundle,
    v3::{
        CanonicalJsonObject, ResourceName, ResourceTypeName, SchemaFingerprint, Timestamp, ZoneId,
        ZoneRevision,
    },
};
use d2b_core_controller::configuration::{
    BundleActivation, CanonicalSpec, GenerationPhase, ManagementAgent, ResourceKey,
    RetainedGenerations, StoredResource, ZoneConfigController,
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

fn input_bundle(value: char, include_device: bool) -> ZoneBundle {
    let resources = if include_device {
        vec![
            BundleResource::new(
                ResourceTypeName::parse("Device").unwrap(),
                BundleMetadata::new(
                    ResourceName::parse("device-a").unwrap(),
                    ZoneId::parse("work").unwrap(),
                    None,
                    BTreeMap::new(),
                    BTreeMap::new(),
                )
                .unwrap(),
                CanonicalJsonObject::parse(br#"{"value":"configured"}"#).unwrap(),
            )
            .unwrap(),
        ]
    } else {
        Vec::new()
    };
    ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest(value),
        resources,
        BTreeMap::new(),
    )
    .unwrap()
}

fn stored_device() -> StoredResource {
    StoredResource::new(
        key("device-a"),
        ManagementAgent::Configuration,
        Some(input_bundle('a', true).content_hash().clone()),
        CanonicalSpec::from_fields([("spec", r#"{"value":"configured"}"#)]).unwrap(),
    )
}

#[test]
fn removed_configuration_resource_is_deleted_asynchronously() {
    let mut controller = ZoneConfigController::new(
        ZoneId::parse("work").unwrap(),
        RetainedGenerations::new(3).unwrap(),
    );
    let first = controller
        .activate(BundleActivation::new(input_bundle('a', true)), &[], &now())
        .unwrap();
    assert!(!first.is_noop());
    assert_eq!(first.state().phase(), GenerationPhase::Pending);
    controller.complete_intent(&key("device-a")).unwrap();

    let second = controller
        .activate(
            BundleActivation::new(input_bundle('b', false)),
            &[stored_device()],
            &now(),
        )
        .unwrap();
    assert_eq!(second.state().phase(), GenerationPhase::Degraded);
    assert_eq!(second.state().pending_cleanup_count(), 1);
    assert!(second.effects().iter().any(|effect| matches!(
        effect,
        d2b_core_controller::configuration::bundle_apply::BundleApplyEffect::DeleteResource { key: target, .. }
            if target == &key("device-a")
    )));
    assert!(!second.effects().iter().any(|effect| matches!(
        effect,
        d2b_core_controller::configuration::bundle_apply::BundleApplyEffect::PersistResource {
            operation:
                d2b_core_controller::configuration::bundle_apply::ResourceApplyOperation::Create,
            ..
        }
    )));

    controller
        .observe_deleted(&key("device-a"), ZoneRevision::new(9), &now())
        .unwrap();
    assert_eq!(controller.state().phase(), GenerationPhase::Ready);
    assert_eq!(controller.state().pending_cleanup_count(), 0);
}
