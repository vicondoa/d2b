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
    AcaClock, AcaController, AcaControllerError, AcaPhase, AcaReconcileOutcome,
};

#[derive(Default)]
struct FakeState {
    candidates: Vec<AcaSandboxRecord>,
    calls: Vec<&'static str>,
    revoked: usize,
    lease_expiries: Vec<u64>,
    resume_lifecycle: Option<AcaSandboxLifecycle>,
    delete_failures: usize,
}

struct FakeLeaseClient {
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl AcaCredentialLeaseClient for FakeLeaseClient {
    async fn acquire(
        &self,
        request: &AcaCredentialLeaseRequest,
    ) -> Result<AcaCredentialLease, AcaControlError> {
        self.state
            .lock()
            .unwrap()
            .lease_expiries
            .push(request.requested_expiry_unix_ms());
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
        let lifecycle = self
            .state
            .lock()
            .unwrap()
            .resume_lifecycle
            .unwrap_or(AcaSandboxLifecycle::Running);
        Ok(record(lifecycle))
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
        let mut state = self.state.lock().unwrap();
        if state.delete_failures > 0 {
            state.delete_failures -= 1;
            return Err(AcaControlError::new(AcaControlErrorKind::Unavailable));
        }
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

struct FixedClock(u64);

impl AcaClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        self.0
    }
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

#[tokio::test]
async fn resume_waits_for_running_lifecycle() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Suspended)],
        resume_lifecycle: Some(AcaSandboxLifecycle::Creating),
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    assert!(matches!(
        controller
            .reconcile(AcaOperationId::parse("operation-resume").unwrap(), 1_000)
            .await
            .unwrap(),
        AcaReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AcaPhase::Provisioning);
}

#[tokio::test]
async fn readiness_attempts_are_bounded() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Creating)],
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    for index in 0..2 {
        assert!(matches!(
            controller
                .reconcile(
                    AcaOperationId::parse(format!("operation-ready-{index}")).unwrap(),
                    1_000
                )
                .await
                .unwrap(),
            AcaReconcileOutcome::Progressing { .. }
        ));
    }
    assert_eq!(
        controller
            .reconcile(
                AcaOperationId::parse("operation-ready-final").unwrap(),
                1_000
            )
            .await
            .unwrap_err(),
        AcaControllerError::ReadinessExhausted
    );
    assert_eq!(controller.phase(), AcaPhase::Failed);
}

#[tokio::test]
async fn lease_expiry_uses_absolute_unix_time() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state)).with_clock(Arc::new(FixedClock(1_234_567)));
    controller
        .reconcile(AcaOperationId::parse("operation-clock").unwrap(), 1_000)
        .await
        .unwrap();
    assert_eq!(state.lock().unwrap().lease_expiries, vec![1_235_567]);
}

#[tokio::test]
async fn finalization_retries_after_partial_delete_failure() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        delete_failures: 1,
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    controller
        .reconcile(
            AcaOperationId::parse("operation-finalize-observe").unwrap(),
            1_000,
        )
        .await
        .unwrap();
    assert_eq!(
        controller
            .finalize(
                AcaOperationId::parse("operation-finalize-first").unwrap(),
                1_000
            )
            .await
            .unwrap_err(),
        AcaControllerError::Effect(AcaControlErrorKind::Unavailable)
    );
    assert!(controller.finalizer_installed());
    controller
        .finalize(
            AcaOperationId::parse("operation-finalize-retry").unwrap(),
            1_000,
        )
        .await
        .unwrap();
    assert!(!controller.finalizer_installed());
}

#[test]
fn stable_error_codes_are_bounded() {
    assert_eq!(
        AcaControlError::new(AcaControlErrorKind::RateLimited).code(),
        "aca-control-rate-limited"
    );
    let _ = ResourceRef::parse("Guest/gateway").unwrap();
}
