use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts_provider::v3::credential::OpaqueAzureRef;
use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_provider_runtime_cloud_hypervisor::{
    AuthenticatedResourceApiAdapter, AuthenticatedResourceSession, BootstrapGraph,
    BootstrapHandoff, CloudHypervisorConfig, CloudHypervisorController,
    CloudHypervisorResourceApiError, CloudHypervisorResourceRequest,
    CloudHypervisorResourceResponse, DescriptorSignature, GuestGenerationSet, GuestSeedContract,
    GuestSetupDescriptor, GuestSetupDescriptorVerifier, GuestSnapshot, SignatureAlgorithm,
};

const ARTIFACT_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SCHEMA_FINGERPRINT: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const ZONE_UID: &str = "223e4567-e89b-42d3-a456-426614174001";

struct AcceptingVerifier;

impl GuestSetupDescriptorVerifier for AcceptingVerifier {
    fn verify(
        &self,
        _key_fingerprint: &d2b_contracts_resource::v3::SchemaFingerprint,
        _descriptor_digest: &d2b_contracts_resource::v3::SchemaFingerprint,
        signature: &str,
    ) -> bool {
        signature == "signature-sentinel"
    }
}

fn descriptor() -> d2b_provider_runtime_cloud_hypervisor::VerifiedGuestSetupDescriptor {
    GuestSetupDescriptor::new(
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        ResourceGeneration::new(3).unwrap(),
        d2b_contracts_resource::v3::ArtifactId::parse("guest-system").unwrap(),
        d2b_contracts_provider::v3::ArtifactDigest::parse(ARTIFACT_DIGEST).unwrap(),
        GuestSeedContract::new(
            "guest-resource-seed",
            d2b_contracts_resource::v3::SchemaVersion::new(1, 0).unwrap(),
            d2b_contracts_resource::v3::SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
        )
        .unwrap(),
        BootstrapHandoff::new("opaque-bootstrap", 30_000).unwrap(),
        DescriptorSignature::new(
            SignatureAlgorithm::Ed25519Blake3,
            d2b_contracts_resource::v3::SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
            "signature-sentinel",
        )
        .unwrap(),
    )
    .unwrap()
    .verify_with(&AcceptingVerifier)
    .unwrap()
}

fn config() -> CloudHypervisorConfig {
    CloudHypervisorConfig {
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
    }
}

fn graph() -> BootstrapGraph {
    BootstrapGraph::new(
        vec![ResourceRef::parse("Device/kvm").unwrap()],
        vec![ResourceRef::parse("Network/work").unwrap()],
        vec![ResourceRef::parse("Volume/store").unwrap()],
        vec![],
    )
    .unwrap()
}

fn guest() -> GuestSnapshot {
    GuestSnapshot::new(
        ZoneId::parse("work").unwrap(),
        ResourceUid::parse(ZONE_UID).unwrap(),
        ResourceRef::parse("Guest/gateway").unwrap(),
        ResourceUid::parse(GUEST_UID).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ZoneRevision::new(7),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        Some("guest-system".to_owned()),
        GuestGenerationSet::all(1),
        false,
    )
    .unwrap()
}

#[derive(Default)]
struct RecordingSession {
    requests: Mutex<Vec<CloudHypervisorResourceRequest>>,
}

