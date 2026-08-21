use std::sync::{Arc, Mutex};
use tokio::time::{Duration, sleep};

use async_trait::async_trait;
use d2b_contracts_provider::v3::credential::OpaqueAzureRef;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_runtime_cloud_hypervisor::{
    CloudHypervisorClock, CloudHypervisorConfig, CloudHypervisorController,
    CloudHypervisorEffectPort, CloudHypervisorGuestSettings, CloudHypervisorPhase,
    CloudHypervisorReconcileOutcome, ConsoleType, GuestControlHealth, GuestControlProbe,
};
use d2b_provider_runtime_cloud_hypervisor::{
    adoption::ProcessIdentity,
    bootstrap_graph::{AttachmentRef, BootstrapGraph},
    health::GuestControlHealthError,
};

#[derive(Default)]
struct FakeState {
    identity: Option<ProcessIdentity>,
    observations: Vec<Option<ProcessIdentity>>,
    launched: bool,
    stopped: bool,
    stop_calls: usize,
    stop_failures: usize,
    stop_leaves_identity: bool,
    launch_delay_ms: u64,
    pidfd_failures: usize,
}

struct FakeEffect {
    state: Arc<Mutex<FakeState>>,
}

struct FixedClock(Arc<Mutex<u64>>);

impl CloudHypervisorClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        *self.0.lock().unwrap()
    }
}

#[async_trait]
impl CloudHypervisorEffectPort for FakeEffect {
    async fn launch(
        &self,
        _: &BootstrapGraph,
        _: &CloudHypervisorConfig,
        _: &CloudHypervisorGuestSettings,
    ) -> Result<ProcessIdentity, d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError> {
        let launch_delay_ms = self.state.lock().unwrap().launch_delay_ms;
        if launch_delay_ms > 0 {
            sleep(Duration::from_millis(launch_delay_ms)).await;
        }
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
        let mut state = self.state.lock().unwrap();
        Ok(state.observations.pop().unwrap_or(state.identity))
    }

    async fn open_pidfd(
        &self,
        _: &ProcessIdentity,
    ) -> Result<(), d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError> {
        let mut state = self.state.lock().unwrap();
        if state.pidfd_failures > 0 {
            state.pidfd_failures -= 1;
            return Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::Effect);
        }
        Ok(())
    }

    async fn stop(
        &self,
        _: &ProcessIdentity,
    ) -> Result<(), d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError> {
        let mut state = self.state.lock().unwrap();
        state.stop_calls += 1;
        if state.stop_failures > 0 {
            state.stop_failures -= 1;
            return Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::Effect);
        }
        state.stopped = true;
        if !state.stop_leaves_identity {
            state.identity = None;
        }
        Ok(())
    }
}

struct ReadyProbe {
    responses: Arc<Mutex<Vec<GuestControlHealth>>>,
    close_calls: Arc<Mutex<Vec<u32>>>,
    delay_ms: u64,
}

impl ReadyProbe {
    fn scripted(responses: Vec<GuestControlHealth>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            close_calls: Arc::new(Mutex::new(Vec::new())),
            delay_ms: 0,
        }
    }

    fn delayed(responses: Vec<GuestControlHealth>, delay_ms: u64) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            close_calls: Arc::new(Mutex::new(Vec::new())),
            delay_ms,
        }
    }
}

