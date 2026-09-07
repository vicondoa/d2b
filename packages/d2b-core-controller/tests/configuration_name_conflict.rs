use std::collections::BTreeMap;

use d2b_contracts_resource::v3::{ConfigurationGeneration, Timestamp, ZoneId};
use d2b_contracts_zone_session::v3::ZoneBundle;
use d2b_core_controller::{
    configuration::{
        ActivationOutcome, BundleResource as PlannedResource, CanonicalSpec, ConfigurationService,
        ResourceBundle, RetainedGenerations,
        generation_transition::{committed_configuration_generation, plan_generation_transition},
    },
    resource_store::{PersistedResourceMetadata, PersistedResourceRecord},
};

mod common;

use common::{input, key};

fn bundle() -> ZoneBundle {
    common::bundle(
        'a',
        vec![
            input("Volume", "conflict", "configured"),
            input("Network", "main", "configured"),
        ],
    )
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
fn owned_name_conflict_skips_only_that_item() {
    for (name, metadata) in [
        ("controller owned", PersistedResourceMetadata::controller()),
        ("api owned", PersistedResourceMetadata::api()),
    ] {
        assert_conflict(name, metadata);
    }
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

#[test]
fn foreign_same_name_with_different_resource_type_does_not_conflict() {
    let plan = plan_generation_transition(
        &bundle(),
        committed(),
        &[PersistedResourceRecord::new(
            key("Guest", "conflict"),
            PersistedResourceMetadata::controller(),
        )],
        &BTreeMap::new(),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap();
    assert!(plan.name_conflicts().is_empty());
    assert_eq!(plan.upserts().len(), 2);
    assert!(
        plan.upserts()
            .iter()
            .any(|upsert| upsert.key() == &key("Volume", "conflict"))
    );
    assert!(
        !plan.audits().iter().any(|audit| matches!(
            audit,
            d2b_core_controller::configuration::generation_transition::GenerationTransitionAudit::ResourceConflictSkipped
        ))
    );
}

fn assert_conflict(row: &str, metadata: PersistedResourceMetadata) {
    let plan = plan_generation_transition(
        &bundle(),
        committed(),
        &[PersistedResourceRecord::new(
            key("Volume", "conflict"),
            metadata,
        )],
        &BTreeMap::new(),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap();
    assert_eq!(plan.name_conflicts().len(), 1, "row: {row}");
    assert_eq!(
        plan.name_conflicts()[0].condition(),
        "Degraded/name-conflict",
        "row: {row}"
    );
    assert_eq!(
        plan.name_conflicts()[0].key(),
        &key("Volume", "conflict"),
        "row: {row}"
    );
    assert_eq!(plan.upserts().len(), 1, "row: {row}");
    assert_eq!(
        plan.upserts()[0].key(),
        &key("Network", "main"),
        "row: {row}"
    );
    assert_eq!(
        plan.audits()
            .iter()
            .filter(|audit| matches!(
                audit,
                d2b_core_controller::configuration::generation_transition::GenerationTransitionAudit::ResourceConflictSkipped
            ))
            .count(),
        1,
        "row: {row}"
    );
}