#[async_trait]
impl AuthenticatedResourceSession for RecordingSession {
    async fn call(
        &self,
        request: CloudHypervisorResourceRequest,
    ) -> Result<CloudHypervisorResourceResponse, CloudHypervisorResourceApiError> {
        self.requests.lock().unwrap().push(request.clone());
        match request {
            CloudHypervisorResourceRequest::Register { .. } => {
                Ok(CloudHypervisorResourceResponse::Registered)
            }
            CloudHypervisorResourceRequest::GetGuest { .. } => {
                Ok(CloudHypervisorResourceResponse::Guest(guest()))
            }
            CloudHypervisorResourceRequest::RelistOwnedChildren { .. } => {
                Ok(CloudHypervisorResourceResponse::OwnedChildren(Vec::new()))
            }
            CloudHypervisorResourceRequest::ObserveDependencies { .. } => {
                Ok(CloudHypervisorResourceResponse::Dependencies(
                    d2b_provider_runtime_cloud_hypervisor::GuestDependencySnapshot::ready(graph()),
                ))
            }
            CloudHypervisorResourceRequest::CommitBatch { .. } => {
                Ok(CloudHypervisorResourceResponse::Committed(
                    d2b_provider_runtime_cloud_hypervisor::GuestChildCommitResponse::Uncertain,
                ))
            }
            CloudHypervisorResourceRequest::UpdateSpec { .. } => {
                Err(CloudHypervisorResourceApiError::InvalidResponse)
            }
            CloudHypervisorResourceRequest::UpdateStatus { .. } => {
                Ok(CloudHypervisorResourceResponse::StatusUpdated)
            }
            CloudHypervisorResourceRequest::ObserveProcessAdoption { .. } => {
                Ok(CloudHypervisorResourceResponse::ProcessAdoption(
                    d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Current,
                ))
            }
            CloudHypervisorResourceRequest::AssessUpdate { .. } => {
                Ok(CloudHypervisorResourceResponse::UpdateAssessment(None))
            }
            CloudHypervisorResourceRequest::ObserveFinalization { .. } => {
                Err(CloudHypervisorResourceApiError::InvalidResponse)
            }
            CloudHypervisorResourceRequest::DrainGuestLocal { .. }
            | CloudHypervisorResourceRequest::CloseGuestSession { .. }
            | CloudHypervisorResourceRequest::DeleteChild { .. }
            | CloudHypervisorResourceRequest::InvalidateGuestSession { .. }
            | CloudHypervisorResourceRequest::ClearGuestFinalizer { .. } => {
                Ok(CloudHypervisorResourceResponse::LifecycleApplied)
            }
        }
    }
}

#[tokio::test]
async fn registration_uses_verified_descriptor_and_authenticated_resource_calls() {
    let session = Arc::new(RecordingSession::default());
    let api = AuthenticatedResourceApiAdapter::new(Arc::clone(&session));
    let mut controller = CloudHypervisorController::from_verified_descriptor(
        config(),
        graph(),
        descriptor(),
        api.into(),
    )
    .unwrap();

    controller.register().await.unwrap();

    let registration = controller.registration();
    assert_eq!(
        registration.provider_ref(),
        &ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap()
    );
    assert_eq!(
        registration.descriptor_digest(),
        descriptor().descriptor().descriptor_digest()
    );
    assert_eq!(
        registration.watched_types(),
        &[
            "Guest".parse().unwrap(),
            "Process".parse().unwrap(),
            "Endpoint".parse().unwrap(),
            "Volume".parse().unwrap(),
        ]
    );
    assert_eq!(
        registration.dependency_types(),
        &["Device".parse().unwrap(), "Network".parse().unwrap()]
    );

    let outcome = controller
        .reconcile(&ResourceRef::parse("Guest/gateway").unwrap())
        .await
        .unwrap();
    assert!(outcome.is_pending());

    let requests = session.requests.lock().unwrap();
    assert!(matches!(
        requests.first(),
        Some(CloudHypervisorResourceRequest::Register { .. })
    ));
    assert!(requests.iter().any(|request| matches!(
        request,
        CloudHypervisorResourceRequest::RelistOwnedChildren { .. }
    )));
}

