//! Admission for optional Provider component state Volumes.
//!
//! This module is pure policy. The production store/watch dispatcher is not
//! present yet, so callers receive an admitted plan rather than a store handle
//! or an effect. A later dispatcher must consume this result before it creates
//! a declared Volume or launches the component.

use d2b_contracts::v3::provider::{ComponentDescriptor, ComponentStateNamespace, StorageNeed};

/// Whether one declared payload genuinely requires a state Volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatePayloadAssessment {
    /// The payload cannot be represented by bounded status, the core Operation
    /// ledger, or external observation and requires its declared Volume.
    RequiresStorage,
    /// The payload can be reconstructed from desired resource spec.
    DerivableFromSpec,
    /// The payload can be represented as bounded, non-secret status.
    DerivableFromStatus,
    /// The payload can be reconstructed from the core Operation ledger.
    DerivableFromCoreLedger,
    /// The payload can be reconstructed by observing the external system.
    DerivableFromExternalObservation,
}

impl StatePayloadAssessment {
    const fn requires_storage(self) -> bool {
        matches!(self, Self::RequiresStorage)
    }
}

/// One namespace admitted to produce a state Volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmittedStateVolume<'a> {
    namespace: &'a ComponentStateNamespace,
}

impl<'a> AdmittedStateVolume<'a> {
    /// Borrow the signed namespace declaration projected into the Volume.
    pub const fn namespace(self) -> &'a ComponentStateNamespace {
        self.namespace
    }

    /// Return the signed storage-need class.
    pub const fn storage_need(self) -> StorageNeed {
        self.namespace.storage_need()
    }
}

/// Optional state admission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionalStateAdmissionError {
    /// The trusted assessment set did not correspond one-to-one with the
    /// descriptor's signed namespace declarations.
    AssessmentCountMismatch,
    /// A declared namespace is derivable without private storage.
    ComponentStateNotJustified,
}

impl OptionalStateAdmissionError {
    /// Return the stable, input-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::AssessmentCountMismatch => "component-state-assessment-mismatch",
            Self::ComponentStateNotJustified => "component-state-not-justified",
        }
    }
}

impl core::fmt::Display for OptionalStateAdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for OptionalStateAdmissionError {}

/// Admit exactly the signed namespaces whose payloads require storage.
///
/// A stateless component and an empty assessment set produce no Volumes. Every
/// declared namespace requires one trusted assessment. Any payload derivable
/// from spec, status, the core ledger, or external observation fails the whole
/// admission before a partial plan can escape.
pub fn admit_optional_state<'a>(
    component: &'a ComponentDescriptor,
    assessments: &[StatePayloadAssessment],
) -> Result<Vec<AdmittedStateVolume<'a>>, OptionalStateAdmissionError> {
    let namespaces = component.state_namespaces();
    if namespaces.len() != assessments.len() {
        return Err(OptionalStateAdmissionError::AssessmentCountMismatch);
    }
    if assessments
        .iter()
        .any(|assessment| !assessment.requires_storage())
    {
        return Err(OptionalStateAdmissionError::ComponentStateNotJustified);
    }
    Ok(namespaces
        .iter()
        .map(|namespace| AdmittedStateVolume { namespace })
        .collect())
}

/// Trusted external state observed during restart recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalStateObservation {
    /// External reality verifies the component state is ready.
    Ready,
    /// External reality verifies the component state is absent or not ready.
    NotReady,
    /// External reality could not be established.
    Unknown,
}

/// Non-authoritative restart hints recovered before external revalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartStateHints {
    /// Whether persisted status previously reported readiness.
    pub status_reported_ready: bool,
    /// Whether the core ledger records a completed operation.
    pub core_ledger_recorded_completion: bool,
}

/// Result of status-first restart revalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartStateRevalidation {
    prior_hints: RestartStateHints,
    observed: ExternalStateObservation,
}

impl RestartStateRevalidation {
    /// Return the prior hints retained for deterministic reconciliation.
    pub const fn prior_hints(self) -> RestartStateHints {
        self.prior_hints
    }

    /// Return the externally re-derived observed state.
    pub const fn observed(self) -> ExternalStateObservation {
        self.observed
    }

    /// Whether external reality, not persisted status, proved readiness.
    pub const fn ready(self) -> bool {
        matches!(self.observed, ExternalStateObservation::Ready)
    }
}

/// Revalidate status and ledger hints against external reality after restart.
///
/// Persisted status is retained only as a reconcile hint. It cannot make the
/// result Ready, even when the core ledger also records completion; only a new
/// external observation can do that.
pub const fn revalidate_after_restart(
    hints: RestartStateHints,
    observed: ExternalStateObservation,
) -> RestartStateRevalidation {
    RestartStateRevalidation {
        prior_hints: hints,
        observed,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use d2b_contracts::v3::{
        ResourceTypeName, SchemaFingerprint, SchemaVersion,
        execution_policy::{BoundedToken, ExecutionDomain},
        provider::{
            ArtifactDigest, ComponentStateKind, ComponentStateNamespace, ComponentStateView,
            ComponentType, StatePlacementMode,
        },
        volume::ViewRight,
        volume_state::{MigrationPolicy, PersistenceClass, SensitivityClass, VolumeStateSchemaId},
    };

    use super::*;

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
        assert!(
            admit_optional_state(&stateless_component(), &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn every_storage_need_variant_is_admitted_one_for_one() {
        let needs = [
            StorageNeed::Secret,
            StorageNeed::LargeBinary,
            StorageNeed::PrivateUnsafeForStatus,
            StorageNeed::RevisionUnsuitable,
        ];
        let component = stateful_component(&needs);
        let admitted =
            admit_optional_state(&component, &[StatePayloadAssessment::RequiresStorage; 4])
                .unwrap();
        assert_eq!(admitted.len(), needs.len());
        assert_eq!(
            admitted
                .iter()
                .copied()
                .map(AdmittedStateVolume::storage_need)
                .collect::<Vec<_>>(),
            needs
        );
    }

    #[test]
    fn every_derivable_payload_class_is_rejected_without_a_partial_plan() {
        let component = stateful_component(&[StorageNeed::Secret]);
        for assessment in [
            StatePayloadAssessment::DerivableFromSpec,
            StatePayloadAssessment::DerivableFromStatus,
            StatePayloadAssessment::DerivableFromCoreLedger,
            StatePayloadAssessment::DerivableFromExternalObservation,
        ] {
            assert_eq!(
                admit_optional_state(&component, &[assessment]),
                Err(OptionalStateAdmissionError::ComponentStateNotJustified)
            );
        }
        assert_eq!(
            OptionalStateAdmissionError::ComponentStateNotJustified.code(),
            "component-state-not-justified"
        );
    }

    #[test]
    fn missing_or_extra_assessments_fail_closed() {
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
    fn restart_status_is_a_hint_and_external_observation_is_authority() {
        let optimistic = RestartStateHints {
            status_reported_ready: true,
            core_ledger_recorded_completion: true,
        };
        assert!(!revalidate_after_restart(optimistic, ExternalStateObservation::NotReady).ready());
        assert!(!revalidate_after_restart(optimistic, ExternalStateObservation::Unknown).ready());
        assert!(
            revalidate_after_restart(
                RestartStateHints {
                    status_reported_ready: false,
                    core_ledger_recorded_completion: false,
                },
                ExternalStateObservation::Ready,
            )
            .ready()
        );
    }
}
