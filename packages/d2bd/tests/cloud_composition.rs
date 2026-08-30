use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts_provider::v3::{
    ArtifactDigest,
    credential::{CredentialLeaseHandle, OpaqueAzureRef},
};
use d2b_contracts_resource::v3::{
    ArtifactId, DesiredLifecycle, ResourceGeneration, ResourcePhase, ResourceRef, ResourceUid,
    SchemaFingerprint, SchemaVersion, ZoneId, ZoneRevision,
};
use d2b_provider_runtime_azure_container_apps::{
    AcaClock, AcaController, AcaPhase, AcaReconcileOutcome,
};
use d2b_provider_runtime_azure_container_apps::{
    AcaConfiguredDiskId, AcaControl, AcaControlContext, AcaControlError, AcaControlErrorKind,
    AcaControlHealth, AcaCpuMillis, AcaCredentialLease, AcaCredentialLeaseClient,
    AcaCredentialLeaseRequest, AcaDeleteOutcome, AcaDesiredDiskImage, AcaDesiredSandbox,
    AcaDiskImageCandidates, AcaDiskImageId, AcaDiskImageRecord, AcaDiskImageSource, AcaMemoryMib,
    AcaOperationId, AcaProfileId, AcaReadinessPolicy, AcaResourceBinding, AcaRuntimeConfig,
    AcaSandboxCandidates, AcaSandboxId, AcaSandboxLifecycle, AcaSandboxProfile, AcaSandboxRecord,
};
use d2b_provider_runtime_cloud_hypervisor::{
    AuthenticatedResourceApiAdapter, AuthenticatedResourceSession, BootstrapGraph,
    BootstrapHandoff, CloudHypervisorConfig, CloudHypervisorController, CloudHypervisorError,
    CloudHypervisorReconcileOutcome, CloudHypervisorResourceApiError,
    CloudHypervisorResourceRequest, CloudHypervisorResourceResponse, CommittedChild,
    DescriptorSignature, GuestChildCommitResponse, GuestGenerationSet, GuestSeedContract,
    GuestSessionEvidence, GuestSetupDescriptor, GuestSetupDescriptorVerifier, GuestSnapshot,
    GuestStatusPhase, OwnedChildSnapshot, SignatureAlgorithm, health::GuestSessionEvidenceBinding,
};

#[derive(Default)]
struct AcaState {
    candidates: Vec<AcaSandboxRecord>,
    health: VecDeque<AcaControlHealth>,
    revoked: usize,
}

struct FakeAcaLease {
    state: Arc<Mutex<AcaState>>,
}

#[async_trait]
impl AcaCredentialLeaseClient for FakeAcaLease {
    async fn acquire(
        &self,
        request: &AcaCredentialLeaseRequest,
    ) -> Result<AcaCredentialLease, AcaControlError> {
        Ok(AcaCredentialLease::from_metadata(
            CredentialLeaseHandle::parse("aca-cloud-composition-lease").unwrap(),
            request.requested_expiry_unix_ms(),
        ))
    }

    async fn revoke(&self, _: &AcaCredentialLease) -> Result<(), AcaControlError> {
        self.state.lock().unwrap().revoked += 1;
        Ok(())
    }
}

struct FakeAcaControl {
    state: Arc<Mutex<AcaState>>,
}

#[async_trait]
impl AcaControl for FakeAcaControl {
    async fn health(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .health
            .pop_front()
            .unwrap_or(AcaControlHealth::Ready))
    }

    async fn find_sandboxes(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &d2b_provider_runtime_azure_container_apps::AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError> {
        AcaSandboxCandidates::new(self.state.lock().unwrap().candidates.clone())
            .map_err(|_| AcaControlError::new(AcaControlErrorKind::InvalidResponse))
    }

    async fn find_disk_images(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError> {
        AcaDiskImageCandidates::new(Vec::new())
            .map_err(|_| AcaControlError::new(AcaControlErrorKind::InvalidResponse))
    }

    async fn create_disk_image(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError> {
        Ok(AcaDiskImageRecord {
            id: AcaDiskImageId::parse("disk-1").unwrap(),
            generation: 1,
        })
    }

    async fn create_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaDesiredSandbox,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        Ok(aca_record("sandbox-created", AcaSandboxLifecycle::Creating))
    }

    async fn resume_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        Ok(aca_record(
            sandbox_id.as_str(),
            AcaSandboxLifecycle::Running,
        ))
    }

    async fn stop_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        Ok(aca_record(
            sandbox_id.as_str(),
            AcaSandboxLifecycle::Stopped,
        ))
    }

    async fn delete_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError> {
        Ok(AcaDeleteOutcome::Deleted)
    }
}