#[tokio::test]
async fn one_uid_free_batch_contains_the_complete_guest_owned_child_graph() {
    let session = Arc::new(RecordingSession::default());
    let api = AuthenticatedResourceApiAdapter::new(Arc::clone(&session));
    let mut controller = CloudHypervisorController::from_verified_descriptor(
        config(),
        graph(),
        descriptor(),
        api.into(),
    )
    .unwrap();
    controller.register().await.unwrap();
    controller
        .reconcile(&ResourceRef::parse("Guest/gateway").unwrap())
        .await
        .unwrap();

    let requests = session.requests.lock().unwrap();
    let Some(CloudHypervisorResourceRequest::CommitBatch { batch }) = requests
        .iter()
        .find(|request| matches!(request, CloudHypervisorResourceRequest::CommitBatch { .. }))
    else {
        panic!("expected one child CommitBatch");
    };
    assert_eq!(batch.mutations().len(), 4);
    let mut targets = batch
        .mutations()
        .iter()
        .map(|mutation| mutation.target().to_canonical_string())
        .collect::<Vec<_>>();
    targets.sort();
    assert_eq!(
        targets,
        vec![
            "Endpoint/gateway-ch-api",
            "Endpoint/gateway-guest-control",
            "Process/gateway-vmm",
            "Volume/gateway-system",
        ]
    );
    assert_eq!(batch.owner_uid(), &ResourceUid::parse(GUEST_UID).unwrap());
    assert_eq!(batch.owner_revision(), ZoneRevision::new(7));
    assert!(batch.mutations().iter().all(|mutation| {
        mutation.expected_uid().is_none()
            && mutation.owner_ref() == &ResourceRef::parse("Guest/gateway").unwrap()
            && mutation.zone() == &ZoneId::parse("work").unwrap()
    }));
    for mutation in batch.mutations() {
        let payload =
            String::from_utf8(batch.canonical_payload(mutation.target()).unwrap()).unwrap();
        assert!(!payload.contains("\"uid\""));
        assert!(!payload.contains("argv"));
        assert!(!payload.contains("credential"));
        assert!(!payload.contains("locator"));
        assert!(!payload.contains("/nix/store"));
    }
}

#[test]
fn same_guest_name_in_different_zones_has_distinct_private_runtime_identity() {
    let session = Arc::new(RecordingSession::default());
    let api = AuthenticatedResourceApiAdapter::new(session);
    let controller = CloudHypervisorController::from_verified_descriptor(
        config(),
        graph(),
        descriptor(),
        api.into(),
    )
    .unwrap();
    let first = guest();
    let second = GuestSnapshot::new(
        ZoneId::parse("other").unwrap(),
        ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
        ResourceRef::parse("Guest/gateway").unwrap(),
        ResourceUid::parse("323e4567-e89b-42d3-a456-426614174003").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ZoneRevision::new(7),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        Some("guest-system".to_owned()),
        GuestGenerationSet::all(1),
        false,
    )
    .unwrap();

    assert_ne!(
        controller.private_runtime_scope(&first, "vmm").unwrap(),
        controller.private_runtime_scope(&second, "vmm").unwrap()
    );
}

#[test]
fn invalid_descriptor_is_rejected_before_registration() {
    let raw = GuestSetupDescriptor::new(
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        ResourceGeneration::new(3).unwrap(),
        d2b_contracts_resource::v3::ArtifactId::parse("guest-system").unwrap(),
        d2b_contracts_provider::v3::ArtifactDigest::parse(ARTIFACT_DIGEST).unwrap(),
        GuestSeedContract::new(
            "guest-resource-seed",
            d2b_contracts_resource::v3::SchemaVersion::new(1, 0).unwrap(),
            d2b_contracts_resource::v3::SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
        )
        .unwrap(),
        BootstrapHandoff::new("opaque-bootstrap", 30_000).unwrap(),
        DescriptorSignature::new(
            SignatureAlgorithm::Ed25519Blake3,
            d2b_contracts_resource::v3::SchemaFingerprint::parse(SCHEMA_FINGERPRINT).unwrap(),
            "wrong-signature",
        )
        .unwrap(),
    )
    .unwrap();
    let session = Arc::new(RecordingSession::default());
    let api = AuthenticatedResourceApiAdapter::new(Arc::clone(&session));

    let error = CloudHypervisorController::from_descriptor(
        config(),
        graph(),
        raw,
        &AcceptingVerifier,
        api.into(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::Descriptor(_)
    ));
    assert!(session.requests.lock().unwrap().is_empty());
}
