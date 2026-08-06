use std::collections::BTreeMap;

use d2b_contracts::{
    BundleMetadata, BundleResource, ZoneBundle, ZoneBundleError,
    v3::{
        CanonicalJsonObject, ResourceBundleGenerationId, ResourceName, ResourceTypeName,
        ResourceUid, SchemaFingerprint, Timestamp, ZoneId, ZoneRevision,
    },
};
use d2b_core_controller::{
    audit::{AuditEventKind, AuditReason},
    cleanup::{CleanupZonePhase, PendingCleanupState},
    configuration::{
        ActivationOutcome, BundleActivation, BundleResource as PlannedResource, CanonicalSpec,
        ConfigurationService, GenerationPhase, ManagementAgent, ResourceBundle, ResourceKey,
        RetainedGenerations, StoredResource, ZoneConfigController,
        bundle_apply::{BundleApplyEffect, ResourceApplyOperation},
    },
};

fn zone() -> ZoneId {
    ZoneId::parse("work").unwrap()
}

fn timestamp() -> Timestamp {
    Timestamp::parse("2026-08-01T00:00:00.000Z").unwrap()
}

fn digest(byte: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn generation(byte: char) -> ResourceBundleGenerationId {
    ResourceBundleGenerationId::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn key(resource_type: &str, name: &str) -> ResourceKey {
    ResourceKey::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        ResourceName::parse(name).unwrap(),
    )
}

fn input(resource_type: &str, name: &str, value: &str) -> BundleResource {
    BundleResource::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        BundleMetadata::new(
            ResourceName::parse(name).unwrap(),
            zone(),
            None,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap(),
        CanonicalJsonObject::parse(format!(r#"{{"value":"{value}"}}"#).as_bytes()).unwrap(),
    )
    .unwrap()
}

fn bundle(byte: char, resources: impl IntoIterator<Item = BundleResource>) -> ZoneBundle {
    ZoneBundle::build(
        zone(),
        digest(byte),
        resources.into_iter().collect(),
        BTreeMap::new(),
    )
    .unwrap()
}

fn stored(
    resource_type: &str,
    name: &str,
    managed_by: ManagementAgent,
    source_generation: Option<char>,
    value: &str,
) -> StoredResource {
    let configuration_generation = source_generation.map(|byte| {
        bundle(byte, [input(resource_type, name, value)])
            .content_hash()
            .clone()
    });
    StoredResource::new(
        key(resource_type, name),
        managed_by,
        configuration_generation,
        CanonicalSpec::from_fields([("spec", format!(r#"{{"value":"{value}"}}"#))]).unwrap(),
    )
}

fn stored_with_generation_id(
    resource_type: &str,
    name: &str,
    managed_by: ManagementAgent,
    configuration_generation: Option<ResourceBundleGenerationId>,
    value: &str,
) -> StoredResource {
    StoredResource::new(
        key(resource_type, name),
        managed_by,
        configuration_generation,
        CanonicalSpec::from_fields([("spec", format!(r#"{{"value":"{value}"}}"#))]).unwrap(),
    )
}

fn configured_bundle(
    byte: char,
    resources: impl IntoIterator<Item = BundleResource>,
) -> ResourceBundle {
    ResourceBundle::new(
        zone(),
        generation(byte),
        resources
            .into_iter()
            .map(|resource| {
                let resource_key = key(
                    resource.resource_type().as_str(),
                    resource.metadata().name().as_str(),
                );
                PlannedResource::new(
                    resource_key,
                    CanonicalSpec::from_fields([(
                        "spec",
                        String::from_utf8(resource.spec().to_canonical_bytes()).unwrap(),
                    )])
                    .unwrap(),
                )
            })
            .collect(),
    )
    .unwrap()
}

fn complete_create(
    controller: &mut ZoneConfigController,
    result: &d2b_core_controller::configuration::ActivationResult,
    keys: &[ResourceKey],
) {
    assert!(!result.is_noop());
    for key in keys {
        controller.complete_intent(key).unwrap();
    }
}

#[test]
fn managedby_configuration_set_on_activated_resources() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let resources = vec![
        input("Provider", "observability-otel", "provider"),
        input("Device", "gpu", "device"),
    ];
    let result = controller
        .activate(
            BundleActivation::new(bundle('a', resources)),
            &[],
            &timestamp(),
        )
        .unwrap();

    let persisted: Vec<_> = result
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            BundleApplyEffect::PersistResource {
                managed_by,
                configuration_generation,
                operation: ResourceApplyOperation::Create,
                ..
            } => Some((*managed_by, *configuration_generation)),
            _ => None,
        })
        .collect();
    assert_eq!(persisted.len(), 2);
    assert!(persisted.iter().all(|(managed_by, generation)| *managed_by
        == ManagementAgent::Configuration
        && generation.get() > 0));
}

#[test]
fn controller_created_resources_have_managedby_controller() {
    let child = stored(
        "Process",
        "provider-child",
        ManagementAgent::Controller,
        None,
        "child",
    );
    assert_eq!(child.managed_by(), ManagementAgent::Controller);
    assert_eq!(child.configuration_generation(), None);

    let desired = bundle('a', [input("Process", "provider-child", "configured")]);
    let diff = d2b_core_controller::configuration::GenerationDiff::compute(&desired, &[child]);
    assert_eq!(
        diff.by_kind(d2b_core_controller::configuration::DiffKind::Collision)
            .len(),
        1
    );
    assert!(
        !diff
            .entries()
            .iter()
            .any(|entry| entry.kind() == d2b_core_controller::configuration::DiffKind::Removed)
    );
}

#[test]
fn absent_resource_receives_delete_on_new_generation() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let provider_key = key("Provider", "observability-otel");
    controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&provider_key).unwrap();

    let result = controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('a'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();
    assert!(result.effects().iter().any(|effect| matches!(
        effect,
        BundleApplyEffect::DeleteResource {
            key,
            deletion_requested_at,
            ..
        } if key == &provider_key && deletion_requested_at == &timestamp()
    )));
    assert_eq!(result.state().pending_cleanup_count(), 1);
    assert!(result.audits().iter().any(|event| {
        event.kind() == AuditEventKind::ResourceDeletionRequested
            && event.event() == "delete-scheduled"
            && event.trigger() == Some("config-cleanup")
            && event.reason() == Some(AuditReason::AbsentFromNewGeneration)
    }));
}

#[test]
fn cleanup_does_not_touch_controller_children() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let provider_key = key("Provider", "observability-otel");
    let child_key = key("Process", "provider-child");
    controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&provider_key).unwrap();

    let result = controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[
                stored(
                    "Provider",
                    "observability-otel",
                    ManagementAgent::Configuration,
                    Some('a'),
                    "provider",
                ),
                stored(
                    "Process",
                    "provider-child",
                    ManagementAgent::Controller,
                    None,
                    "child",
                ),
            ],
            &timestamp(),
        )
        .unwrap();
    let deleted: Vec<_> = result
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            BundleApplyEffect::DeleteResource { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(deleted, vec![&provider_key]);
    assert!(!deleted.contains(&&child_key));
}

#[test]
fn deletion_sets_deletionrequestedat_not_phase() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let resource_key = key("Provider", "observability-otel");
    controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&resource_key).unwrap();
    let result = controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('a'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();
    let delete = result
        .effects()
        .iter()
        .find_map(|effect| match effect {
            BundleApplyEffect::DeleteResource {
                deletion_requested_at,
                ..
            } => Some(deletion_requested_at),
            _ => None,
        })
        .expect("a removed resource receives a Delete intent");
    assert_eq!(delete, &timestamp());
    let rendered = format!("{result:?}");
    assert!(!rendered.contains("Deleting"));
}

#[test]
fn final_deletion_is_atomic() {
    let mut service = ConfigurationService::empty(zone(), RetainedGenerations::default_value());
    let first = configured_bundle('a', [input("Provider", "observability-otel", "provider")]);
    let first_plan = match service.begin_activation(&first, &[], &timestamp()).unwrap() {
        ActivationOutcome::Planned(plan) => plan,
        ActivationOutcome::Unchanged => panic!("first generation must be new"),
    };
    let proof = service.commit_activation(first_plan, &timestamp()).unwrap();
    service.release_activation_effects(proof).unwrap();
    let resource_key = key("Provider", "observability-otel");
    service.complete_intent(&resource_key).unwrap();

    let second = configured_bundle('b', []);
    let second_plan = match service
        .begin_activation(
            &second,
            &[stored_with_generation_id(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some(generation('a')),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap()
    {
        ActivationOutcome::Planned(plan) => plan,
        ActivationOutcome::Unchanged => panic!("second generation must remove the provider"),
    };
    let proof = service
        .commit_activation(second_plan, &timestamp())
        .unwrap();
    service.release_activation_effects(proof).unwrap();
    let revision = ZoneRevision::new(17);
    assert_eq!(
        service.record_cleanup_audit_appended(revision),
        Err(d2b_core_controller::configuration::ConfigurationError::CleanupNotCompleted)
    );

    let pass = service
        .begin_cleanup(
            &resource_key,
            d2b_core_controller::configuration::CleanupObservation::new(0, 0),
        )
        .unwrap();
    let proof = service.commit_cleanup(pass, revision).unwrap();
    assert!(service.cleanup_tracking(&resource_key).is_none());
    assert_eq!(
        service.record_cleanup_audit_appended(revision),
        Err(d2b_core_controller::configuration::ConfigurationError::CleanupNotCompleted)
    );
    let effects = service.release_cleanup_effects(proof).unwrap();
    assert_eq!(
        effects,
        vec![d2b_core_controller::configuration::CleanupEffect::AppendCleanupAudit(revision)]
    );
    assert!(service.cleanup_audit_pending(revision));
    assert!(service.record_cleanup_audit_appended(revision).is_ok());
    assert!(!service.cleanup_audit_pending(revision));
}

#[test]
fn pending_cleanup_condition_set_on_zone() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let resource_key = key("Provider", "observability-otel");
    controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&resource_key).unwrap();
    let result = controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('a'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();
    let condition = result.state().pending_cleanup_condition();
    assert_eq!(condition.condition_type(), "pending-cleanup");
    assert_eq!(condition.status(), PendingCleanupState::True.as_str());
    assert_eq!(condition.reason(), "ConfigRemoved");
}

#[test]
fn zone_is_degraded_not_failed_during_cleanup() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let resource_key = key("Provider", "observability-otel");
    controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&resource_key).unwrap();
    let result = controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('a'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();
    assert_eq!(result.state().phase(), GenerationPhase::Degraded);
    assert!(!result.state().cleanup_failed());
}

#[test]
fn pending_cleanup_cleared_after_deletion_completes() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let resource_key = key("Provider", "observability-otel");
    controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&resource_key).unwrap();
    controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('a'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();
    assert_eq!(
        controller
            .observe_deleted(&resource_key, ZoneRevision::new(9), &timestamp())
            .unwrap(),
        d2b_core_controller::configuration::CleanupOutcome::Deleted
    );
    let condition = controller.state().pending_cleanup_condition();
    assert_eq!(condition.state(), PendingCleanupState::False);
    assert_eq!(condition.phase(), CleanupZonePhase::Ready);
    assert_eq!(controller.state().phase(), GenerationPhase::Ready);
}

#[test]
fn prior_generation_retained_count_based() {
    let mut service = ConfigurationService::empty(zone(), RetainedGenerations::default_value());
    for byte in ['a', 'b', 'c', 'd', 'e'] {
        let plan = match service
            .begin_activation(&configured_bundle(byte, []), &[], &timestamp())
            .unwrap()
        {
            ActivationOutcome::Planned(plan) => plan,
            ActivationOutcome::Unchanged => panic!("each generation has a new digest"),
        };
        let proof = service.commit_activation(plan, &timestamp()).unwrap();
        service.release_activation_effects(proof).unwrap();
    }
    let record = service.record().unwrap();
    assert_eq!(record.retention_ring().len(), 3);
    assert_eq!(
        record.retention_ring(),
        &[generation('b'), generation('c'), generation('d')]
    );
    assert_eq!(record.active_generation_id(), &generation('e'));
}

#[test]
fn rollback_schedules_delete_for_new_generation_resources() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let provider_key = key("Provider", "observability-otel");
    let first = bundle('a', []);
    controller
        .activate(BundleActivation::new(first.clone()), &[], &timestamp())
        .unwrap();
    let second = bundle('b', [input("Provider", "observability-otel", "provider")]);
    let second_result = controller
        .activate(BundleActivation::new(second), &[], &timestamp())
        .unwrap();
    complete_create(
        &mut controller,
        &second_result,
        std::slice::from_ref(&provider_key),
    );

    let rolled_back = controller
        .rollback(
            &first.content_hash().clone(),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('b'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();
    assert!(rolled_back.effects().iter().any(|effect| matches!(
        effect,
        BundleApplyEffect::DeleteResource { key, .. } if key == &provider_key
    )));
    assert_eq!(rolled_back.state().pending_cleanup_count(), 1);
}

#[test]
fn audit_segments_preserved_on_provider_delete() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let provider_key = key("Provider", "observability-otel");
    let first = controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&provider_key).unwrap();
    let before_delete = controller.audit().events().to_vec();
    assert!(!before_delete.is_empty());

    controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('a'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();
    let after_schedule = controller.audit().events();
    assert!(after_schedule.len() > before_delete.len());
    assert_eq!(
        &after_schedule[..before_delete.len()],
        before_delete.as_slice()
    );
    assert!(
        first
            .audits()
            .iter()
            .any(|event| event.kind() == AuditEventKind::GenerationActivated)
    );
}

#[test]
fn cleanup_stall_condition_set() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let provider_key = key("Provider", "observability-otel");
    controller
        .activate(
            BundleActivation::new(bundle(
                'a',
                [input("Provider", "observability-otel", "provider")],
            )),
            &[],
            &timestamp(),
        )
        .unwrap();
    controller.complete_intent(&provider_key).unwrap();
    controller
        .activate(
            BundleActivation::new(bundle('b', [])),
            &[stored(
                "Provider",
                "observability-otel",
                ManagementAgent::Configuration,
                Some('a'),
                "provider",
            )],
            &timestamp(),
        )
        .unwrap();

    assert_eq!(
        controller
            .mark_cleanup_stalled(&provider_key, AuditReason::FinalizerBlocked, &timestamp())
            .unwrap(),
        d2b_core_controller::configuration::CleanupOutcome::Stalled
    );
    let condition = controller.state().cleanup_stall_condition().unwrap();
    assert_eq!(condition.condition_type(), "cleanup-stalled");
    assert_eq!(condition.status(), "True");
    assert_eq!(condition.reason(), "finalizer-blocked");
    assert_eq!(condition.phase(), CleanupZonePhase::Degraded);
    assert_eq!(controller.state().phase(), GenerationPhase::Degraded);
    assert!(
        controller
            .audit()
            .events()
            .iter()
            .any(|event| event.kind() == AuditEventKind::CleanupStalled)
    );
}

#[test]
fn generation_rejected_emits_audit_record() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let candidate = BundleActivation::new(bundle(
        'a',
        [input("Provider", "observability-otel", "provider")],
    ))
    .with_schema_validation_failure();
    let error = controller
        .activate(candidate, &[], &timestamp())
        .unwrap_err();
    assert_eq!(
        error,
        d2b_core_controller::configuration::ActivationError::SchemaValidationFailed
    );
    assert!(controller.service().record().is_none());
    assert!(controller.pending_cleanup().is_empty());
    let event = controller
        .audit()
        .events()
        .iter()
        .find(|event| event.kind() == AuditEventKind::GenerationRejected)
        .expect("schema rejection is authoritative audit");
    assert_eq!(event.event(), "generation-rejected");
    assert_eq!(event.reason(), Some(AuditReason::SchemaValidationFailed));
}

#[test]
fn invalid_provider_config_does_not_block_unrelated_resource_activation() {
    let mut controller = ZoneConfigController::with_defaults(zone());
    let provider_key = key("Provider", "broken-provider");
    let result = controller
        .activate(
            BundleActivation::new(bundle(
                'd',
                [
                    input("Provider", "broken-provider", "invalid"),
                    input("Device", "unrelated", "valid"),
                ],
            ))
            .with_invalid_provider_config([provider_key.clone()]),
            &[],
            &timestamp(),
        )
        .unwrap();

    assert_eq!(result.state().invalid_provider_config_count(), 1);
    assert_eq!(result.state().active_generation().unwrap().get(), 1);
    assert!(result.effects().iter().any(|effect| matches!(
        effect,
        BundleApplyEffect::MarkProviderConfigInvalid { key } if key == &provider_key
    )));
    assert!(result.effects().iter().any(|effect| matches!(
        effect,
        BundleApplyEffect::PersistResource {
            resource,
            operation: ResourceApplyOperation::Create,
            ..
        } if resource.key() == &key("Device", "unrelated")
    )));
    let meta_index = result
        .effects()
        .iter()
        .position(|effect| matches!(effect, BundleApplyEffect::RecordStoreMeta(_)))
        .expect("store_meta is recorded after the generation commit");
    let first_resource_index = result
        .effects()
        .iter()
        .position(|effect| matches!(effect, BundleApplyEffect::PersistResource { .. }))
        .expect("resource effects are present");
    assert!(meta_index < first_resource_index);
}

#[test]
fn zone_uid_and_artifact_catalog_mismatch_fail_before_activation() {
    let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let other_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap();
    let mut controller = ZoneConfigController::with_defaults(zone());
    controller.set_artifact_catalog_digest(digest('f'));
    let catalog_error = controller
        .activate(BundleActivation::new(bundle('a', [])), &[], &timestamp())
        .unwrap_err();
    assert_eq!(
        catalog_error,
        d2b_core_controller::configuration::ActivationError::ArtifactCatalogMismatch
    );

    let mut controller = ZoneConfigController::with_defaults(zone());
    controller
        .activate(
            BundleActivation::new(bundle('a', [input("Device", "gpu", "first")]))
                .with_zone_uid(Some(uid)),
            &[],
            &timestamp(),
        )
        .unwrap();
    let error = controller
        .activate(
            BundleActivation::new(bundle('b', [input("Device", "gpu", "second")]))
                .with_zone_uid(Some(other_uid)),
            &[],
            &timestamp(),
        )
        .unwrap_err();
    assert_eq!(
        error,
        d2b_core_controller::configuration::ActivationError::ZoneUidMismatch
    );
}

#[test]
fn tampered_content_hash_is_rejected_before_configuration_activation() {
    let original = bundle('a', [input("Device", "gpu", "configured")]);
    let mut wire: serde_json::Value =
        serde_json::from_slice(&original.canonical_bytes().unwrap()).unwrap();
    wire["contentHash"] = serde_json::Value::String(format!("sha256:{}", "f".repeat(64)));
    let tampered = serde_json::to_vec(&wire).unwrap();
    assert_eq!(
        ZoneBundle::from_json(&tampered),
        Err(ZoneBundleError::IntegrityFailure)
    );
}
