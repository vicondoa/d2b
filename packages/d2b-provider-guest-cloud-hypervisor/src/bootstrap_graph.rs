//! Dependency-gated VMM bootstrap graph.

use std::fmt;

use d2b_contracts_resource::v3::{
    BindingKind, BindingLifecycleState, BindingObservation, ChildBindingRequest,
    ChildRequestDefaults, ChildSupportCeiling, ReleaseOutcome, RequestedRights, ResourceRef,
    ResourceUid, VolumeBindingRequest, ZoneId,
};
use d2b_core_controller::OwnedChildKind;

use crate::{
    adoption::BindingAdoptionStatus,
    descriptor::VerifiedGuestSetupDescriptor,
    execution_parent::GuestExecutionParent,
    identity::GuestChildBatch,
};

/// Readiness of one dependency family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyReadiness {
    /// All required effects are ready.
    Ready,
    /// At least one dependency is still pending.
    Pending,
}

/// Pure VMM lifecycle eligibility derived from dependency readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmLifecycleEligibility {
    /// Keep the VMM Process stopped.
    Stopped,
    /// Permit the VMM Process to transition to running.
    Running,
}

impl VmmLifecycleEligibility {
    /// Return whether the VMM Process may transition to running.
    pub const fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// Closed failures while constructing or planning a Guest bootstrap graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapGraphError {
    /// An attachment ticket was empty, oversized, or contained non-printable bytes.
    InvalidAttachmentRef,
    /// A direct dependency named a Host reference, which a Guest graph never permits.
    HostReferenceNotAllowed,
    /// The verified setup descriptor did not project a direct-child batch.
    InvalidSetupDescriptor,
}

impl fmt::Display for BootstrapGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidAttachmentRef => "bootstrap-graph-invalid-attachment-ref",
            Self::HostReferenceNotAllowed => "bootstrap-graph-host-reference-not-allowed",
            Self::InvalidSetupDescriptor => "bootstrap-graph-invalid-setup-descriptor",
        })
    }
}

impl std::error::Error for BootstrapGraphError {}

/// Opaque VMM attachment reference.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttachmentRef(String);

impl AttachmentRef {
    /// Construct a bounded opaque attachment ref.
    pub fn new(value: impl Into<String>) -> Result<Self, BootstrapGraphError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(BootstrapGraphError::InvalidAttachmentRef);
        }
        Ok(Self(value))
    }
}

impl fmt::Debug for AttachmentRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AttachmentRef(<opaque>)")
    }
}

/// The dependency snapshot required before VMM launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapGraph {
    /// Device references.
    pub devices: Vec<ResourceRef>,
    /// Network references.
    pub networks: Vec<ResourceRef>,
    /// Virtiofs volume references.
    pub volumes: Vec<ResourceRef>,
    /// VolumeBinding references whose fenced readiness gates VMM start.
    pub bindings: Vec<ResourceRef>,
    /// Opaque attachment tickets resolved by Core.
    pub attachments: Vec<AttachmentRef>,
}

impl BootstrapGraph {
    /// Plan the deterministic, UID-free direct children for one Guest.
    pub fn plan_children(
        zone: ZoneId,
        guest_ref: ResourceRef,
        execution_ref: ResourceRef,
        descriptor: &VerifiedGuestSetupDescriptor,
    ) -> Result<GuestChildGraphPlan, BootstrapGraphError> {
        GuestChildGraphPlan::from_descriptor(zone, guest_ref, execution_ref, descriptor)
    }

    /// Construct and validate the explicit KVM rule.
    pub fn new(
        devices: Vec<ResourceRef>,
        networks: Vec<ResourceRef>,
        volumes: Vec<ResourceRef>,
        bindings: Vec<ResourceRef>,
        attachments: Vec<AttachmentRef>,
    ) -> Result<Self, BootstrapGraphError> {
        if devices
            .iter()
            .chain(networks.iter())
            .chain(volumes.iter())
            .chain(bindings.iter())
            .any(|reference| reference.resource_type().as_str() == "Host")
        {
            return Err(BootstrapGraphError::HostReferenceNotAllowed);
        }
        Ok(Self {
            devices,
            networks,
            volumes,
            bindings,
            attachments,
        })
    }

