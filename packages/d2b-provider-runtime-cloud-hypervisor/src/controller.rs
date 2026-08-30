//! Resource-first Cloud Hypervisor Guest reconciliation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, DesiredLifecycle, ResourceGeneration, ResourcePhase, ResourceRef,
    ResourceTypeName, ResourceUid, SchemaFingerprint, ZoneId, ZoneRevision,
};
use d2b_core_controller::{HintTarget, ObservedChild, OwnerIndex, OwnerLimits};

use crate::{
    bootstrap_graph::{BootstrapGraph, DependencyReadiness, GuestChildGraphPlan},
    descriptor::{
        GuestSetupDescriptor, GuestSetupDescriptorError, GuestSetupDescriptorVerifier,
        VerifiedGuestSetupDescriptor,
    },
    health::{GuestSessionEvidence, GuestSessionHealth},
    identity::{
        ChildCreateBody, ChildMutation, ChildRole, ChildRoleSet, CommittedChild, GuestChildBatch,
        PrivateRuntimeScope, derive_private_runtime_scope,
    },
    state::{
        GuestGenerationSet, GuestRuntimeStatus, GuestStatusObservation, GuestStatusPhase,
        reduce_status,
    },
};

/// The finalizer owned by the Cloud Hypervisor Guest controller.
pub const GUEST_CONTROLLER_FINALIZER: &str = "runtime-cloud-hypervisor.d2bus.org/guest";

/// Errors returned by the authenticated Resource API seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudHypervisorResourceApiError {
    /// The authenticated session was unavailable.
    Authentication,
    /// The Resource API transport failed.
    Transport,
    /// The target did not exist.
    NotFound,
    /// A UID or revision precondition conflicted.
    Conflict,
    /// The response could not be trusted as complete.
    Uncertain,
    /// A bounded response was truncated.
    Truncated,
    /// The response type did not match the request.
    InvalidResponse,
}

impl CloudHypervisorResourceApiError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Authentication => "cloud-hypervisor-resource-authentication",
            Self::Transport => "cloud-hypervisor-resource-transport",
            Self::NotFound => "cloud-hypervisor-resource-not-found",
            Self::Conflict => "cloud-hypervisor-resource-conflict",
            Self::Uncertain => "cloud-hypervisor-resource-uncertain",
            Self::Truncated => "cloud-hypervisor-resource-truncated",
            Self::InvalidResponse => "cloud-hypervisor-resource-invalid-response",
        }
    }
}

impl fmt::Display for CloudHypervisorResourceApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for CloudHypervisorResourceApiError {}

/// Cloud Hypervisor controller failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudHypervisorError {
    /// The controller configuration was invalid.
    InvalidConfiguration,
    /// The private descriptor did not verify.
    Descriptor(GuestSetupDescriptorError),
    /// The controller has not been registered with an authenticated API.
    NotRegistered,
    /// The Guest snapshot did not match the verified Provider contract.
    InvalidGuest,
    /// A child had a foreign or stale owner identity.
    ChildConflict,
    /// The child batch response was incomplete or malformed.
    BatchResponseInvalid,
    /// The authenticated Resource API failed.
    ResourceApi(CloudHypervisorResourceApiError),
}

impl fmt::Display for CloudHypervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration => {
                formatter.write_str("cloud-hypervisor-invalid-configuration")
            }
            Self::Descriptor(error) => error.fmt(formatter),
            Self::NotRegistered => {
                formatter.write_str("cloud-hypervisor-controller-not-registered")
            }
            Self::InvalidGuest => formatter.write_str("cloud-hypervisor-guest-invalid"),
            Self::ChildConflict => formatter.write_str("cloud-hypervisor-child-conflict"),
            Self::BatchResponseInvalid => {
                formatter.write_str("cloud-hypervisor-batch-response-invalid")
            }
            Self::ResourceApi(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CloudHypervisorError {}

impl From<GuestSetupDescriptorError> for CloudHypervisorError {
    fn from(error: GuestSetupDescriptorError) -> Self {
        Self::Descriptor(error)
    }
}

impl From<CloudHypervisorResourceApiError> for CloudHypervisorError {
    fn from(error: CloudHypervisorResourceApiError) -> Self {
        Self::ResourceApi(error)
    }
}

/// The verified controller registration and its bounded watch set.
#[derive(Clone, PartialEq, Eq)]
pub struct CloudHypervisorControllerRegistration {
    provider_ref: ResourceRef,
    provider_generation: ResourceGeneration,
    descriptor_digest: SchemaFingerprint,
    child_roles: ChildRoleSet,
    watched_types: Vec<ResourceTypeName>,
    dependency_types: Vec<ResourceTypeName>,
    finalizer: String,
}

impl CloudHypervisorControllerRegistration {
    /// Build the registration from a verified private descriptor.
    pub fn from_verified_descriptor(
        descriptor: &VerifiedGuestSetupDescriptor,
    ) -> Result<Self, CloudHypervisorError> {
        let provider_ref = ResourceRef::parse(crate::PROVIDER_REF)
            .map_err(|_| CloudHypervisorError::InvalidConfiguration)?;
        if descriptor.descriptor().provider_ref() != &provider_ref
            || !descriptor.descriptor().child_roles().is_fixed()
        {
            return Err(CloudHypervisorError::InvalidConfiguration);
        }
        let watched_types = ["Guest", "Process", "Endpoint", "Volume"]
            .into_iter()
            .map(|value| {
                ResourceTypeName::parse(value)
                    .map_err(|_| CloudHypervisorError::InvalidConfiguration)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dependency_types = ["Device", "Network"]
            .into_iter()
            .map(|value| {
                ResourceTypeName::parse(value)
                    .map_err(|_| CloudHypervisorError::InvalidConfiguration)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            provider_ref,
            provider_generation: descriptor.descriptor().provider_generation(),
            descriptor_digest: descriptor.descriptor().descriptor_digest().clone(),
            child_roles: descriptor.descriptor().child_roles().clone(),
            watched_types,
            dependency_types,
            finalizer: GUEST_CONTROLLER_FINALIZER.to_owned(),
        })
    }

    /// Borrow the Provider identity.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Return the Provider generation bound into registration.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Borrow the verified descriptor digest bound into registration.
    pub const fn descriptor_digest(&self) -> &SchemaFingerprint {
        &self.descriptor_digest
    }

    /// Borrow the direct child role set.
    pub const fn child_roles(&self) -> &ChildRoleSet {
        &self.child_roles
    }

    /// Borrow the ResourceTypes watched by this controller.
    pub fn watched_types(&self) -> &[ResourceTypeName] {
        &self.watched_types
    }

    /// Borrow the dependency ResourceTypes watched by this controller.
    pub fn dependency_types(&self) -> &[ResourceTypeName] {
        &self.dependency_types
    }

    /// Borrow the controller finalizer ID.
    pub fn finalizer(&self) -> &str {
        &self.finalizer
    }
}

impl fmt::Debug for CloudHypervisorControllerRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudHypervisorControllerRegistration")
            .field("provider_ref", &self.provider_ref)
            .field("provider_generation", &self.provider_generation)
            .field("child_role_count", &self.child_roles.iter().count())
            .field("watched_type_count", &self.watched_types.len())
            .field("dependency_type_count", &self.dependency_types.len())
            .field("has_finalizer", &true)
            .finish()
    }
}

/// A fresh Guest snapshot read through the authenticated Resource API.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestSnapshot {
    zone: ZoneId,
    zone_uid: ResourceUid,
    resource_ref: ResourceRef,
    uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
    execution_ref: ResourceRef,
    provider_ref: ResourceRef,
    system_artifact_id: Option<String>,
    generations: GuestGenerationSet,
    session_evidence: Option<GuestSessionEvidence>,
    deleting: bool,
}

