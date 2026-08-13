use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts::{
    provider_effects::aca::{
        AcaConfiguredDiskId, AcaControl, AcaControlContext, AcaControlError, AcaControlErrorKind,
        AcaControlHealth, AcaCpuMillis, AcaCredentialLease, AcaCredentialLeaseClient,
        AcaCredentialLeaseRequest, AcaDeleteOutcome, AcaDesiredDiskImage, AcaDesiredSandbox,
        AcaDiskImageCandidates, AcaDiskImageId, AcaDiskImageRecord, AcaDiskImageSource,
        AcaMemoryMib, AcaOperationId, AcaProfileId, AcaReadinessPolicy, AcaResourceBinding,
        AcaRuntimeConfig, AcaSandboxCandidates, AcaSandboxId, AcaSandboxLifecycle,
        AcaSandboxProfile, AcaSandboxRecord,
    },
    v3::{ResourceRef, ResourceUid, credential::CredentialLeaseHandle},
};
use d2b_provider_runtime_azure_container_apps::{
    AcaController, AcaControllerError, AcaPhase, AcaReconcileOutcome,
};

#[derive(Default)]
struct FakeState {
    candidates: Vec<AcaSandboxRecord>,
    calls: Vec<&'static str>,
    revoked: usize,
}

struct FakeLeaseClient {
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl AcaCredentialLeaseClient for FakeLeaseClient {
    async fn acquire(
        &self,
        _: &AcaCredentialLeaseRequest,
    ) -> Result<AcaCredentialLease, AcaControlError> {
        Ok(AcaCredentialLease::from_metadata(
            CredentialLeaseHandle::parse("aca-test-lease").unwrap(),
            10_000,
        ))
    }

    async fn revoke(&self, _: &AcaCredentialLease) -> Result<(), AcaControlError> {
        self.state.lock().unwrap().revoked += 1;
        Ok(())
    }
}

struct FakeControl {
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl AcaControl for FakeControl {
    async fn health(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError> {
        Ok(AcaControlHealth::Ready)
    }

    async fn find_sandboxes(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &d2b_provider_runtime_azure_container_apps::AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError> {
        self.state.lock().unwrap().calls.push("find-sandboxes");
        Ok(AcaSandboxCandidates::new(self.state.lock().unwrap().candidates.clone()).unwrap())
    }

    async fn find_disk_images(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError> {
        self.state.lock().unwrap().calls.push("find-images");
        Ok(AcaDiskImageCandidates::new(Vec::new()).unwrap())
    }

    async fn create_disk_image(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError> {
        self.state.lock().unwrap().calls.push("create-image");
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
        self.state.lock().unwrap().calls.push("create-sandbox");
        Ok(record(AcaSandboxLifecycle::Creating))
    }

    async fn resume_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.state.lock().unwrap().calls.push("resume");
        Ok(record(AcaSandboxLifecycle::Running))
    }

    async fn stop_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.state.lock().unwrap().calls.push("stop");
        Ok(record(AcaSandboxLifecycle::Stopped))
    }

    async fn delete_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError> {
        self.state.lock().unwrap().calls.push("delete");
        Ok(AcaDeleteOutcome::Deleted)
    }
}

fn record(lifecycle: AcaSandboxLifecycle) -> AcaSandboxRecord {
    AcaSandboxRecord {
        id: AcaSandboxId::parse("sandbox-1").unwrap(),
        lifecycle,
        generation: 1,
    }
}

fn controller(state: Arc<Mutex<FakeState>>) -> AcaController<FakeControl, FakeLeaseClient> {
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
        Arc::new(FakeControl {
            state: Arc::clone(&state),
        }),
        Arc::new(FakeLeaseClient { state }),
    )
}

#[tokio::test]
async fn running_sandbox_reaches_ready_without_exposing_identity() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    let operation = AcaOperationId::parse("operation-1").unwrap();
    assert_eq!(
        controller.reconcile(operation, 1_000).await.unwrap(),
        AcaReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), AcaPhase::Ready);
    assert!(!format!("{:?}", controller.status()).contains("sandbox-1"));
    assert_eq!(state.lock().unwrap().revoked, 1);
}

#[tokio::test]
async fn ambiguous_adoption_fails_closed() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![
            record(AcaSandboxLifecycle::Running),
            AcaSandboxRecord {
                id: AcaSandboxId::parse("sandbox-2").unwrap(),
                lifecycle: AcaSandboxLifecycle::Running,
                generation: 1,
            },
        ],
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    let error = controller
        .reconcile(AcaOperationId::parse("operation-2").unwrap(), 1_000)
        .await
        .unwrap_err();
    assert_eq!(error, AcaControllerError::AmbiguousAdoption);
    assert_eq!(controller.phase(), AcaPhase::Degraded);
}

#[tokio::test]
async fn missing_sandbox_uses_disk_and_sandbox_effects_then_finalizes() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut controller = controller(Arc::clone(&state));
    assert!(matches!(
        controller
            .reconcile(AcaOperationId::parse("operation-3").unwrap(), 1_000)
            .await
            .unwrap(),
        AcaReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AcaPhase::Provisioning);
    assert_eq!(
        state.lock().unwrap().calls,
        [
            "find-sandboxes",
            "find-images",
            "create-image",
            "create-sandbox"
        ]
    );
    controller
        .finalize(AcaOperationId::parse("operation-4").unwrap(), 1_000)
        .await
        .unwrap();
    assert_eq!(controller.phase(), AcaPhase::Finalized);
    assert!(!controller.finalizer_installed());
    assert_eq!(state.lock().unwrap().calls.last(), Some(&"delete"));
}

#[test]
fn stable_error_codes_are_bounded() {
    assert_eq!(
        AcaControlError::new(AcaControlErrorKind::RateLimited).code(),
        "aca-control-rate-limited"
    );
    let _ = ResourceRef::parse("Guest/gateway").unwrap();
}
