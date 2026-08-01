use std::collections::BTreeMap;

use d2b_contracts::v3::{
    CanonicalJsonObject, ConfigurationGeneration, ResourceName, ResourceTypeName,
    SchemaFingerprint, Timestamp, ZoneId,
};
use d2b_contracts::{BundleMetadata, BundleResource, ZoneBundle};
use d2b_core_controller::{
    configuration::{
        ActivationOutcome, BundleResource as PlannedResource, CanonicalSpec, ConfigurationService,
        ResourceBundle, ResourceKey, RetainedGenerations,
        generation_transition::{committed_configuration_generation, plan_generation_transition},
    },
    resource_store::{PersistedResourceMetadata, PersistedResourceRecord},
};

fn digest(byte: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn key(resource_type: &str, name: &str) -> ResourceKey {
    ResourceKey::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        ResourceName::parse(name).unwrap(),
    )
}

fn input(resource_type: &str, name: &str) -> BundleResource {
    BundleResource::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        BundleMetadata::new(
            ResourceName::parse(name).unwrap(),
            ZoneId::parse("work").unwrap(),
            None,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap(),
        CanonicalJsonObject::parse(br#"{"value":"configured"}"#).unwrap(),
    )
    .unwrap()
}

fn bundle() -> ZoneBundle {
    ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest('a'),
        vec![input("Volume", "conflict"), input("Network", "main")],
        BTreeMap::new(),
    )
    .unwrap()
}

fn committed()
-> d2b_core_controller::configuration::generation_transition::CommittedConfigurationGeneration {
    let input = bundle();
    let mut service = ConfigurationService::empty(
        ZoneId::parse("work").unwrap(),
        RetainedGenerations::default_value(),
    );
    let resource_bundle = ResourceBundle::new(
        ZoneId::parse("work").unwrap(),
        input.content_hash().clone(),
        vec![PlannedResource::new(
            key("Network", "main"),
            CanonicalSpec::from_fields([("value", "configured")]).unwrap(),
        )],
    )
    .unwrap();
    let plan = match service
        .begin_activation(
            &resource_bundle,
            &[],
            &Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
        )
        .unwrap()
    {
        ActivationOutcome::Planned(plan) => plan,
        ActivationOutcome::Unchanged => panic!("new generation must plan"),
    };
    assert_eq!(
        plan.next_record().active_ordinal(),
        ConfigurationGeneration::new(1).unwrap()
    );
    service
        .commit_activation(plan, &Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap())
        .unwrap();
    committed_configuration_generation(&service).unwrap()
}

#[test]
fn controller_owned_name_conflict_skips_only_that_item() {
    assert_conflict(PersistedResourceMetadata::controller());
}

#[test]
fn api_owned_name_conflict_skips_only_that_item() {
    assert_conflict(PersistedResourceMetadata::api());
}

#[test]
fn configuration_owned_same_name_is_reapplied_after_prior_delete_completed() {
    let plan = plan_generation_transition(
        &bundle(),
        committed(),
        &[],
        &BTreeMap::new(),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap();
    assert!(plan.name_conflicts().is_empty());
    assert_eq!(plan.upserts().len(), 2);
}

fn assert_conflict(metadata: PersistedResourceMetadata) {
    let plan = plan_generation_transition(
        &bundle(),
        committed(),
        &[PersistedResourceRecord::new(
            key("Guest", "conflict"),
            metadata,
        )],
        &BTreeMap::new(),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap();
    assert_eq!(plan.name_conflicts().len(), 1);
    assert_eq!(
        plan.name_conflicts()[0].condition(),
        "Degraded/name-conflict"
    );
    assert_eq!(plan.name_conflicts()[0].key(), &key("Volume", "conflict"));
    assert_eq!(plan.upserts().len(), 1);
    assert_eq!(plan.upserts()[0].key(), &key("Network", "main"));
    assert_eq!(
        plan.audits()
            .iter()
            .filter(|audit| matches!(
                audit,
                d2b_core_controller::configuration::generation_transition::GenerationTransitionAudit::ResourceConflictSkipped
            ))
            .count(),
        1
    );
}