    /// Check all pre-start dependencies without performing an effect.
    pub fn vmm_readiness(&self, snapshot: VmmReadinessSnapshot) -> DependencyReadiness {
        if snapshot.all_ready() {
            DependencyReadiness::Ready
        } else {
            DependencyReadiness::Pending
        }
    }

    /// Return the pure VMM lifecycle decision for a dependency snapshot.
    pub fn vmm_lifecycle(&self, snapshot: VmmReadinessSnapshot) -> VmmLifecycleEligibility {
        match self.vmm_readiness(snapshot) {
            DependencyReadiness::Ready => VmmLifecycleEligibility::Running,
            DependencyReadiness::Pending => VmmLifecycleEligibility::Stopped,
        }
    }
}

/// Immutable readiness facts gating VMM start.
///
/// Carried as one struct so a swapped argument cannot silently change the
/// start gate; the facts are produced by `GuestDependencySnapshot` accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmmReadinessSnapshot {
    /// Device family readiness.
    pub devices_ready: bool,
    /// Network family readiness.
    pub networks_ready: bool,
    /// Volume family readiness.
    pub volumes_ready: bool,
    /// VolumeBinding family readiness under the current fence.
    pub bindings_ready: bool,
    /// Descriptor setup-volume readiness.
    pub setup_ready: bool,
}

impl VmmReadinessSnapshot {
    /// Return whether every gating fact is ready.
    pub const fn all_ready(self) -> bool {
        self.devices_ready
            && self.networks_ready
            && self.volumes_ready
            && self.bindings_ready
            && self.setup_ready
    }
}

/// Deterministic direct-child plan for one Cloud Hypervisor Guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestChildGraphPlan {
    batch: GuestChildBatch,
    creation_order: Vec<ResourceRef>,
    deletion_order: Vec<ResourceRef>,
}

impl GuestChildGraphPlan {
    /// Construct the direct-child plan from a verified setup descriptor.
    pub fn from_descriptor(
        zone: ZoneId,
        guest_ref: ResourceRef,
        execution_ref: ResourceRef,
        descriptor: &VerifiedGuestSetupDescriptor,
    ) -> Result<Self, BootstrapGraphError> {
        let batch = GuestChildBatch::from_descriptor(zone, guest_ref, execution_ref, descriptor)
            .map_err(|_| BootstrapGraphError::InvalidSetupDescriptor)?;
        let mut creation_order = child_refs(&batch);
        creation_order.sort_by_key(|target| {
            (
                OwnedChildKind::from_resource_ref(target).creation_rank(),
                target.clone(),
            )
        });
        let mut deletion_order = child_refs(&batch);
        deletion_order.sort_by_key(|target| {
            (
                OwnedChildKind::from_resource_ref(target).deletion_rank(),
                target.clone(),
            )
        });
        Ok(Self {
            batch,
            creation_order,
            deletion_order,
        })
    }

    /// Borrow the UID-free direct-child batch.
    pub const fn child_batch(&self) -> &GuestChildBatch {
        &self.batch
    }

    /// Borrow the Core-compatible dependency-first creation order.
    pub fn creation_order(&self) -> &[ResourceRef] {
        &self.creation_order
    }

    /// Borrow the Core-compatible dependent-first deletion order.
    pub fn deletion_order(&self) -> &[ResourceRef] {
        &self.deletion_order
    }
}

fn child_refs(batch: &GuestChildBatch) -> Vec<ResourceRef> {
    batch
        .mutations()
        .iter()
        .map(|mutation| mutation.target().clone())
        .collect()
}

/// One admitted relationship the Guest itself consumes (AE32).
///
/// The request is the Guest's own desired declaration, and the evidence is
/// the observation of the relationship the source provider admitted for it.
/// Both halves are fenced to exact identities, so an ownership, view,
/// consumer, or source-replacement change cannot be satisfied by a cached
/// readiness value (R35, R41).
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedGuestBinding {
    request: VolumeBindingRequest,
    source_uid: Option<ResourceUid>,
    observation: Option<BindingObservation>,
}

impl AdmittedGuestBinding {
    /// Construct one relationship with no observation yet.
    fn new(request: VolumeBindingRequest) -> Self {
        Self {
            request,
            source_uid: None,
            observation: None,
        }
    }

