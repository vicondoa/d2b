//! Dependency-gated VMM bootstrap graph.

use std::fmt;

use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_core_controller::OwnedChildKind;

use crate::{descriptor::VerifiedGuestSetupDescriptor, identity::GuestChildBatch};

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

/// Opaque VMM attachment reference.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttachmentRef(String);

impl AttachmentRef {
    /// Construct a bounded opaque attachment ref.
    pub fn new(value: impl Into<String>) -> Result<Self, BootstrapGraphError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(BootstrapGraphError::InvalidReference);
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
        attachments: Vec<AttachmentRef>,
    ) -> Result<Self, BootstrapGraphError> {
        if devices
            .iter()
            .chain(networks.iter())
            .chain(volumes.iter())
            .any(|reference| reference.resource_type().as_str() == "Host")
        {
            return Err(BootstrapGraphError::InvalidReference);
        }
        Ok(Self {
            devices,
            networks,
            volumes,
            attachments,
        })
    }

    /// Check the dependency barrier.
    pub fn readiness(
        &self,
        devices_ready: bool,
        networks_ready: bool,
        volumes_ready: bool,
    ) -> DependencyReadiness {
        self.vmm_readiness(devices_ready, networks_ready, volumes_ready, true, true)
    }

    /// Check all pre-start dependencies without performing an effect.
    pub fn vmm_readiness(
        &self,
        devices_ready: bool,
        networks_ready: bool,
        volumes_ready: bool,
        exports_ready: bool,
        setup_ready: bool,
    ) -> DependencyReadiness {
        if devices_ready && networks_ready && volumes_ready && exports_ready && setup_ready {
            DependencyReadiness::Ready
        } else {
            DependencyReadiness::Pending
        }
    }

    /// Return the pure VMM lifecycle decision for a dependency snapshot.
    pub fn vmm_lifecycle(
        &self,
        devices_ready: bool,
        networks_ready: bool,
        volumes_ready: bool,
        exports_ready: bool,
        setup_ready: bool,
    ) -> VmmLifecycleEligibility {
        match self.vmm_readiness(
            devices_ready,
            networks_ready,
            volumes_ready,
            exports_ready,
            setup_ready,
        ) {
            DependencyReadiness::Ready => VmmLifecycleEligibility::Running,
            DependencyReadiness::Pending => VmmLifecycleEligibility::Stopped,
        }
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
            .map_err(|_| BootstrapGraphError::InvalidReference)?;
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

/// Bootstrap graph construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapGraphError {
    /// A reference or opaque ticket was invalid.
    InvalidReference,
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
            mutation.owner_ref() == &guest
                && mutation.zone() == &zone
                && mutation.expected_uid().is_none()
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
            vec![AttachmentRef::new("launch-ticket").unwrap()],
        )
        .unwrap();

        for pending in 0..5 {
            let mut ready = [true; 5];
            ready[pending] = false;
            assert_eq!(
                graph.vmm_lifecycle(ready[0], ready[1], ready[2], ready[3], ready[4]),
                VmmLifecycleEligibility::Stopped
            );
            assert_eq!(
                graph.vmm_readiness(ready[0], ready[1], ready[2], ready[3], ready[4]),
                DependencyReadiness::Pending
            );
        }
        assert_eq!(
            graph.vmm_lifecycle(true, true, true, true, true),
            VmmLifecycleEligibility::Running
        );
        assert_eq!(
            graph.vmm_readiness(true, true, true, true, true),
            DependencyReadiness::Ready
        );
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
        let graph = BootstrapGraph::new(Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
        assert_eq!(
            graph.readiness(true, true, true),
            DependencyReadiness::Ready
        );
        assert_eq!(
            graph.readiness(true, true, false),
            DependencyReadiness::Pending
        );
    }
}
