use std::collections::BTreeMap;

use d2b_contracts::v3::{
    CanonicalJsonObject, ResourceName, ResourceTypeName, SchemaFingerprint, Timestamp, ZoneId,
};
use d2b_contracts::{BundleMetadata, BundleResource, ZoneBundle};
use d2b_core_controller::{
    configuration::{
        ConfigurationService, RetainedGenerations,
        generation_transition::{
            GenerationTransitionAudit, committed_configuration_generation,
            plan_generation_transition,
        },
    },
    resource_store::{ManagedBy, PersistedResourceMetadata, PersistedResourceRecord},
};

fn digest(byte: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn key(resource_type: &str, name: &str) -> d2b_core_controller::configuration::ResourceKey {
    d2b_core_controller::configuration::ResourceKey::new(
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
        CanonicalJsonObject::parse(br#"{"providerRef":"Provider/network-local"}"#).unwrap(),
    )
    .unwrap()
}

fn bundle(resources: Vec<BundleResource>) -> ZoneBundle {
    ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest('a'),
        resources,
        BTreeMap::from([("network-local".to_owned(), digest('b'))]),
    )
    .unwrap()
}

fn configured(resource_type: &str, name: &str, generation: u64) -> PersistedResourceRecord {
    PersistedResourceRecord::new(
        key(resource_type, name),
        PersistedResourceMetadata::configuration(generation).unwrap(),
    )
}

fn committed_generation(
    incoming: &ZoneBundle,
) -> d2b_core_controller::configuration::generation_transition::CommittedConfigurationGeneration {
    use d2b_core_controller::configuration::{ActivationOutcome, ResourceBundle};

    let mut service = ConfigurationService::empty(
        ZoneId::parse("work").unwrap(),
        RetainedGenerations::default_value(),
    );
    let planned = ResourceBundle::new(
        ZoneId::parse("work").unwrap(),
        incoming.content_hash().clone(),
        Vec::new(),
    )
    .unwrap();
    let activation = match service
        .begin_activation(
            &planned,
            &[],
            &Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
        )
        .unwrap()
    {
        ActivationOutcome::Planned(plan) => plan,
        ActivationOutcome::Unchanged => panic!("new generation must plan"),
    };
    service
        .commit_activation(
            activation,
            &Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
        )
        .unwrap();
    committed_configuration_generation(&service).unwrap()
}

#[test]
fn removed_network_is_scheduled_after_commit_but_controller_guest_is_not() {
    let incoming = bundle(vec![]);
    let controller_guest = PersistedResourceRecord::new(
        key("Guest", "net-vm"),
        PersistedResourceMetadata::controller(),
    );
    let plan = plan_generation_transition(
        &incoming,
        committed_generation(&incoming),
        &[configured("Network", "main", 1), controller_guest],
        &BTreeMap::from([("network-local".to_owned(), digest('b'))]),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap();

    assert_eq!(plan.removals().len(), 1);
    assert_eq!(plan.removals()[0].key(), &key("Network", "main"));
    assert!(
        plan.removals()[0]
            .metadata()
            .deletion_requested_at()
            .is_some()
    );
    assert!(plan.removals()[0].newly_scheduled());
    assert_eq!(
        plan.audits()
            .iter()
            .filter(|audit| matches!(audit, GenerationTransitionAudit::ResourceDeletionScheduled))
            .count(),
        1
    );
}

#[test]
fn redeclared_network_is_not_scheduled_and_receives_committed_metadata() {
    let incoming = bundle(vec![input("Network", "main")]);
    let plan = plan_generation_transition(
        &incoming,
        committed_generation(&incoming),
        &[configured("Network", "main", 1)],
        &BTreeMap::from([("network-local".to_owned(), digest('b'))]),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap();

    assert!(plan.removals().is_empty());
    assert_eq!(plan.upserts().len(), 1);
    assert_eq!(
        plan.upserts()[0].metadata().managed_by(),
        ManagedBy::Configuration
    );
    assert_eq!(
        plan.upserts()[0].metadata().configuration_generation(),
        Some(1)
    );
}

#[test]
fn transition_debug_and_audits_do_not_expose_resource_identity() {
    let incoming = bundle(vec![input("Network", "private-name")]);
    let plan = plan_generation_transition(
        &incoming,
        committed_generation(&incoming),
        &[],
        &BTreeMap::from([("network-local".to_owned(), digest('b'))]),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap();
    let rendered = format!("{plan:?} {:?}", plan.audits());
    assert!(!rendered.contains("private-name"));
    assert!(!rendered.contains("sha256:"));
}

#[test]
fn provider_schema_mismatch_rejects_before_any_plan() {
    let incoming = bundle(vec![input("Network", "main")]);
    let result = plan_generation_transition(
        &incoming,
        committed_generation(&incoming),
        &[],
        &BTreeMap::from([("network-local".to_owned(), digest('c'))]),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    );
    let error = result.unwrap_err();
    assert_eq!(error.label(), "bundle-provider-schema-digest-mismatch");
    assert_eq!(
        error.rejection_audit(),
        Some(GenerationTransitionAudit::BundleRejected)
    );
}

#[test]
fn post_commit_evidence_is_unavailable_before_commit_and_bundle_bound() {
    let service = ConfigurationService::empty(
        ZoneId::parse("work").unwrap(),
        RetainedGenerations::default_value(),
    );
    assert_eq!(
        committed_configuration_generation(&service)
            .unwrap_err()
            .label(),
        "generation-transition-not-committed"
    );

    let committed_bundle = bundle(vec![input("Network", "main")]);
    let different_bundle = bundle(vec![]);
    let error = plan_generation_transition(
        &different_bundle,
        committed_generation(&committed_bundle),
        &[],
        &BTreeMap::from([("network-local".to_owned(), digest('b'))]),
        &Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.label(), "generation-transition-bundle-mismatch");
}