impl GuestSnapshot {
    /// Construct a validated Guest snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZoneId,
        zone_uid: ResourceUid,
        resource_ref: ResourceRef,
        uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
        execution_ref: ResourceRef,
        provider_ref: ResourceRef,
        system_artifact_id: Option<String>,
        generations: GuestGenerationSet,
        deleting: bool,
    ) -> Result<Self, CloudHypervisorError> {
        if resource_ref.resource_type().as_str() != "Guest"
            || execution_ref.resource_type().as_str() != "Host"
            || provider_ref.resource_type().as_str() != "Provider"
            || generation.get() == 0
            || revision.get() == 0
            || system_artifact_id
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 63)
        {
            return Err(CloudHypervisorError::InvalidGuest);
        }
        Ok(Self {
            zone,
            zone_uid,
            resource_ref,
            uid,
            generation,
            revision,
            execution_ref,
            provider_ref,
            system_artifact_id,
            generations,
            session_evidence: None,
            deleting,
        })
    }

    /// Attach the latest bounded authenticated session evidence.
    pub fn with_session_evidence(mut self, evidence: GuestSessionEvidence) -> Self {
        self.session_evidence = Some(evidence);
        self
    }

    /// Borrow the Guest Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the immutable Zone UID used only for private runtime fencing.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Borrow the Guest ResourceRef.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the store-assigned Guest UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the Guest desired-state generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Return the Guest revision used for update fencing.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Borrow the semantic execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the selected Provider.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the selected public system artifact ID.
    pub fn system_artifact_id(&self) -> Option<&str> {
        self.system_artifact_id.as_deref()
    }

    /// Borrow the generation observation consumed by the pure status reducer.
    pub const fn generations(&self) -> GuestGenerationSet {
        self.generations
    }

    /// Borrow the latest authenticated session evidence.
    pub fn session_evidence(&self) -> Option<&GuestSessionEvidence> {
        self.session_evidence.as_ref()
    }

    /// Whether deletion has been requested.
    pub const fn deleting(&self) -> bool {
        self.deleting
    }
}

impl fmt::Debug for GuestSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestSnapshot")
            .field("resource_type", &self.resource_ref.resource_type())
            .field("has_zone", &true)
            .field("has_uid", &true)
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .field("has_execution_ref", &true)
            .field("has_provider_ref", &true)
            .field("has_system_artifact_id", &self.system_artifact_id.is_some())
            .field("deleting", &self.deleting)
            .finish()
    }
}

/// A complete owner-index row for one direct child.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnedChildSnapshot {
    resource_ref: ResourceRef,
    zone: ZoneId,
    owner_ref: ResourceRef,
    owner_uid: Option<ResourceUid>,
    uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
    spec_digest: String,
    phase: ResourcePhase,
    desired_lifecycle: Option<DesiredLifecycle>,
    healthy: bool,
}

impl OwnedChildSnapshot {
    /// Construct one observed child row.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resource_ref: ResourceRef,
        zone: ZoneId,
        owner_ref: ResourceRef,
        uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
        spec_digest: impl Into<String>,
        phase: ResourcePhase,
        desired_lifecycle: Option<DesiredLifecycle>,
        healthy: bool,
    ) -> Result<Self, CloudHypervisorError> {
        let spec_digest = spec_digest.into();
        if !matches!(
            resource_ref.resource_type().as_str(),
            "Process" | "Endpoint" | "Volume"
        ) || owner_ref.resource_type().as_str() != "Guest"
            || generation.get() == 0
            || revision.get() == 0
            || spec_digest.is_empty()
            || resource_ref.resource_type().as_str() != "Process" && desired_lifecycle.is_some()
        {
            return Err(CloudHypervisorError::ChildConflict);
        }
        if spec_digest.len() > 128 {
            return Err(CloudHypervisorError::ChildConflict);
        }
        Ok(Self {
            resource_ref,
            zone,
            owner_ref,
            owner_uid: None,
            uid,
            generation,
            revision,
            spec_digest,
            phase,
            desired_lifecycle,
            healthy,
        })
    }

    /// Attach the exact Guest owner UID from the Resource envelope.
    pub fn with_owner_uid(mut self, owner_uid: ResourceUid) -> Self {
        self.owner_uid = Some(owner_uid);
        self
    }

    /// Override the observed Process desired lifecycle after a successful
    /// optimistic update.
    pub fn with_desired_lifecycle(mut self, desired: DesiredLifecycle) -> Self {
        self.desired_lifecycle = Some(desired);
        self
    }

    /// Borrow the child ResourceRef.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the child Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the singular owner ResourceRef.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the exact owner UID, when supplied by the Resource API.
    pub fn owner_uid(&self) -> Option<&ResourceUid> {
        self.owner_uid.as_ref()
    }

    /// Borrow the child UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the child generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Return the child revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Borrow the semantic desired-state digest.
    pub fn spec_digest(&self) -> &str {
        &self.spec_digest
    }

    /// Return the universal lifecycle phase.
    pub const fn phase(&self) -> ResourcePhase {
        self.phase
    }

    /// Return the Process desired lifecycle, when this is a Process.
    pub const fn desired_lifecycle(&self) -> Option<DesiredLifecycle> {
        self.desired_lifecycle
    }

    /// Whether the child status is healthy.
    pub const fn healthy(&self) -> bool {
        self.healthy
    }
}

impl fmt::Debug for OwnedChildSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedChildSnapshot")
            .field("resource_type", &self.resource_ref.resource_type())
            .field("has_zone", &true)
            .field("has_owner", &true)
            .field("has_owner_uid", &self.owner_uid.is_some())
            .field("has_uid", &true)
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .field("phase", &self.phase)
            .field("has_spec_digest", &true)
            .finish()
    }
}

/// Bounded Device, Network, and Volume dependency status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestDependencySnapshot {
    devices: Vec<(ResourceRef, ResourcePhase)>,
    networks: Vec<(ResourceRef, ResourcePhase)>,
    volumes: Vec<(ResourceRef, ResourcePhase)>,
    exports_ready: bool,
    setup_ready: bool,
}

