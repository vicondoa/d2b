use std::collections::BTreeMap;

use d2b_contracts::v3::{
    ResourceTypeName, SchemaFingerprint, SchemaVersion,
    execution_policy::{BoundedToken, ExecutionDomain},
    provider::{
        ArtifactDigest, ComponentDescriptor, ComponentStateKind, ComponentStateNamespace,
        ComponentStateView, ComponentType, StatePlacementMode, StorageNeed,
    },
    volume::ViewRight,
    volume_state::{MigrationPolicy, PersistenceClass, SensitivityClass, VolumeStateSchemaId},
};
use d2b_core_controller::optional_state_admission::{
    ExternalStateObservation, OptionalStateAdmissionError, RestartStateHints,
    StatePayloadAssessment, admit_optional_state, revalidate_after_restart,
};

const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

fn stateless_component() -> ComponentDescriptor {
    ComponentDescriptor::new(
        BoundedToken::parse("controller").unwrap(),
        ComponentType::Controller,
        [ResourceTypeName::parse("Volume").unwrap()],
        [],
        [ExecutionDomain::System],
        1,
        ArtifactDigest::parse(DIGEST).unwrap(),
        [],
        false,
    )
    .unwrap()
}

fn namespace(id: &str, storage_need: StorageNeed) -> ComponentStateNamespace {
    ComponentStateNamespace::new(
        BoundedToken::parse(id).unwrap(),
        ComponentStateKind::State,
        VolumeStateSchemaId::parse(format!("example.d2bus.org/controller/{id}")).unwrap(),
        SchemaVersion::new(1, 0).unwrap(),
        SchemaFingerprint::parse(DIGEST).unwrap(),
        PersistenceClass::Persistent,
        SensitivityClass::Private,
        MigrationPolicy::PreLaunchRequired,
        4096,
        Some(storage_need),
        false,
        Some(StatePlacementMode::GuestLocal),
        false,
        BTreeMap::from([(
            "main".to_owned(),
            ComponentStateView::new(
                String::new(),
                vec![ViewRight::Read, ViewRight::Write, ViewRight::Traverse],
            )
            .unwrap(),
        )]),
    )
    .unwrap()
}

fn stateful_component(storage_needs: &[StorageNeed]) -> ComponentDescriptor {
    stateless_component()
        .with_state_namespaces(
            storage_needs
                .iter()
                .enumerate()
                .map(|(index, need)| namespace(&format!("state-{index}"), *need)),
        )
        .unwrap()
}

#[test]
fn stateless_component_produces_no_volume() {
    let component = stateless_component();
    let admitted = admit_optional_state(&component, &[]).unwrap();
    assert!(admitted.is_empty());
}

#[test]
fn each_storage_need_admits_exactly_its_declared_volume() {
    let needs = [
        StorageNeed::Secret,
        StorageNeed::LargeBinary,
        StorageNeed::PrivateUnsafeForStatus,
        StorageNeed::RevisionUnsuitable,
    ];
    let component = stateful_component(&needs);
    let admitted =
        admit_optional_state(&component, &[StatePayloadAssessment::RequiresStorage; 4]).unwrap();

    assert_eq!(admitted.len(), needs.len());
    for ((volume, namespace), expected_need) in
        admitted.iter().zip(component.state_namespaces()).zip(needs)
    {
        assert!(core::ptr::eq(volume.namespace(), namespace));
        assert_eq!(volume.storage_need(), expected_need);
    }
}

#[test]
fn every_derivable_payload_rejects_the_entire_admission() {
    let component = stateful_component(&[StorageNeed::Secret, StorageNeed::LargeBinary]);
    for derivable in [
        StatePayloadAssessment::DerivableFromSpec,
        StatePayloadAssessment::DerivableFromStatus,
        StatePayloadAssessment::DerivableFromCoreLedger,
        StatePayloadAssessment::DerivableFromExternalObservation,
    ] {
        assert_eq!(
            admit_optional_state(
                &component,
                &[StatePayloadAssessment::RequiresStorage, derivable],
            ),
            Err(OptionalStateAdmissionError::ComponentStateNotJustified)
        );
    }
    assert_eq!(
        OptionalStateAdmissionError::ComponentStateNotJustified.to_string(),
        "component-state-not-justified"
    );
}

#[test]
fn assessment_count_mismatch_fails_closed() {
    let component = stateful_component(&[StorageNeed::Secret]);
    assert_eq!(
        admit_optional_state(&component, &[]),
        Err(OptionalStateAdmissionError::AssessmentCountMismatch)
    );
    assert_eq!(
        admit_optional_state(
            &component,
            &[
                StatePayloadAssessment::RequiresStorage,
                StatePayloadAssessment::RequiresStorage,
            ],
        ),
        Err(OptionalStateAdmissionError::AssessmentCountMismatch)
    );
}

#[test]
fn restart_status_and_ledger_are_hints_not_authority() {
    for status_reported_ready in [false, true] {
        for core_ledger_recorded_completion in [false, true] {
            let hints = RestartStateHints {
                status_reported_ready,
                core_ledger_recorded_completion,
            };
            for observation in [
                ExternalStateObservation::NotReady,
                ExternalStateObservation::Unknown,
            ] {
                let revalidated = revalidate_after_restart(hints, observation);
                assert!(!revalidated.ready());
                assert_eq!(revalidated.prior_hints(), hints);
                assert_eq!(revalidated.observed(), observation);
            }
        }
    }

    let revalidated = revalidate_after_restart(
        RestartStateHints {
            status_reported_ready: false,
            core_ledger_recorded_completion: false,
        },
        ExternalStateObservation::Ready,
    );
    assert!(revalidated.ready());
}