fn aca_record(id: &str, lifecycle: AcaSandboxLifecycle) -> AcaSandboxRecord {
    AcaSandboxRecord {
        id: AcaSandboxId::parse(id).unwrap(),
        lifecycle,
        generation: 1,
    }
}

struct FixedAcaClock;

impl AcaClock for FixedAcaClock {
    fn now_unix_ms(&self) -> u64 {
        1_000
    }
}

fn aca_controller(state: Arc<Mutex<AcaState>>) -> AcaController<FakeAcaControl, FakeAcaLease> {
    let profile = AcaSandboxProfile::new(
        AcaProfileId::parse("default").unwrap(),
        AcaDiskImageSource::ConfiguredDisk {
            binding_id: AcaConfiguredDiskId::parse("image-1").unwrap(),
        },
        AcaCpuMillis::new(500).unwrap(),
        AcaMemoryMib::new(2_048).unwrap(),
        300,
        None,
    )
    .unwrap();
    let config =
        AcaRuntimeConfig::new(profile, AcaReadinessPolicy::new(3, 10).unwrap(), 1_000, 4).unwrap();
    let binding = AcaResourceBinding {
        guest_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        provider_generation: 1,
        config_fingerprint: [7; 32],
    };
    AcaController::new(
        binding,
        config,
        Arc::new(FakeAcaControl {
            state: Arc::clone(&state),
        }),
        Arc::new(FakeAcaLease { state }),
    )
    .with_clock(Arc::new(FixedAcaClock))
}

const CLOUD_ARTIFACT_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CLOUD_SCHEMA_FINGERPRINT: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CLOUD_GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const CLOUD_ZONE_UID: &str = "223e4567-e89b-42d3-a456-426614174001";

struct AcceptingCloudDescriptorVerifier;

impl GuestSetupDescriptorVerifier for AcceptingCloudDescriptorVerifier {
    fn verify(
        &self,
        _key_fingerprint: &SchemaFingerprint,
        _descriptor_digest: &SchemaFingerprint,
        signature: &str,
    ) -> bool {
        signature == "signature-sentinel"
    }
}

fn cloud_descriptor() -> d2b_provider_runtime_cloud_hypervisor::VerifiedGuestSetupDescriptor {
    GuestSetupDescriptor::new(
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        ResourceGeneration::new(3).unwrap(),
        ArtifactId::parse("guest-system").unwrap(),
        ArtifactDigest::parse(CLOUD_ARTIFACT_DIGEST).unwrap(),
        GuestSeedContract::new(
            "guest-resource-seed",
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(CLOUD_SCHEMA_FINGERPRINT).unwrap(),
        )
        .unwrap(),
        BootstrapHandoff::new("opaque-bootstrap", 30_000).unwrap(),
        DescriptorSignature::new(
            SignatureAlgorithm::Ed25519Blake3,
            SchemaFingerprint::parse(CLOUD_SCHEMA_FINGERPRINT).unwrap(),
            "signature-sentinel",
        )
        .unwrap(),
    )
    .unwrap()
    .verify_with(&AcceptingCloudDescriptorVerifier)
    .unwrap()
}