impl GuestDependencySnapshot {
    /// Construct and validate dependency status rows.
    pub fn new(
        devices: Vec<(ResourceRef, ResourcePhase)>,
        networks: Vec<(ResourceRef, ResourcePhase)>,
        volumes: Vec<(ResourceRef, ResourcePhase)>,
        exports_ready: bool,
        setup_ready: bool,
    ) -> Result<Self, CloudHypervisorError> {
        validate_dependency_family(&devices, "Device")?;
        validate_dependency_family(&networks, "Network")?;
        validate_dependency_family(&volumes, "Volume")?;
        let mut refs = BTreeSet::new();
        for (reference, _) in devices.iter().chain(&networks).chain(&volumes) {
            if !refs.insert(reference.clone()) {
                return Err(CloudHypervisorError::InvalidGuest);
            }
        }
        Ok(Self {
            devices,
            networks,
            volumes,
            exports_ready,
            setup_ready,
        })
    }

    /// Construct all-ready dependency evidence for a graph.
    pub fn ready(graph: BootstrapGraph) -> Self {
        Self {
            devices: graph
                .devices
                .into_iter()
                .map(|reference| (reference, ResourcePhase::Ready))
                .collect(),
            networks: graph
                .networks
                .into_iter()
                .map(|reference| (reference, ResourcePhase::Ready))
                .collect(),
            volumes: graph
                .volumes
                .into_iter()
                .map(|reference| (reference, ResourcePhase::Ready))
                .collect(),
            exports_ready: true,
            setup_ready: true,
        }
    }

    /// Return the Device dependency readiness.
    pub fn devices_ready(&self, graph: &BootstrapGraph) -> bool {
        all_family_ready(&graph.devices, &self.devices)
    }

    /// Return the Network dependency readiness.
    pub fn networks_ready(&self, graph: &BootstrapGraph) -> bool {
        all_family_ready(&graph.networks, &self.networks)
    }

    /// Return the backing Volume dependency readiness.
    pub fn volumes_ready(&self, graph: &BootstrapGraph) -> bool {
        all_family_ready(&graph.volumes, &self.volumes)
    }

    /// Return whether all required Volume Exports are Ready.
    pub const fn exports_ready(&self) -> bool {
        self.exports_ready
    }

    /// Return whether all descriptor-declared setup Volumes are Ready.
    pub const fn setup_ready(&self) -> bool {
        self.setup_ready
    }

    fn readiness(&self, graph: &BootstrapGraph) -> (DependencyReadiness, Vec<GuestCondition>) {
        let devices_ready = self.devices_ready(graph);
        let networks_ready = self.networks_ready(graph);
        let volumes_ready = self.volumes_ready(graph);
        let eligibility = graph.vmm_lifecycle(
            devices_ready,
            networks_ready,
            volumes_ready,
            self.exports_ready,
            self.setup_ready,
        );
        let mut conditions = Vec::new();
        if !devices_ready {
            conditions.push(GuestCondition::DeviceDependencyNotReady);
        }
        if !networks_ready {
            conditions.push(GuestCondition::NetworkDependencyNotReady);
        }
        if !volumes_ready {
            conditions.push(GuestCondition::VolumeDependencyNotReady);
        }
        if !self.exports_ready {
            conditions.push(GuestCondition::ExportDependencyNotReady);
        }
        if !self.setup_ready {
            conditions.push(GuestCondition::SetupVolumeNotReady);
        }
        (
            if eligibility.is_running() {
                DependencyReadiness::Ready
            } else {
                DependencyReadiness::Pending
            },
            conditions,
        )
    }
}

fn validate_dependency_family(
    rows: &[(ResourceRef, ResourcePhase)],
    expected_type: &str,
) -> Result<(), CloudHypervisorError> {
    let mut refs = BTreeSet::new();
    if rows.iter().any(|(reference, _)| {
        reference.resource_type().as_str() != expected_type || !refs.insert(reference.clone())
    }) {
        return Err(CloudHypervisorError::InvalidGuest);
    }
    Ok(())
}

fn all_family_ready(expected: &[ResourceRef], observed: &[(ResourceRef, ResourcePhase)]) -> bool {
    expected.iter().all(|reference| {
        observed
            .iter()
            .find(|(observed_ref, _)| observed_ref == reference)
            .is_some_and(|(_, phase)| *phase == ResourcePhase::Ready)
    })
}

/// One bounded UID-free child CommitBatch request.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestChildCreateBatch {
    zone: ZoneId,
    owner_ref: ResourceRef,
    owner_uid: ResourceUid,
    owner_revision: ZoneRevision,
    source: GuestChildBatch,
    mutations: Vec<ChildMutation>,
}

impl GuestChildCreateBatch {
    /// Select missing deterministic children from the complete pure plan.
    pub fn new(
        guest: &GuestSnapshot,
        source: &GuestChildBatch,
        missing: impl IntoIterator<Item = ResourceRef>,
    ) -> Result<Self, CloudHypervisorError> {
        if source.zone() != guest.zone() || source.owner_ref() != guest.resource_ref() {
            return Err(CloudHypervisorError::ChildConflict);
        }
        let expected = source
            .mutations()
            .iter()
            .map(|mutation| mutation.target().clone())
            .collect::<BTreeSet<_>>();
        let missing = missing.into_iter().collect::<BTreeSet<_>>();
        if missing.is_empty()
            || missing.len() > 128
            || missing.iter().any(|target| !expected.contains(target))
        {
            return Err(CloudHypervisorError::BatchResponseInvalid);
        }
        let mutations = source
            .mutations()
            .iter()
            .filter(|mutation| missing.contains(mutation.target()))
            .cloned()
            .collect::<Vec<_>>();
        if mutations.len() != missing.len() {
            return Err(CloudHypervisorError::BatchResponseInvalid);
        }
        Ok(Self {
            zone: guest.zone.clone(),
            owner_ref: guest.resource_ref.clone(),
            owner_uid: guest.uid.clone(),
            owner_revision: guest.revision,
            source: source.clone(),
            mutations,
        })
    }

    /// Borrow the batch Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the Guest owner ResourceRef.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the exact Guest UID fence.
    pub const fn owner_uid(&self) -> &ResourceUid {
        &self.owner_uid
    }

    /// Return the exact Guest revision fence.
    pub const fn owner_revision(&self) -> ZoneRevision {
        self.owner_revision
    }

    /// Borrow the complete pure child batch used as the source.
    pub const fn source(&self) -> &GuestChildBatch {
        &self.source
    }

    /// Borrow only the missing UID-free mutations submitted to the API.
    pub fn mutations(&self) -> &[ChildMutation] {
        &self.mutations
    }