    /// Borrow the Guest's own desired request.
    pub const fn request(&self) -> &VolumeBindingRequest {
        &self.request
    }

    /// Borrow the source identity the latest observation was made under.
    pub const fn source_uid(&self) -> Option<&ResourceUid> {
        self.source_uid.as_ref()
    }

    /// Borrow the latest fenced observation.
    pub const fn observation(&self) -> Option<&BindingObservation> {
        self.observation.as_ref()
    }

    /// Whether the source side is prepared under the current fence.
    ///
    /// This is the pre-start condition alone. It never folds in the
    /// consumer's completion, because a Guest that has not booted cannot have
    /// mounted and treating that as a failure would make its own start gate
    /// wait on itself (R39).
    pub fn source_prepared(&self) -> bool {
        self.observation
            .is_some_and(|observation| observation.prepare().is_complete())
    }

    /// Whether the consumer side has completed under the current fence.
    pub fn consumer_complete(&self) -> bool {
        self.observation
            .is_some_and(|observation| observation.consumer_completion().is_complete())
    }

    /// Whether no outstanding use of this relationship remains.
    pub fn released(&self) -> bool {
        self.observation
            .is_some_and(|observation| matches!(observation.release(), ReleaseOutcome::Released))
    }
}

/// Why the admitted graph refused to record or accept one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmittedGraphError {
    /// The observed relationship is not one the Guest consumes.
    ///
    /// A support ceiling and a child default are inputs to someone else's
    /// decision, so neither can be recorded here: there is no path by which
    /// they become Guest use.
    UnknownRelationship,
    /// The execution-parent fragment could not be classified.
    ExecutionParent,
}

impl fmt::Display for AdmittedGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownRelationship => "guest-graph-unknown-relationship",
            Self::ExecutionParent => "guest-graph-execution-parent-unclassified",
        })
    }
}

impl std::error::Error for AdmittedGraphError {}

/// The Guest's pre-start condition over its admitted relationships (R39).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestStartGate {
    /// Every relationship the Guest consumes has its source prepared, so the
    /// VMM Process may transition to running. Consumer completion follows
    /// the boot rather than gating it (R40).
    Permitted,
    /// At least one source is still preparing.
    SourcePending,
    /// At least one relationship refused under its own identity.
    Refused,
}

impl GuestStartGate {
    /// Whether the VMM Process may transition to running.
    pub const fn permits_start(self) -> bool {
        matches!(self, Self::Permitted)
    }
}

/// The consumer-side completion of the Guest's admitted relationships.
///
/// It is observed only after boot, and it is reported rather than waited on:
/// the two conditions stay separate so storage preparation never forms a
/// startup cycle with the mount the Guest makes after it starts (AE21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestConsumerCompletion {
    /// The Guest has not started, so no mount exists to observe yet.
    NotYetRunning,
    /// Every observed consumer side completed.
    Complete,
    /// The consumer is running and reports at least one relationship did not
    /// complete. The start gate is unaffected; this is a post-boot
    /// regression, not a pre-boot condition.
    Incomplete,
}

/// The admitted graph backing one Cloud Hypervisor Guest.
///
/// This is the composition the graph contract describes, and it replaces the
/// flattened attachment lists as the source of the start gate:
///
/// - the child target-support ceiling bounds what a child of this Guest may
///   request and contributes no member, no wait, and no access (AE31);
/// - the Guest's own consumption is one admitted relationship per request,
///   each with the Guest as its consumer (AE32);
/// - a child default shapes one named child's request and never becomes a
///   Guest relationship or a Guest boot dependency (AE33).
///
/// Nothing here is an independently authored policy table. Every field is
/// either a classified input or an observation of an admitted relationship,
/// so the launch decision cannot be made by a list the provider authored.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedGuestGraph {
    guest_ref: ResourceRef,
    support: ChildSupportCeiling,
    bindings: Vec<AdmittedGuestBinding>,
    child_defaults: Vec<ChildRequestDefaults>,
}