#[async_trait]
impl GuestControlProbe for ReadyProbe {
    async fn probe(&self, _: u32, _: u32) -> Result<GuestControlHealth, GuestControlHealthError> {
        if self.delay_ms > 0 {
            sleep(Duration::from_millis(self.delay_ms)).await;
        }
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(GuestControlHealth::Ready))
    }

    async fn close(&self, cid: u32) -> Result<(), GuestControlHealthError> {
        self.close_calls.lock().unwrap().push(cid);
        Ok(())
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

fn controller_with_probe(
    state: Arc<Mutex<FakeState>>,
    probe: ReadyProbe,
) -> CloudHypervisorController<FakeEffect, ReadyProbe> {
    controller_with_probe_and_deadline(state, probe, 120_000)
}

fn controller_with_probe_and_deadline(
    state: Arc<Mutex<FakeState>>,
    probe: ReadyProbe,
    startup_deadline_ms: u32,
) -> CloudHypervisorController<FakeEffect, ReadyProbe> {
    controller_with_windows(state, probe, startup_deadline_ms, 30_000)
}

fn controller_with_windows(
    state: Arc<Mutex<FakeState>>,
    probe: ReadyProbe,
    startup_deadline_ms: u32,
    adoption_window_ms: u32,
) -> CloudHypervisorController<FakeEffect, ReadyProbe> {
    let config = CloudHypervisorConfig {
        controller_execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        default_vcpus: 2,
        default_memory_mb: 512,
        default_machine_type: OpaqueAzureRef::parse("q35").unwrap(),
        watchdog: true,
        adoption_window_ms,
        health_check_interval_ms: 30_000,
        health_check_timeout_ms: 5_000,
        health_check_failure_threshold: 3,
        startup_deadline_ms,
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
        Arc::new(probe),
    )
    .unwrap()
}

fn controller(state: Arc<Mutex<FakeState>>) -> CloudHypervisorController<FakeEffect, ReadyProbe> {
    controller_with_probe(state, ReadyProbe::scripted(Vec::new()))
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
async fn failed_pidfd_open_stops_the_newly_launched_vmm() {
    let state = Arc::new(Mutex::new(FakeState {
        pidfd_failures: 1,
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));

    assert_eq!(
        controller.reconcile(true, true, true, 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::Effect)
    );
    let state = state.lock().unwrap();
    assert_eq!(state.stop_calls, 1);
    assert!(state.stopped);
    assert!(
        !controller.finalizer_installed() || controller.phase() == CloudHypervisorPhase::Failed
    );
}

#[tokio::test]
async fn restart_rejects_stale_generation_before_pidfd_open() {
    let state = Arc::new(Mutex::new(FakeState {
        identity: Some(identity()),
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    let mut expected = identity();
    expected.generation = 2;
    assert!(matches!(
        controller.adopt(expected, 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::AdoptionAmbiguous)
    ));
    assert_eq!(controller.phase(), CloudHypervisorPhase::Degraded);
}

#[tokio::test]
async fn adoption_window_is_not_reset_between_retries() {
    let state = Arc::new(Mutex::new(FakeState {
        identity: Some(identity()),
        observations: vec![Some(identity()), None],
        ..FakeState::default()
    }));
    let mut controller = controller_with_windows(
        Arc::clone(&state),
        ReadyProbe::scripted(Vec::new()),
        120_000,
        20,
    );

    assert!(matches!(
        controller.adopt(identity(), 14).await,
        Ok(CloudHypervisorReconcileOutcome::Retry { .. })
    ));
    sleep(Duration::from_millis(30)).await;
    assert_eq!(
        controller.adopt(identity(), 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::AdoptionAmbiguous)
    );
    assert_eq!(controller.phase(), CloudHypervisorPhase::Degraded);
}

#[tokio::test]
async fn adoption_window_survives_controller_restart() {
    let now = Arc::new(Mutex::new(1_000));
    let state = Arc::new(Mutex::new(FakeState {
        identity: Some(identity()),
        observations: vec![None],
        ..FakeState::default()
    }));
    let mut controller = controller_with_windows(
        Arc::clone(&state),
        ReadyProbe::scripted(Vec::new()),
        120_000,
        20,
    )
    .with_clock(Arc::new(FixedClock(Arc::clone(&now))));

    assert!(matches!(
        controller.adopt(identity(), 14).await,
        Ok(CloudHypervisorReconcileOutcome::Retry { .. })
    ));
    let recovery = controller.recovery_state();
    *now.lock().unwrap() = 1_030;
    let mut restored =
        controller_with_windows(state, ReadyProbe::scripted(Vec::new()), 120_000, 20)
            .with_clock(Arc::new(FixedClock(Arc::clone(&now))))
            .restore_recovery_state(recovery)
            .unwrap();
    assert_eq!(
        restored.adopt(identity(), 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::AdoptionAmbiguous)
    );
    assert_eq!(restored.phase(), CloudHypervisorPhase::Degraded);
}

#[tokio::test]
async fn observed_process_without_durable_identity_is_rejected() {
    let state = Arc::new(Mutex::new(FakeState {
        identity: Some(identity()),
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    assert!(matches!(
        controller.reconcile(true, true, true, 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::AdoptionAmbiguous)
    ));
}

#[tokio::test]
async fn failed_stop_retains_identity_for_retry() {
    let state = Arc::new(Mutex::new(FakeState {
        stop_failures: 1,
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    controller.reconcile(true, true, true, 14).await.unwrap();
    assert_eq!(
        controller.finalize().await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::Effect)
    );
    assert!(controller.finalizer_installed());
    controller.finalize().await.unwrap();
    assert!(!controller.finalizer_installed());
    assert_eq!(state.lock().unwrap().stop_calls, 2);
}

#[tokio::test]
async fn finalization_requires_observing_process_exit() {
    let state = Arc::new(Mutex::new(FakeState {
        stop_leaves_identity: true,
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    controller.reconcile(true, true, true, 14).await.unwrap();
    assert_eq!(
        controller.finalize().await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::Effect)
    );
    assert!(controller.finalizer_installed());
    state.lock().unwrap().stop_leaves_identity = false;
    controller.finalize().await.unwrap();
    assert!(!controller.finalizer_installed());
}

#[tokio::test]
async fn absent_process_still_closes_guest_control() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let probe = ReadyProbe::scripted(Vec::new());
    let close_calls = Arc::clone(&probe.close_calls);
    let mut controller = controller_with_probe(Arc::clone(&state), probe);
    controller.reconcile(true, true, true, 14).await.unwrap();
    state.lock().unwrap().identity = None;

    controller.finalize().await.unwrap();

    assert_eq!(*close_calls.lock().unwrap(), vec![14]);
    assert!(!controller.finalizer_installed());
}

#[tokio::test]
async fn degraded_health_requires_threshold_before_phase_change() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut controller = controller_with_probe(
        state,
        ReadyProbe::scripted(vec![
            GuestControlHealth::Ready,
            GuestControlHealth::Degraded,
            GuestControlHealth::Degraded,
        ]),
    );
    assert!(matches!(
        controller.reconcile(true, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Retry { .. }
    ));
    assert_eq!(controller.phase(), CloudHypervisorPhase::Bootstrapping);
    assert!(matches!(
        controller.reconcile(true, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Retry { .. }
    ));
    assert_eq!(controller.phase(), CloudHypervisorPhase::Bootstrapping);
    assert_eq!(
        controller.reconcile(true, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), CloudHypervisorPhase::Ready);
}

#[tokio::test]
async fn launch_is_cancelled_at_startup_deadline() {
    let state = Arc::new(Mutex::new(FakeState {
        launch_delay_ms: 100,
        ..FakeState::default()
    }));
    let mut controller =
        controller_with_probe_and_deadline(state, ReadyProbe::scripted(Vec::new()), 20);
    assert_eq!(
        controller.reconcile(true, true, true, 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::StartupDeadlineExceeded)
    );
    assert_eq!(controller.phase(), CloudHypervisorPhase::Failed);
}

#[tokio::test]
async fn initial_guest_control_probe_is_cancelled_at_startup_deadline() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut controller =
        controller_with_probe_and_deadline(state, ReadyProbe::delayed(Vec::new(), 100), 20);
    assert_eq!(
        controller.reconcile(true, true, true, 14).await,
        Err(d2b_provider_runtime_cloud_hypervisor::CloudHypervisorError::StartupDeadlineExceeded)
    );
    assert_eq!(controller.phase(), CloudHypervisorPhase::Failed);
}