    /// Return the canonical desired digest for one child.
    pub fn desired_digest(&self, target: &ResourceRef) -> Result<String, CloudHypervisorError> {
        let mutation = self
            .source
            .mutations()
            .iter()
            .find(|mutation| mutation.target() == target)
            .ok_or(CloudHypervisorError::BatchResponseInvalid)?;
        let payload = materialize_child_payload(mutation)?;
        d2b_core_controller::semantic_child_digest(&payload)
            .map_err(|_| CloudHypervisorError::BatchResponseInvalid)
    }

    /// Return the canonical UID-free Resource payload for one child.
    pub fn canonical_payload(&self, target: &ResourceRef) -> Result<Vec<u8>, CloudHypervisorError> {
        let mutation = self
            .mutations
            .iter()
            .find(|mutation| mutation.target() == target)
            .ok_or(CloudHypervisorError::BatchResponseInvalid)?;
        materialize_child_payload(mutation)
    }
}

impl fmt::Debug for GuestChildCreateBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestChildCreateBatch")
            .field("child_count", &self.mutations.len())
            .field("has_owner_uid", &true)
            .field("has_owner_revision", &true)
            .finish()
    }
}

/// One exact UID/revision-fenced UpdateSpec request.
#[derive(Clone, PartialEq, Eq)]
pub struct ChildSpecUpdate {
    target: ResourceRef,
    expected_uid: ResourceUid,
    expected_revision: ZoneRevision,
    body: ChildCreateBody,
    desired_lifecycle: Option<DesiredLifecycle>,
}

impl ChildSpecUpdate {
    /// Construct one exact child spec update.
    pub fn new(
        target: ResourceRef,
        expected_uid: ResourceUid,
        expected_revision: ZoneRevision,
        body: ChildCreateBody,
        desired_lifecycle: Option<DesiredLifecycle>,
    ) -> Result<Self, CloudHypervisorError> {
        if expected_revision.get() == 0
            || target.resource_type().as_str()
                != match body {
                    ChildCreateBody::Process(_) => "Process",
                    ChildCreateBody::Endpoint(_) => "Endpoint",
                    ChildCreateBody::Volume(_) => "Volume",
                }
            || target.resource_type().as_str() != "Process" && desired_lifecycle.is_some()
        {
            return Err(CloudHypervisorError::ChildConflict);
        }
        Ok(Self {
            target,
            expected_uid,
            expected_revision,
            body,
            desired_lifecycle,
        })
    }

    /// Borrow the updated child ResourceRef.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Borrow the exact UID precondition.
    pub const fn expected_uid(&self) -> &ResourceUid {
        &self.expected_uid
    }

    /// Return the exact revision precondition.
    pub const fn expected_revision(&self) -> ZoneRevision {
        self.expected_revision
    }

    /// Borrow the semantic replacement body.
    pub const fn body(&self) -> &ChildCreateBody {
        &self.body
    }

    /// Return the requested Process lifecycle, when present.
    pub const fn desired_lifecycle(&self) -> Option<DesiredLifecycle> {
        self.desired_lifecycle
    }
}

impl fmt::Debug for ChildSpecUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildSpecUpdate")
            .field("resource_type", &self.target.resource_type())
            .field("has_expected_uid", &true)
            .field("expected_revision", &self.expected_revision)
            .field("has_desired_lifecycle", &self.desired_lifecycle.is_some())
            .finish()
    }
}

/// Bounded Guest status conditions aggregated from base children and
/// dependency states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GuestCondition {
    /// A deterministic direct child is absent.
    ChildMissing(ChildRole),
    /// A deterministic direct child is not Ready.
    ChildNotReady(ChildRole),
    /// A deterministic direct child reported unhealthy.
    ChildUnhealthy(ChildRole),
    /// The Device dependency family is not Ready.
    DeviceDependencyNotReady,
    /// The Network dependency family is not Ready.
    NetworkDependencyNotReady,
    /// The backing Volume dependency family is not Ready.
    VolumeDependencyNotReady,
    /// A required Volume Export is not Ready.
    ExportDependencyNotReady,
    /// A descriptor-declared setup Volume is not Ready.
    SetupVolumeNotReady,
    /// The VMM desired lifecycle is still stopped.
    ProcessStopped,
    /// The authenticated Guest session is not Ready.
    SessionNotReady,
    /// The authenticated Guest session is degraded.
    SessionDegraded,
}

/// Public Guest status plus only bounded base conditions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestStatusProjection {
    status: GuestRuntimeStatus,
    conditions: Vec<GuestCondition>,
}

impl GuestStatusProjection {
    /// Construct a bounded status projection.
    fn new(status: GuestRuntimeStatus, mut conditions: Vec<GuestCondition>) -> Self {
        conditions.sort();
        conditions.dedup();
        conditions.truncate(32);
        Self { status, conditions }
    }

    /// Borrow the pure public Guest status.
    pub const fn status(&self) -> &GuestRuntimeStatus {
        &self.status
    }

    /// Borrow bounded status conditions.
    pub fn conditions(&self) -> &[GuestCondition] {
        &self.conditions
    }
}

/// Resource API requests emitted by the authenticated adapter.
#[derive(Clone, PartialEq, Eq)]
pub enum CloudHypervisorResourceRequest {
    /// Register the verified controller descriptor.
    Register {
        /// Controller registration.
        registration: CloudHypervisorControllerRegistration,
    },
    /// Read a fresh Guest snapshot.
    GetGuest {
        /// Guest ResourceRef.
        guest_ref: ResourceRef,
    },
    /// Relist the complete owner-index view.
    RelistOwnedChildren {
        /// Guest owner.
        guest_ref: ResourceRef,
        /// Expected direct child addresses.
        expected_refs: Vec<ResourceRef>,
    },
    /// Read Device, Network, and Volume dependency status.
    ObserveDependencies {
        /// Guest owner.
        guest_ref: ResourceRef,
        /// Pure dependency graph.
        graph: BootstrapGraph,
    },
    /// Create missing direct children atomically.
    CommitBatch {
        /// UID-free bounded batch.
        batch: GuestChildCreateBatch,
    },
    /// Update one child spec under exact identity preconditions.
    UpdateSpec {
        /// Fenced update.
        update: ChildSpecUpdate,
    },
    /// Persist the bounded Guest status projection.
    UpdateStatus {
        /// Guest owner.
        guest_ref: ResourceRef,
        /// Status candidate.
        status: GuestStatusProjection,
    },
}

impl fmt::Debug for CloudHypervisorResourceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Register { .. } => "CloudHypervisorResourceRequest::Register",
            Self::GetGuest { .. } => "CloudHypervisorResourceRequest::GetGuest",
            Self::RelistOwnedChildren { .. } => {
                "CloudHypervisorResourceRequest::RelistOwnedChildren"
            }
            Self::ObserveDependencies { .. } => {
                "CloudHypervisorResourceRequest::ObserveDependencies"
            }
            Self::CommitBatch { .. } => "CloudHypervisorResourceRequest::CommitBatch",
            Self::UpdateSpec { .. } => "CloudHypervisorResourceRequest::UpdateSpec",
            Self::UpdateStatus { .. } => "CloudHypervisorResourceRequest::UpdateStatus",
        })
    }
}