fn cloud_guest() -> GuestSnapshot {
    let guest_ref = ResourceRef::parse("Guest/gateway").unwrap();
    let evidence = GuestSessionEvidence::current_bound(
        guest_ref.clone(),
        format!("sha256:{}", "0".repeat(64)),
        Vec::<String>::new(),
        true,
        true,
        true,
        GuestSessionEvidenceBinding::new(
            CLOUD_GUEST_UID,
            CLOUD_SCHEMA_FINGERPRINT,
            CLOUD_SCHEMA_FINGERPRINT,
            1,
            1,
            1,
            1,
            1,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    GuestSnapshot::new(
        ZoneId::parse("work").unwrap(),
        ResourceUid::parse(CLOUD_ZONE_UID).unwrap(),
        guest_ref,
        ResourceUid::parse(CLOUD_GUEST_UID).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ZoneRevision::new(7),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        Some("guest-system".to_owned()),
        GuestGenerationSet::all(1),
        false,
    )
    .unwrap()
    .with_session_evidence(evidence)
}

fn cloud_graph() -> BootstrapGraph {
    BootstrapGraph::new(
        vec![ResourceRef::parse("Device/kvm").unwrap()],
        vec![ResourceRef::parse("Network/cloud").unwrap()],
        vec![ResourceRef::parse("Volume/state").unwrap()],
        vec![],
    )
    .unwrap()
}

fn cloud_children(
    guest: &GuestSnapshot,
    expected_refs: &[ResourceRef],
    owner_uid: &ResourceUid,
) -> Vec<OwnedChildSnapshot> {
    expected_refs
        .iter()
        .enumerate()
        .map(|(index, resource_ref)| {
            OwnedChildSnapshot::new(
                resource_ref.clone(),
                guest.zone().clone(),
                guest.resource_ref().clone(),
                ResourceUid::parse(format!("323e4567-e89b-42d3-a456-42661417{index:04}")).unwrap(),
                guest.generation(),
                guest.revision(),
                "ready",
                ResourcePhase::Ready,
                (resource_ref.resource_type().as_str() == "Process")
                    .then_some(DesiredLifecycle::Running),
                true,
            )
            .unwrap()
            .with_owner_uid(owner_uid.clone())
        })
        .collect()
}

#[derive(Clone, Copy)]
enum CloudMode {
    Ready,
    Failed,
    Ambiguous,
}

struct FakeCloudSession {
    mode: CloudMode,
}

#[async_trait]
impl AuthenticatedResourceSession for FakeCloudSession {
    async fn call(
        &self,
        request: CloudHypervisorResourceRequest,
    ) -> Result<CloudHypervisorResourceResponse, CloudHypervisorResourceApiError> {
        match request {
            CloudHypervisorResourceRequest::Register { .. } => {
                Ok(CloudHypervisorResourceResponse::Registered)
            }
            CloudHypervisorResourceRequest::GetGuest { .. } => match self.mode {
                CloudMode::Failed => Err(CloudHypervisorResourceApiError::Transport),
                CloudMode::Ready | CloudMode::Ambiguous => {
                    Ok(CloudHypervisorResourceResponse::Guest(cloud_guest()))
                }
            },
            CloudHypervisorResourceRequest::RelistOwnedChildren { expected_refs, .. } => {
                let guest = cloud_guest();
                let children = match self.mode {
                    CloudMode::Ambiguous => {
                        let wrong_owner =
                            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap();
                        cloud_children(&guest, &expected_refs[..1], &wrong_owner)
                    }
                    CloudMode::Ready | CloudMode::Failed => {
                        cloud_children(&guest, &expected_refs, guest.uid())
                    }
                };
                Ok(CloudHypervisorResourceResponse::OwnedChildren(children))
            }
            CloudHypervisorResourceRequest::ObserveDependencies { graph, .. } => {
                Ok(CloudHypervisorResourceResponse::Dependencies(
                    d2b_provider_runtime_cloud_hypervisor::GuestDependencySnapshot::ready(graph),
                ))
            }
            CloudHypervisorResourceRequest::CommitBatch { .. } => Ok(
                CloudHypervisorResourceResponse::Committed(GuestChildCommitResponse::Uncertain),
            ),
            CloudHypervisorResourceRequest::UpdateSpec { update } => {
                Ok(CloudHypervisorResourceResponse::Updated(
                    CommittedChild::new(
                        update.target().clone(),
                        ResourceRef::parse("Guest/gateway").unwrap(),
                        ZoneId::parse("work").unwrap(),
                        update.expected_uid().clone(),
                        ZoneRevision::new(update.expected_revision().get().saturating_add(1)),
                    )
                    .unwrap(),
                ))
            }
            CloudHypervisorResourceRequest::UpdateStatus { .. } => {
                Ok(CloudHypervisorResourceResponse::StatusUpdated)
            }
        }
    }
}

fn cloud_controller(
    mode: CloudMode,
) -> CloudHypervisorController<AuthenticatedResourceApiAdapter<FakeCloudSession>> {
    let config = CloudHypervisorConfig {
        controller_execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        default_vcpus: 2,
        default_memory_mb: 512,
        default_machine_type: OpaqueAzureRef::parse("q35").unwrap(),
        watchdog: true,
        adoption_window_ms: 30_000,
        health_check_interval_ms: 30_000,
        health_check_timeout_ms: 5_000,
        health_check_failure_threshold: 3,
        startup_deadline_ms: 120_000,
    };
    let api = AuthenticatedResourceApiAdapter::new(Arc::new(FakeCloudSession { mode }));
    CloudHypervisorController::from_verified_descriptor(
        config,
        cloud_graph(),
        cloud_descriptor(),
        Arc::new(api),
    )
    .unwrap()
}

#[tokio::test]
async fn cloud_composition_reaches_ready_through_production_controllers() {
    let mut cloud = cloud_controller(CloudMode::Ready);
    cloud.register().await.unwrap();
    let cloud_outcome = cloud
        .reconcile(&ResourceRef::parse("Guest/gateway").unwrap())
        .await
        .unwrap();
    assert_eq!(
        cloud_outcome.status().status().phase,
        GuestStatusPhase::Ready
    );
    assert!(matches!(
        cloud_outcome,
        CloudHypervisorReconcileOutcome::Ready(_)
    ));

    let aca_state = Arc::new(Mutex::new(AcaState {
        candidates: vec![aca_record("sandbox-1", AcaSandboxLifecycle::Running)],
        ..AcaState::default()
    }));
    let mut aca = aca_controller(Arc::clone(&aca_state));
    assert_eq!(
        aca.reconcile(AcaOperationId::parse("cloud-compose").unwrap(), 1_000)
            .await
            .unwrap(),
        AcaReconcileOutcome::Converged
    );
    assert_eq!(aca.phase(), AcaPhase::Ready);
    assert!(
        !format!("{:?}", aca.status()).contains("sandbox-1"),
        "Azure provider status must retain only a digest, not a cloud identity"
    );
    assert!(aca_state.lock().unwrap().revoked > 0);
}

#[tokio::test]
async fn cloud_composition_fails_closed_on_ambiguous_or_failed_effects() {
    let mut cloud = cloud_controller(CloudMode::Failed);
    cloud.register().await.unwrap();
    assert_eq!(
        cloud
            .reconcile(&ResourceRef::parse("Guest/gateway").unwrap())
            .await,
        Err(CloudHypervisorError::ResourceApi(
            CloudHypervisorResourceApiError::Transport
        ))
    );

    let mut ambiguous_cloud = cloud_controller(CloudMode::Ambiguous);
    ambiguous_cloud.register().await.unwrap();
    assert_eq!(
        ambiguous_cloud
            .reconcile(&ResourceRef::parse("Guest/gateway").unwrap())
            .await,
        Err(CloudHypervisorError::ChildConflict)
    );

    let aca_state = Arc::new(Mutex::new(AcaState {
        candidates: vec![
            aca_record("sandbox-1", AcaSandboxLifecycle::Running),
            aca_record("sandbox-2", AcaSandboxLifecycle::Running),
        ],
        ..AcaState::default()
    }));
    let mut aca = aca_controller(aca_state);
    assert_eq!(
        aca.reconcile(AcaOperationId::parse("cloud-ambiguous").unwrap(), 1_000)
            .await,
        Err(d2b_provider_runtime_azure_container_apps::AcaControllerError::AmbiguousAdoption)
    );
}
