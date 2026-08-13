use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts::v3::{ResourceRef, credential::OpaqueAzureRef};
use d2b_provider_runtime_cloud_hypervisor::{
    CloudHypervisorConfig, CloudHypervisorController, CloudHypervisorEffectPort,
    CloudHypervisorGuestSettings, CloudHypervisorPhase, CloudHypervisorReconcileOutcome,
    ConsoleType, GuestControlHealth, GuestControlProbe,
};
use d2b_provider_runtime_cloud_hypervisor::{
    adoption::ProcessIdentity,
    bootstrap_graph::{AttachmentRef, BootstrapGraph},
    health::GuestControlHealthError,
};

#[derive(Default)]
struct FakeState {
    identity: Option<ProcessIdentity>,
    launched: bool,
    stopped: bool,
}

struct FakeEffect {
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl CloudHypervisorEffectPort for FakeEffect {
    async fn launch(
        &self,
        _: &BootstrapGraph,
        _: &CloudHypervisorConfig,
        _: &CloudHypervisorGuestSettings,
    ) -> Result<ProcessIdentity, d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError> {
        let mut state = self.state.lock().unwrap();
        state.launched = true;
        let identity = identity();
        state.identity = Some(identity);
        Ok(identity)
    }

    async fn observe(
        &self,
    ) -> Result<Option<ProcessIdentity>, d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError>
    {
        Ok(self.state.lock().unwrap().identity)
    }

    async fn open_pidfd(
        &self,
        _: &ProcessIdentity,
    ) -> Result<(), d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError> {
        Ok(())
    }

    async fn stop(
        &self,
        _: &ProcessIdentity,
    ) -> Result<(), d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError> {
        self.state.lock().unwrap().stopped = true;
        Ok(())
    }
}

struct ReadyProbe;

#[async_trait]
impl GuestControlProbe for ReadyProbe {
    async fn probe(&self, _: u32, _: u32) -> Result<GuestControlHealth, GuestControlHealthError> {
        Ok(GuestControlHealth::Ready)
    }
}

fn identity() -> ProcessIdentity {
    ProcessIdentity {
        pid: 42,
        start_time_ticks: 7,
        cgroup_digest: [1; 32],
        executable_digest: [2; 32],
        template_digest: [3; 32],
        generation: 1,
    }
}

fn controller(state: Arc<Mutex<FakeState>>) -> CloudHypervisorController<FakeEffect, ReadyProbe> {
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
        vec![ResourceRef::parse("Network/work").unwrap()],
        vec![ResourceRef::parse("Volume/store").unwrap()],
        vec![AttachmentRef::new("launch-ticket").unwrap()],
    )
    .unwrap();
    CloudHypervisorController::new(
        config,
        settings,
        graph,
        Arc::new(FakeEffect { state }),
        Arc::new(ReadyProbe),
    )
    .unwrap()
}

#[tokio::test]
async fn dependency_barrier_prevents_process_launch() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut controller = controller(Arc::clone(&state));
    assert!(matches!(
        controller.reconcile(false, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Retry { .. }
    ));
    assert_eq!(controller.phase(), CloudHypervisorPhase::Pending);
    assert!(!state.lock().unwrap().launched);
}

#[tokio::test]
async fn launch_requires_authenticated_guest_control_before_ready() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut controller = controller(Arc::clone(&state));
    assert_eq!(
        controller.reconcile(true, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), CloudHypervisorPhase::Ready);
    controller.finalize().await.unwrap();
    assert!(state.lock().unwrap().stopped);
    assert!(!controller.finalizer_installed());
}

#[tokio::test]
async fn restart_rejects_stale_generation_before_pidfd_open() {
    let state = Arc::new(Mutex::new(FakeState {
        identity: Some(identity()),
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    let mut expected = identity();
    expected.generation = 2;
    assert!(matches!(
        controller.adopt(expected, 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::AdoptionAmbiguous)
    ));
    assert_eq!(controller.phase(), CloudHypervisorPhase::Degraded);
}