/// Resource API responses returned by an authenticated session.
#[derive(Clone, PartialEq, Eq)]
pub enum CloudHypervisorResourceResponse {
    /// Registration succeeded.
    Registered,
    /// A fresh Guest snapshot.
    Guest(GuestSnapshot),
    /// Complete owner-index rows.
    OwnedChildren(Vec<OwnedChildSnapshot>),
    /// Dependency status.
    Dependencies(GuestDependencySnapshot),
    /// CommitBatch result.
    Committed(GuestChildCommitResponse),
    /// UpdateSpec result.
    Updated(CommittedChild),
    /// UpdateStatus succeeded.
    StatusUpdated,
}

impl fmt::Debug for CloudHypervisorResourceResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Registered => "CloudHypervisorResourceResponse::Registered",
            Self::Guest(_) => "CloudHypervisorResourceResponse::Guest",
            Self::OwnedChildren(_) => "CloudHypervisorResourceResponse::OwnedChildren",
            Self::Dependencies(_) => "CloudHypervisorResourceResponse::Dependencies",
            Self::Committed(_) => "CloudHypervisorResourceResponse::Committed",
            Self::Updated(_) => "CloudHypervisorResourceResponse::Updated",
            Self::StatusUpdated => "CloudHypervisorResourceResponse::StatusUpdated",
        })
    }
}

/// A session that is already authenticated and route-pinned by its owner.
#[async_trait]
pub trait AuthenticatedResourceSession: Send + Sync {
    /// Send one bounded Resource API request.
    async fn call(
        &self,
        request: CloudHypervisorResourceRequest,
    ) -> Result<CloudHypervisorResourceResponse, CloudHypervisorResourceApiError>;
}

/// Adapter from an authenticated session to the typed Guest controller API.
pub struct AuthenticatedResourceApiAdapter<S> {
    session: Arc<S>,
}

impl<S> AuthenticatedResourceApiAdapter<S> {
    /// Bind the adapter to an already authenticated session.
    pub const fn new(session: Arc<S>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl<S> CloudHypervisorResourceApi for AuthenticatedResourceApiAdapter<S>
where
    S: AuthenticatedResourceSession + 'static,
{
    async fn register(
        &self,
        registration: &CloudHypervisorControllerRegistration,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        match self
            .session
            .call(CloudHypervisorResourceRequest::Register {
                registration: registration.clone(),
            })
            .await?
        {
            CloudHypervisorResourceResponse::Registered => Ok(()),
            _ => Err(CloudHypervisorResourceApiError::InvalidResponse),
        }
    }

    async fn get_guest(
        &self,
        guest_ref: &ResourceRef,
    ) -> Result<GuestSnapshot, CloudHypervisorResourceApiError> {
        match self
            .session
            .call(CloudHypervisorResourceRequest::GetGuest {
                guest_ref: guest_ref.clone(),
            })
            .await?
        {
            CloudHypervisorResourceResponse::Guest(guest) => Ok(guest),
            _ => Err(CloudHypervisorResourceApiError::InvalidResponse),
        }
    }

    async fn relist_owned_children(
        &self,
        guest: &GuestSnapshot,
        expected_refs: &[ResourceRef],
    ) -> Result<Vec<OwnedChildSnapshot>, CloudHypervisorResourceApiError> {
        match self
            .session
            .call(CloudHypervisorResourceRequest::RelistOwnedChildren {
                guest_ref: guest.resource_ref.clone(),
                expected_refs: expected_refs.to_vec(),
            })
            .await?
        {
            CloudHypervisorResourceResponse::OwnedChildren(children) => Ok(children),
            _ => Err(CloudHypervisorResourceApiError::InvalidResponse),
        }
    }

    async fn observe_dependencies(
        &self,
        guest: &GuestSnapshot,
        graph: &BootstrapGraph,
    ) -> Result<GuestDependencySnapshot, CloudHypervisorResourceApiError> {
        match self
            .session
            .call(CloudHypervisorResourceRequest::ObserveDependencies {
                guest_ref: guest.resource_ref.clone(),
                graph: graph.clone(),
            })
            .await?
        {
            CloudHypervisorResourceResponse::Dependencies(dependencies) => Ok(dependencies),
            _ => Err(CloudHypervisorResourceApiError::InvalidResponse),
        }
    }

    async fn commit_batch(
        &self,
        batch: GuestChildCreateBatch,
    ) -> Result<GuestChildCommitResponse, CloudHypervisorResourceApiError> {
        match self
            .session
            .call(CloudHypervisorResourceRequest::CommitBatch { batch })
            .await?
        {
            CloudHypervisorResourceResponse::Committed(result) => Ok(result),
            _ => Err(CloudHypervisorResourceApiError::InvalidResponse),
        }
    }

    async fn update_spec(
        &self,
        update: ChildSpecUpdate,
    ) -> Result<CommittedChild, CloudHypervisorResourceApiError> {
        match self
            .session
            .call(CloudHypervisorResourceRequest::UpdateSpec { update })
            .await?
        {
            CloudHypervisorResourceResponse::Updated(child) => Ok(child),
            _ => Err(CloudHypervisorResourceApiError::InvalidResponse),
        }
    }

    async fn update_status(
        &self,
        guest: &GuestSnapshot,
        status: GuestStatusProjection,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        match self
            .session
            .call(CloudHypervisorResourceRequest::UpdateStatus {
                guest_ref: guest.resource_ref.clone(),
                status,
            })
            .await?
        {
            CloudHypervisorResourceResponse::StatusUpdated => Ok(()),
            _ => Err(CloudHypervisorResourceApiError::InvalidResponse),
        }
    }
}

/// Typed authenticated Resource API operations required by the Guest
/// controller.
#[async_trait]
pub trait CloudHypervisorResourceApi: Send + Sync {
    /// Register the controller descriptor.
    async fn register(
        &self,
        registration: &CloudHypervisorControllerRegistration,
    ) -> Result<(), CloudHypervisorResourceApiError>;

    /// Read a fresh Guest snapshot.
    async fn get_guest(
        &self,
        guest_ref: &ResourceRef,
    ) -> Result<GuestSnapshot, CloudHypervisorResourceApiError>;

    /// Replace the complete owner-index relist.
    async fn relist_owned_children(
        &self,
        guest: &GuestSnapshot,
        expected_refs: &[ResourceRef],
    ) -> Result<Vec<OwnedChildSnapshot>, CloudHypervisorResourceApiError>;

    /// Observe all dependency status needed by the pure bootstrap graph.
    async fn observe_dependencies(
        &self,
        guest: &GuestSnapshot,
        graph: &BootstrapGraph,
    ) -> Result<GuestDependencySnapshot, CloudHypervisorResourceApiError>;

    /// Commit missing direct children atomically.
    async fn commit_batch(
        &self,
        batch: GuestChildCreateBatch,
    ) -> Result<GuestChildCommitResponse, CloudHypervisorResourceApiError>;

    /// Update one child spec under exact UID/revision fencing.
    async fn update_spec(
        &self,
        update: ChildSpecUpdate,
    ) -> Result<CommittedChild, CloudHypervisorResourceApiError>;

    /// Persist the bounded Guest status projection.
    async fn update_status(
        &self,
        guest: &GuestSnapshot,
        status: GuestStatusProjection,
    ) -> Result<(), CloudHypervisorResourceApiError>;
}

/// Result of one direct-child CommitBatch call.
#[derive(Clone, PartialEq, Eq)]
pub enum GuestChildCommitResponse {
    /// Complete identities returned by the Resource API.
    Committed(Vec<CommittedChild>),
    /// The transport outcome is unknown and requires relisting.
    Uncertain,
    /// The response was bounded but truncated.
    Truncated,
}

impl fmt::Debug for GuestChildCommitResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Committed(_) => "GuestChildCommitResponse::Committed",
            Self::Uncertain => "GuestChildCommitResponse::Uncertain",
            Self::Truncated => "GuestChildCommitResponse::Truncated",
        })
    }
}