impl AdmittedGuestGraph {
    /// Compose the graph from one classified execution parent.
    ///
    /// # Errors
    ///
    /// Refuses a fragment with no ceiling or with a support entry the binding
    /// kinds do not admit. An absent ceiling would be a target that admits
    /// anything, which is the opposite of what an empty attachment list
    /// means.
    pub fn from_execution_parent(
        guest_ref: ResourceRef,
        parent: &GuestExecutionParent,
    ) -> Result<Self, AdmittedGraphError> {
        let support = parent
            .support_ceiling()
            .ok_or(AdmittedGraphError::ExecutionParent)?
            .clone();
        Ok(Self {
            bindings: parent
                .parent_use()
                .cloned()
                .map(AdmittedGuestBinding::new)
                .collect(),
            guest_ref,
            support,
            child_defaults: parent.child_defaults().cloned().collect(),
        })
    }

    /// Borrow the Guest this graph describes.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the child target-support ceiling.
    pub const fn support_ceiling(&self) -> &ChildSupportCeiling {
        &self.support
    }

    /// Borrow the relationships the Guest itself consumes.
    pub fn guest_bindings(&self) -> &[AdmittedGuestBinding] {
        &self.bindings
    }

    /// Borrow the defaults supplied to named children.
    pub fn child_defaults(&self) -> &[ChildRequestDefaults] {
        &self.child_defaults
    }

    /// Record one observation of a relationship the Guest consumes.
    ///
    /// The caller decides whether the observed row still describes this
    /// relationship; this method only records what survived that decision.
    /// A row under a different source identity is a successor relationship
    /// rather than a continuation, so the earlier source's readiness is
    /// dropped instead of carried over (R19, R35).
    ///
    /// # Errors
    ///
    /// Refuses a relationship this Guest does not consume, which is the
    /// structural half of AE31 and AE33: a ceiling entry and a child default
    /// are not relationships and can never reach this method.
    pub fn observe_binding(
        &mut self,
        request: &VolumeBindingRequest,
        source_uid: &ResourceUid,
        observation: BindingObservation,
    ) -> Result<BindingAdoptionStatus, AdmittedGraphError> {
        if request.consumer_ref() != &self.guest_ref {
            return Err(AdmittedGraphError::UnknownRelationship);
        }
        let Some(binding) = self
            .bindings
            .iter_mut()
            .find(|binding| binding.request() == request)
        else {
            return Err(AdmittedGraphError::UnknownRelationship);
        };
        let status = if binding.source_uid.as_ref() == Some(source_uid) {
            if binding.observation.is_some() {
                BindingAdoptionStatus::Current
            } else {
                BindingAdoptionStatus::Adopted
            }
        } else {
            BindingAdoptionStatus::Adopted
        };
        binding.source_uid = Some(source_uid.clone());
        binding.observation = Some(observation);
        Ok(status)
    }

    /// The Guest's pre-start condition over every relationship it consumes.
    ///
    /// The gate reads source preparation alone. A Guest whose storage export
    /// is prepared but not yet mounted may start, and the mount that follows
    /// is observed afterwards rather than waited for here (AE6, AE21).
    pub fn start_gate(&self) -> GuestStartGate {
        let mut pending = false;
        for binding in &self.bindings {
            match binding.observation() {
                None => pending = true,
                Some(observation) => {
                    if matches!(observation.state(), BindingLifecycleState::Refused) {
                        return GuestStartGate::Refused;
                    }
                    if !observation.prepare().is_complete() {
                        pending = true;
                    }
                }
            }
        }
        if pending {
            GuestStartGate::SourcePending
        } else {
            GuestStartGate::Permitted
        }
    }

    /// The consumer-side completion that follows the permitted start.
    pub fn consumer_completion(&self) -> GuestConsumerCompletion {
        if self
            .bindings
            .iter()
            .any(|binding| !binding.source_prepared())
        {
            return GuestConsumerCompletion::NotYetRunning;
        }
        if self
            .bindings
            .iter()
            .all(AdmittedGuestBinding::consumer_complete)
        {
            GuestConsumerCompletion::Complete
        } else {
            GuestConsumerCompletion::Incomplete
        }
    }

    /// Whether any relationship still holds outstanding use.
    ///
    /// This is the last fact a stop has to settle: descendants are drained
    /// and the Guest's own use is released before the Guest's finalizer may
    /// be cleared (R36).
    pub fn use_outstanding(&self) -> bool {
        self.bindings.iter().any(|binding| !binding.released())
    }

