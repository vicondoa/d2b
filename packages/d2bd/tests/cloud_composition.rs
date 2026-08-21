use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts_provider::v3::credential::CredentialLeaseHandle;
use d2b_contracts_zone_session::v3::{ResourceRef, ResourceUid};
use d2b_provider_runtime_azure_container_apps::{
    AcaConfiguredDiskId, AcaControl, AcaControlContext, AcaControlError, AcaControlErrorKind,
    AcaControlHealth, AcaCpuMillis, AcaCredentialLease, AcaCredentialLeaseClient,
    AcaCredentialLeaseRequest, AcaDeleteOutcome, AcaDesiredDiskImage, AcaDesiredSandbox,
    AcaDiskImageCandidates, AcaDiskImageId, AcaDiskImageRecord, AcaDiskImageSource, AcaMemoryMib,
    AcaOperationId, AcaProfileId, AcaReadinessPolicy, AcaResourceBinding, AcaRuntimeConfig,
    AcaSandboxCandidates, AcaSandboxId, AcaSandboxLifecycle, AcaSandboxProfile, AcaSandboxRecord,
};
use d2b_provider_runtime_azure_container_apps::{
    AcaClock, AcaController, AcaPhase, AcaReconcileOutcome,
};
use d2b_provider_runtime_cloud_hypervisor::{
    CloudHypervisorClock, CloudHypervisorConfig, CloudHypervisorController,
    CloudHypervisorEffectPort, CloudHypervisorError, CloudHypervisorGuestSettings,
    CloudHypervisorPhase, CloudHypervisorReconcileOutcome, ConsoleType, GuestControlHealth,
    GuestControlHealthError, GuestControlProbe,
    adoption::ProcessIdentity,
    bootstrap_graph::{AttachmentRef, BootstrapGraph},
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

struct FakeCloudEffect {
    identity: Mutex<Option<ProcessIdentity>>,
    fail_launch: bool,
}

#[async_trait]
impl CloudHypervisorEffectPort for FakeCloudEffect {
    async fn launch(
        &self,
        _: &BootstrapGraph,
        _: &CloudHypervisorConfig,
        _: &CloudHypervisorGuestSettings,
    ) -> Result<ProcessIdentity, CloudHypervisorError> {
        if self.fail_launch {
            return Err(CloudHypervisorError::Effect);
        }
        let identity = cloud_identity();
        *self.identity.lock().unwrap() = Some(identity);
        Ok(identity)
    }

    async fn observe(&self) -> Result<Option<ProcessIdentity>, CloudHypervisorError> {
        Ok(*self.identity.lock().unwrap())
    }

    async fn open_pidfd(&self, identity: &ProcessIdentity) -> Result<(), CloudHypervisorError> {
        if self.identity.lock().unwrap().as_ref() != Some(identity) {
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        }
        Ok(())
    }

    async fn stop(&self, identity: &ProcessIdentity) -> Result<(), CloudHypervisorError> {
        let mut current = self.identity.lock().unwrap();
        if current.as_ref() != Some(identity) {
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        }
        *current = None;
        Ok(())
    }
}

fn cloud_identity() -> ProcessIdentity {
    ProcessIdentity {
        pid: 41,
        start_time_ticks: 7,
        cgroup_digest: [1; 32],
        executable_digest: [2; 32],
        template_digest: [3; 32],
        generation: 1,
    }
}

struct FakeGuestControl;

#[async_trait]
impl GuestControlProbe for FakeGuestControl {
    async fn probe(&self, _: u32, _: u32) -> Result<GuestControlHealth, GuestControlHealthError> {
        Ok(GuestControlHealth::Ready)
    }

    async fn close(&self, _: u32) -> Result<(), GuestControlHealthError> {
        Ok(())
    }
}

struct FixedCloudClock;

impl CloudHypervisorClock for FixedCloudClock {
    fn now_unix_ms(&self) -> u64 {
        1_000
    }
}

fn cloud_controller(
    effect: Arc<FakeCloudEffect>,
) -> CloudHypervisorController<FakeCloudEffect, FakeGuestControl> {
    let config = CloudHypervisorConfig {
        controller_execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        default_vcpus: 2,
        default_memory_mb: 512,
        default_machine_type: d2b_contracts_provider::v3::credential::OpaqueAzureRef::parse(
            "q35",
        )
        .unwrap(),
        watchdog: true,
        adoption_window_ms: 30_000,
        health_check_interval_ms: 30_000,
        health_check_timeout_ms: 5_000,
        health_check_failure_threshold: 3,
        startup_deadline_ms: 30_000,
    };
    let settings = CloudHypervisorGuestSettings {
        vcpus: Some(2),
        memory_mb: Some(512),
        machine_type: None,
        console_type: ConsoleType::Null,
        serial_port: true,
        pvpanic: false,
        watchdog_override: None,
        memory_shared: true,
        has_virtiofs_attachment: false,
        system_artifact_id: Some("system-artifact".to_owned()),
    };
    let graph = BootstrapGraph::new(
        vec![ResourceRef::parse("Device/kvm").unwrap()],
        vec![ResourceRef::parse("Network/cloud").unwrap()],
        vec![ResourceRef::parse("Volume/state").unwrap()],
        vec![AttachmentRef::new("launch-ticket").unwrap()],
    )
    .unwrap();
    CloudHypervisorController::new(config, settings, graph, effect, Arc::new(FakeGuestControl))
        .unwrap()
        .with_clock(Arc::new(FixedCloudClock))
}

#[tokio::test]
async fn cloud_composition_reaches_ready_through_production_controllers() {
    let cloud_effect = Arc::new(FakeCloudEffect {
        identity: Mutex::new(None),
        fail_launch: false,
    });
    let mut cloud = cloud_controller(Arc::clone(&cloud_effect));
    assert_eq!(
        cloud.reconcile(false, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Retry { after_ms: 500 }
    );
    assert_eq!(
        cloud.reconcile(true, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Converged
    );
    assert_eq!(cloud.phase(), CloudHypervisorPhase::Ready);

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

    cloud.finalize().await.unwrap();
    assert!(cloud_effect.identity.lock().unwrap().is_none());
}

#[tokio::test]
async fn cloud_composition_fails_closed_on_ambiguous_or_failed_effects() {
    let failed_effect = Arc::new(FakeCloudEffect {
        identity: Mutex::new(None),
        fail_launch: true,
    });
    let mut cloud = cloud_controller(failed_effect);
    assert_eq!(
        cloud.reconcile(true, true, true, 14).await,
        Err(CloudHypervisorError::Effect)
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