/// Result of one Resource API reconcile pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudHypervisorReconcileOutcome {
    /// The bounded status remains pending and should be retried.
    Pending(GuestStatusProjection),
    /// The child batch response requires an authoritative relist.
    RelistRequired(GuestStatusProjection),
    /// The Guest status is ready.
    Ready(GuestStatusProjection),
    /// The Guest status is degraded after prior readiness.
    Degraded(GuestStatusProjection),
}

impl CloudHypervisorReconcileOutcome {
    /// Whether the pass leaves the Guest pending.
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_) | Self::RelistRequired(_))
    }

    /// Borrow the bounded status projection.
    pub const fn status(&self) -> &GuestStatusProjection {
        match self {
            Self::Pending(status)
            | Self::RelistRequired(status)
            | Self::Ready(status)
            | Self::Degraded(status) => status,
        }
    }

    fn from_status(status: GuestStatusProjection, relist_required: bool) -> Self {
        if relist_required {
            return Self::RelistRequired(status);
        }
        match status.status.phase {
            GuestStatusPhase::Ready => Self::Ready(status),
            GuestStatusPhase::Degraded => Self::Degraded(status),
            GuestStatusPhase::Pending | GuestStatusPhase::Draining => Self::Pending(status),
        }
    }
}

/// Cloud Hypervisor Guest controller.
pub struct CloudHypervisorController<A> {
    _config: crate::CloudHypervisorConfig,
    graph: BootstrapGraph,
    descriptor: VerifiedGuestSetupDescriptor,
    registration: CloudHypervisorControllerRegistration,
    api: Arc<A>,
    registered: bool,
}