    /// Shape one child's request with this Guest's defaults (AE33) and
    /// check it against the child's support ceiling (AE31).
    ///
    /// A request the ceiling does not admit is refused: the ceiling is the
    /// constraint, and applying the defaults never widens what a child may
    /// ask for.
    pub fn shape_child_request(
        &self,
        draft: &ChildBindingRequest,
    ) -> Result<ChildBindingRequest, AdmittedGraphError> {
        let shaped = match self
            .child_defaults
            .iter()
            .find(|defaults| defaults.child_ref() == draft.consumer_ref())
        {
            Some(defaults) => draft.apply_defaults(defaults),
            None => Ok(draft.clone()),
        }
        .map_err(|_| AdmittedGraphError::ExecutionParent)?;
        let Some(rights) = shaped.rights() else {
            return Ok(shaped);
        };
        if !self.support.admits(shaped.kind(), rights) {
            return Err(AdmittedGraphError::ExecutionParent);
        }
        Ok(shaped)
    }

    /// The ceiling's verdict on one child request, for callers that already
    /// shaped the request.
    pub fn admits_child_request(&self, kind: BindingKind, rights: RequestedRights) -> bool {
        self.support.admits(kind, rights)
    }
}

impl fmt::Debug for AdmittedGuestGraph {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedGuestGraph")
            .field("guest_ref", &self.guest_ref)
            .field("support_entries", &self.support.entries().len())
            .field("guest_bindings", &self.bindings.len())
            .field("child_defaults", &self.child_defaults.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{
        BootstrapHandoff, DescriptorSignature, GuestSeedContract, GuestSetupDescriptor,
        GuestSetupDescriptorVerifier, SignatureAlgorithm, VerifiedGuestSetupDescriptor,
    };
    use d2b_contracts_provider::v3::ArtifactDigest;
    use d2b_contracts_resource::v3::{
        ArtifactId, ResourceGeneration, SchemaFingerprint, SchemaVersion,
    };

    const ARTIFACT_DIGEST: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SCHEMA_FINGERPRINT: &str =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    struct TestVerifier;

    impl GuestSetupDescriptorVerifier for TestVerifier {
        fn verify(
            &self,
            _key_fingerprint: &SchemaFingerprint,
            _descriptor_digest: &SchemaFingerprint,
            signature: &str,
        ) -> bool {
            signature == "signature-sentinel"
        }
    }

    fn descriptor() -> VerifiedGuestSetupDescriptor {
        GuestSetupDescriptor::new(
            ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
            ResourceGeneration::new(3).unwrap(),
            ArtifactId::parse("guest-system").unwrap(),
            ArtifactDigest::parse(ARTIFACT_DIGEST).unwrap(),
            GuestSeedContract::new(
                "guest-resource-seed",
                SchemaVersion::new(1, 0).unwrap(),
                SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
            )
            .unwrap(),
            BootstrapHandoff::new("opaque-bootstrap", 30_000).unwrap(),
            DescriptorSignature::new(
                SignatureAlgorithm::Ed25519Blake3,
                SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
                "signature-sentinel",
            )
            .unwrap(),
        )
        .unwrap()
        .verify_with(&TestVerifier)
        .unwrap()
    }

    #[test]
    fn guest_child_graph_is_deterministic_name_addressed_and_redacted() {
        let zone = ZoneId::parse("dev").unwrap();
        let guest = ResourceRef::parse("Guest/gateway").unwrap();
        let execution = ResourceRef::parse("Host/host-system").unwrap();
        let descriptor = descriptor();

        let first = GuestChildGraphPlan::from_descriptor(
            zone.clone(),
            guest.clone(),
            execution.clone(),
            &descriptor,
        )
        .unwrap();
        let second =
            BootstrapGraph::plan_children(zone.clone(), guest.clone(), execution, &descriptor)
                .unwrap();

        assert_eq!(first, second);
        assert_eq!(
            first.creation_order(),
            &[
                ResourceRef::parse("Volume/gateway-system").unwrap(),
                ResourceRef::parse("Process/gateway-vmm").unwrap(),
                ResourceRef::parse("Endpoint/gateway-ch-api").unwrap(),
                ResourceRef::parse("Endpoint/gateway-guest-control").unwrap(),
            ]
        );
        assert_eq!(
            first.deletion_order(),
            &[
                ResourceRef::parse("Endpoint/gateway-ch-api").unwrap(),
                ResourceRef::parse("Endpoint/gateway-guest-control").unwrap(),
                ResourceRef::parse("Process/gateway-vmm").unwrap(),
                ResourceRef::parse("Volume/gateway-system").unwrap(),
            ]
        );

        let batch = first.child_batch();
        assert_eq!(batch.mutations().len(), 4);
        assert!(batch.mutations().iter().all(|mutation| {
            mutation.owner_ref() == &guest && mutation.zone() == &zone
        }));

        let rendered = format!("{first:?}");
        let canonical = String::from_utf8(batch.canonical_bytes().unwrap()).unwrap();
        for output in [rendered, canonical] {
            assert!(!output.contains("uid"));
            assert!(!output.contains("store"));
            assert!(!output.contains("credential"));
            assert!(!output.contains("argv"));
            assert!(!output.contains("locator"));
            assert!(!output.contains("opaque-bootstrap"));
            assert!(!output.contains("signature-sentinel"));
        }
    }

    #[test]
    fn vmm_lifecycle_stays_stopped_until_every_dependency_is_ready() {
        let graph = BootstrapGraph::new(
            vec![ResourceRef::parse("Device/kvm").unwrap()],
            vec![ResourceRef::parse("Network/cloud").unwrap()],
            vec![ResourceRef::parse("Volume/state").unwrap()],
            vec![ResourceRef::parse("VolumeBinding/state-share").unwrap()],
            vec![AttachmentRef::new("launch-ticket").unwrap()],
        )
        .unwrap();

        for pending in 0..5 {
            let mut ready = [true; 5];
            ready[pending] = false;
            let snapshot = VmmReadinessSnapshot {
                devices_ready: ready[0],
                networks_ready: ready[1],
                volumes_ready: ready[2],
                bindings_ready: ready[3],
                setup_ready: ready[4],
            };
            assert_eq!(
                graph.vmm_lifecycle(snapshot),
                VmmLifecycleEligibility::Stopped
            );
            assert_eq!(graph.vmm_readiness(snapshot), DependencyReadiness::Pending);
        }
        let all_ready = VmmReadinessSnapshot {
            devices_ready: true,
            networks_ready: true,
            volumes_ready: true,
            bindings_ready: true,
            setup_ready: true,
        };
        assert_eq!(
            graph.vmm_lifecycle(all_ready),
            VmmLifecycleEligibility::Running
        );
        assert_eq!(graph.vmm_readiness(all_ready), DependencyReadiness::Ready);
    }

    #[test]
    fn child_planning_rejects_invalid_resource_references_before_returning_a_graph() {
        let descriptor = descriptor();
        assert!(
            GuestChildGraphPlan::from_descriptor(
                ZoneId::parse("dev").unwrap(),
                ResourceRef::parse("Process/not-a-guest").unwrap(),
                ResourceRef::parse("Host/host-system").unwrap(),
                &descriptor,
            )
            .is_err()
        );
        assert!(
            GuestChildGraphPlan::from_descriptor(
                ZoneId::parse("dev").unwrap(),
                ResourceRef::parse("Guest/gateway").unwrap(),
                ResourceRef::parse("Guest/not-a-host").unwrap(),
                &descriptor,
            )
            .is_err()
        );
    }

    #[test]
    fn legacy_three_dependency_readiness_remains_a_strict_subset() {
        let graph = BootstrapGraph::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
            .unwrap();
        let all_ready = VmmReadinessSnapshot {
            devices_ready: true,
            networks_ready: true,
            volumes_ready: true,
            bindings_ready: true,
            setup_ready: true,
        };
        let volume_pending = VmmReadinessSnapshot {
            devices_ready: true,
            networks_ready: true,
            volumes_ready: false,
            bindings_ready: true,
            setup_ready: true,
        };
        assert_eq!(
            graph.vmm_readiness(all_ready),
            DependencyReadiness::Ready
        );
        assert_eq!(
            graph.vmm_readiness(volume_pending),
            DependencyReadiness::Pending
        );
    }
}