impl<A> CloudHypervisorController<A>
where
    A: CloudHypervisorResourceApi + 'static,
{
    /// Construct a controller from a verified descriptor without performing a
    /// Resource API call.
    pub fn from_verified_descriptor(
        config: crate::CloudHypervisorConfig,
        graph: BootstrapGraph,
        descriptor: VerifiedGuestSetupDescriptor,
        api: Arc<A>,
    ) -> Result<Self, CloudHypervisorError> {
        config
            .validate()
            .map_err(|_| CloudHypervisorError::InvalidConfiguration)?;
        let registration =
            CloudHypervisorControllerRegistration::from_verified_descriptor(&descriptor)?;
        Ok(Self {
            _config: config,
            graph,
            descriptor,
            registration,
            api,
            registered: false,
        })
    }

    /// Verify a raw descriptor before binding any authenticated API.
    pub fn from_descriptor(
        config: crate::CloudHypervisorConfig,
        graph: BootstrapGraph,
        descriptor: GuestSetupDescriptor,
        verifier: &impl GuestSetupDescriptorVerifier,
        api: Arc<A>,
    ) -> Result<Self, CloudHypervisorError> {
        let descriptor = descriptor.verify_with(verifier)?;
        Self::from_verified_descriptor(config, graph, descriptor, api)
    }

    /// Borrow the verified controller registration.
    pub const fn registration(&self) -> &CloudHypervisorControllerRegistration {
        &self.registration
    }

    /// Borrow the verified setup descriptor.
    pub const fn descriptor(&self) -> &VerifiedGuestSetupDescriptor {
        &self.descriptor
    }

    /// Register the controller on its authenticated Resource API session.
    pub async fn register(&mut self) -> Result<(), CloudHypervisorError> {
        if self.registered {
            return Ok(());
        }
        self.api.register(&self.registration).await?;
        self.registered = true;
        Ok(())
    }

    /// Derive a private UID-based runtime scope without exposing it to the
    /// Resource API.
    pub fn private_runtime_scope(
        &self,
        guest: &GuestSnapshot,
        role: &str,
    ) -> Result<PrivateRuntimeScope, CloudHypervisorError> {
        derive_private_runtime_scope(
            guest.zone_uid(),
            guest.uid(),
            role,
            self.descriptor.descriptor().provider_generation(),
        )
        .map_err(|_| CloudHypervisorError::InvalidGuest)
    }

    /// Reconcile one Guest from a fresh snapshot and complete owner relist.
    pub async fn reconcile(
        &self,
        guest_ref: &ResourceRef,
    ) -> Result<CloudHypervisorReconcileOutcome, CloudHypervisorError> {
        if !self.registered {
            return Err(CloudHypervisorError::NotRegistered);
        }
        let guest = self.api.get_guest(guest_ref).await?;
        self.validate_guest(&guest, guest_ref)?;
        let child_plan = BootstrapGraph::plan_children(
            guest.zone.clone(),
            guest.resource_ref.clone(),
            guest.execution_ref.clone(),
            &self.descriptor,
        )
        .map_err(|_| CloudHypervisorError::InvalidConfiguration)?;
        let expected_refs = child_plan
            .child_batch()
            .mutations()
            .iter()
            .map(|mutation| mutation.target().clone())
            .collect::<Vec<_>>();
        let observed = self
            .api
            .relist_owned_children(&guest, &expected_refs)
            .await?;
        let children = self.validate_owner_relist(&guest, &expected_refs, observed)?;
        let dependencies = self.api.observe_dependencies(&guest, &self.graph).await?;
        let (dependency_readiness, dependency_conditions) = dependencies.readiness(&self.graph);

        if guest.deleting() {
            let status = self.project_status(
                &guest,
                &child_plan,
                &children,
                dependency_readiness,
                dependency_conditions,
            );
            self.api.update_status(&guest, status.clone()).await?;
            return Ok(CloudHypervisorReconcileOutcome::from_status(status, false));
        }

        let missing = expected_refs
            .iter()
            .filter(|target| !children.contains_key(*target))
            .cloned()
            .collect::<Vec<_>>();
        let mut committed = BTreeMap::new();
        if !missing.is_empty() {
            let batch = GuestChildCreateBatch::new(&guest, child_plan.child_batch(), missing)?;
            let response = match self.api.commit_batch(batch.clone()).await {
                Ok(response) => response,
                Err(error)
                    if matches!(
                        error,
                        CloudHypervisorResourceApiError::Conflict
                            | CloudHypervisorResourceApiError::Uncertain
                            | CloudHypervisorResourceApiError::Truncated
                    ) =>
                {
                    return self
                        .pending_after_batch(
                            &guest,
                            &child_plan,
                            &children,
                            dependency_readiness,
                            dependency_conditions,
                            true,
                        )
                        .await;
                }
                Err(error) => return Err(error.into()),
            };
            match response {
                GuestChildCommitResponse::Committed(returned) => {
                    committed = match validate_commit_response(&batch, returned) {
                        Ok(committed) => committed,
                        Err(CloudHypervisorError::BatchResponseInvalid) => {
                            return self
                                .pending_after_batch(
                                    &guest,
                                    &child_plan,
                                    &children,
                                    dependency_readiness,
                                    dependency_conditions,
                                    true,
                                )
                                .await;
                        }
                        Err(error) => return Err(error),
                    };
                }
                GuestChildCommitResponse::Uncertain | GuestChildCommitResponse::Truncated => {
                    return self
                        .pending_after_batch(
                            &guest,
                            &child_plan,
                            &children,
                            dependency_readiness,
                            dependency_conditions,
                            true,
                        )
                        .await;
                }
            }
        }

        let desired_lifecycle = if dependency_readiness == DependencyReadiness::Ready {
            DesiredLifecycle::Running
        } else {
            DesiredLifecycle::Stopped
        };
        if let Err(error) = self
            .repair_children(
                child_plan.child_batch(),
                &children,
                &committed,
                guest.generation(),
                desired_lifecycle,
            )
            .await
        {
            if error == CloudHypervisorError::ResourceApi(CloudHypervisorResourceApiError::Conflict)
            {
                let status = self.project_status(
                    &guest,
                    &child_plan,
                    &children,
                    dependency_readiness,
                    dependency_conditions,
                );
                self.api.update_status(&guest, status.clone()).await?;
                return Ok(CloudHypervisorReconcileOutcome::from_status(status, false));
            }
            return Err(error);
        }

        let status = self.project_status(
            &guest,
            &child_plan,
            &children,
            dependency_readiness,
            dependency_conditions,
        );
        self.api.update_status(&guest, status.clone()).await?;
        Ok(CloudHypervisorReconcileOutcome::from_status(status, false))
    }

    fn validate_guest(
        &self,
        guest: &GuestSnapshot,
        requested_ref: &ResourceRef,
    ) -> Result<(), CloudHypervisorError> {
        if guest.resource_ref() != requested_ref
            || guest.provider_ref() != self.registration.provider_ref()
            || guest.system_artifact_id()
                != Some(self.descriptor.descriptor().system_artifact_id().as_str())
        {
            return Err(CloudHypervisorError::InvalidGuest);
        }
        Ok(())
    }

    async fn pending_after_batch(
        &self,
        guest: &GuestSnapshot,
        plan: &GuestChildGraphPlan,
        children: &BTreeMap<ResourceRef, OwnedChildSnapshot>,
        dependency_readiness: DependencyReadiness,
        conditions: Vec<GuestCondition>,
        relist_required: bool,
    ) -> Result<CloudHypervisorReconcileOutcome, CloudHypervisorError> {
        let status = self.project_status(guest, plan, children, dependency_readiness, conditions);
        self.api.update_status(guest, status.clone()).await?;
        Ok(CloudHypervisorReconcileOutcome::from_status(
            status,
            relist_required,
        ))
    }

    fn validate_owner_relist(
        &self,
        guest: &GuestSnapshot,
        expected_refs: &[ResourceRef],
        observed: Vec<OwnedChildSnapshot>,
    ) -> Result<BTreeMap<ResourceRef, OwnedChildSnapshot>, CloudHypervisorError> {
        let expected = expected_refs.iter().collect::<BTreeSet<_>>();
        let mut children = BTreeMap::new();
        for child in observed {
            if child.zone() != guest.zone()
                || child.owner_ref() != guest.resource_ref()
                || child.owner_uid() != Some(guest.uid())
            {
                return Err(CloudHypervisorError::ChildConflict);
            }
            if !expected.contains(child.resource_ref()) {
                return Err(CloudHypervisorError::ChildConflict);
            }
            if children
                .insert(child.resource_ref().clone(), child)
                .is_some()
            {
                return Err(CloudHypervisorError::ChildConflict);
            }
        }
        let owner = HintTarget::new(
            guest.zone.clone(),
            guest.resource_ref.clone(),
            guest.uid.clone(),
        );
        let indexed = children
            .values()
            .map(|child| {
                let target = HintTarget::new(
                    child.zone.clone(),
                    child.resource_ref.clone(),
                    child.uid.clone(),
                );
                ObservedChild::with_owner_identity(
                    target,
                    guest.resource_ref.clone(),
                    guest.uid.clone(),
                    guest.generation,
                    child.revision,
                    child.spec_digest.clone(),
                    false,
                    false,
                    std::iter::empty(),
                )
                .map(|observed| observed.with_generation(child.generation))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| CloudHypervisorError::ChildConflict)?;
        let mut owner_index =
            OwnerIndex::new(OwnerLimits::new(8, 128).expect("fixed owner limits"));
        owner_index
            .relist_with_owner_generation(owner, guest.generation, indexed)
            .map_err(|_| CloudHypervisorError::ChildConflict)?;
        Ok(children)
    }

    async fn repair_children(
        &self,
        batch: &GuestChildBatch,
        observed: &BTreeMap<ResourceRef, OwnedChildSnapshot>,
        committed: &BTreeMap<ResourceRef, CommittedChild>,
        expected_generation: ResourceGeneration,
        desired_lifecycle: DesiredLifecycle,
    ) -> Result<(), CloudHypervisorError> {
        for mutation in batch.mutations() {
            let target = mutation.target();
            let Some(child) = observed.get(target) else {
                if target.resource_type().as_str() == "Process"
                    && desired_lifecycle == DesiredLifecycle::Running
                    && let Some(identity) = committed.get(target)
                {
                    let update = ChildSpecUpdate::new(
                        target.clone(),
                        identity.uid().clone(),
                        identity.revision(),
                        mutation.body().clone(),
                        Some(desired_lifecycle),
                    )?;
                    self.api.update_spec(update).await?;
                }
                continue;
            };
            let desired_digest =
                d2b_core_controller::semantic_child_digest(&materialize_child_payload(mutation)?)
                    .map_err(|_| CloudHypervisorError::BatchResponseInvalid)?;
            let lifecycle_drift = target.resource_type().as_str() == "Process"
                && child.desired_lifecycle() != Some(desired_lifecycle);
            let generation_drift = child.generation() != expected_generation;
            if child.spec_digest() != desired_digest || lifecycle_drift || generation_drift {
                let update = ChildSpecUpdate::new(
                    target.clone(),
                    child.uid().clone(),
                    child.revision(),
                    mutation.body().clone(),
                    (target.resource_type().as_str() == "Process").then_some(desired_lifecycle),
                )?;
                self.api.update_spec(update).await?;
            }
        }
        Ok(())
    }

    fn project_status(
        &self,
        guest: &GuestSnapshot,
        plan: &GuestChildGraphPlan,
        children: &BTreeMap<ResourceRef, OwnedChildSnapshot>,
        dependency_readiness: DependencyReadiness,
        mut conditions: Vec<GuestCondition>,
    ) -> GuestStatusProjection {
        for mutation in plan.child_batch().mutations() {
            let target = mutation.target();
            let role = [
                ChildRole::VmmProcess,
                ChildRole::ChApiEndpoint,
                ChildRole::GuestControlEndpoint,
                ChildRole::SystemVolume,
            ]
            .into_iter()
            .find(|role| plan.child_batch().child_ref(*role) == Some(target))
            .expect("fixed child plan role");
            match children.get(target) {
                None => conditions.push(GuestCondition::ChildMissing(role)),
                Some(child) => {
                    if child.phase() != ResourcePhase::Ready
                        || child.generation() != guest.generation()
                    {
                        conditions.push(GuestCondition::ChildNotReady(role));
                    }
                    if !child.healthy() {
                        conditions.push(GuestCondition::ChildUnhealthy(role));
                    }
                }
            }
        }
        let process = plan
            .child_batch()
            .child_ref(ChildRole::VmmProcess)
            .and_then(|target| children.get(target));
        if process.is_none_or(|child| child.desired_lifecycle() != Some(DesiredLifecycle::Running))
        {
            conditions.push(GuestCondition::ProcessStopped);
        }
        let endpoint_ready = [ChildRole::ChApiEndpoint, ChildRole::GuestControlEndpoint]
            .into_iter()
            .all(|role| {
                plan.child_batch()
                    .child_ref(role)
                    .and_then(|target| children.get(target))
                    .is_some_and(|child| child.phase() == ResourcePhase::Ready)
            });
        let process_ready = process.is_some_and(|child| child.phase() == ResourcePhase::Ready);
        let session_observed = guest.session_evidence().is_some();
        let session_evidence = guest
            .session_evidence()
            .cloned()
            .unwrap_or_else(GuestSessionEvidence::failed);
        let session_ready =
            session_observed && session_evidence.health() == GuestSessionHealth::Ready;
        let session_healthy =
            session_observed && session_evidence.health() == GuestSessionHealth::Ready;
        match guest.session_evidence().map(GuestSessionEvidence::health) {
            None => conditions.push(GuestCondition::SessionNotReady),
            Some(GuestSessionHealth::Ready) => {}
            Some(_) => conditions.push(GuestCondition::SessionDegraded),
        }
        let child_healthy = plan.child_batch().mutations().iter().all(|mutation| {
            children
                .get(mutation.target())
                .is_some_and(|child| child.healthy() && child.generation() == guest.generation())
        });
        let observation = GuestStatusObservation {
            generations: guest.generations(),
            dependencies_ready: dependency_readiness == DependencyReadiness::Ready,
            process_ready,
            endpoint_ready,
            session_ready,
            seed_ready: session_observed && session_evidence.seed_ready(),
            session_healthy,
            required_children_healthy: child_healthy,
            deletion_requested: guest.deleting(),
            session_active: session_observed,
            descendants_present: !children.is_empty(),
            process_stopped: process
                .is_none_or(|child| child.desired_lifecycle() != Some(DesiredLifecycle::Running)),
        };
        let status = reduce_status(&observation);
        GuestStatusProjection::new(status, conditions)
    }
}

impl<A> fmt::Debug for CloudHypervisorController<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudHypervisorController")
            .field("registration", &self.registration)
            .field("registered", &self.registered)
            .finish()
    }
}

fn validate_commit_response(
    batch: &GuestChildCreateBatch,
    returned: Vec<CommittedChild>,
) -> Result<BTreeMap<ResourceRef, CommittedChild>, CloudHypervisorError> {
    let expected = batch
        .mutations()
        .iter()
        .map(|mutation| mutation.target().clone())
        .collect::<BTreeSet<_>>();
    if returned.len() != expected.len() {
        return Err(CloudHypervisorError::BatchResponseInvalid);
    }
    let mut seen_refs = BTreeSet::new();
    let mut seen_uids = BTreeSet::new();
    let mut mapped = BTreeMap::new();
    for child in returned {
        if !expected.contains(child.resource_ref())
            || child.zone() != batch.zone()
            || child.owner_ref() != batch.owner_ref()
            || !seen_refs.insert(child.resource_ref().clone())
            || !seen_uids.insert(child.uid().clone())
        {
            return Err(CloudHypervisorError::BatchResponseInvalid);
        }
        mapped.insert(child.resource_ref().clone(), child);
    }
    Ok(mapped)
}

fn materialize_child_payload(mutation: &ChildMutation) -> Result<Vec<u8>, CloudHypervisorError> {
    let body = serde_json::to_value(mutation.body())
        .map_err(|_| CloudHypervisorError::BatchResponseInvalid)?;
    let spec = body
        .get("spec")
        .cloned()
        .ok_or(CloudHypervisorError::BatchResponseInvalid)?;
    let value = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": mutation.target().resource_type().as_str(),
        "metadata": {
            "name": mutation.target().name().as_str(),
            "zone": mutation.zone().as_str(),
            "ownerRef": mutation.owner_ref().to_canonical_string(),
            "finalizers": [],
            "deletionRequestedAt": null,
            "createdAt": "1970-01-01T00:00:00.000Z",
            "updatedAt": "1970-01-01T00:00:00.000Z",
            "generation": 1,
            "revision": 1,
            "managedBy": "controller"
        },
        "spec": spec,
        "status": {
            "observedGeneration": 0,
            "phase": "Pending",
            "conditions": [],
            "lastReconciledAt": null,
            "startedAt": null,
            "completedAt": null,
            "outcome": null,
            "update": {
                "dependencies": {"count": 0, "refs": []},
                "disruption": "None",
                "lastAssessedAt": null,
                "observedGeneration": 0,
                "operationId": null,
                "owned": {"count": 0, "refs": []},
                "preserveState": true,
                "reasons": [],
                "state": "Unknown",
                "targetGeneration": 1
            },
            "resource": {}
        }
    });
    let bytes =
        serde_json::to_vec(&value).map_err(|_| CloudHypervisorError::BatchResponseInvalid)?;
    let canonical = CanonicalJsonValue::parse(&bytes)
        .map_err(|_| CloudHypervisorError::BatchResponseInvalid)?
        .to_canonical_bytes();
    Ok(canonical)
}
