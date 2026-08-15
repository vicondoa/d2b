use std::sync::Arc;

use d2b_process_conformance::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionCandidate, IdentityBinding, LaunchTicket, LaunchedProcess, PidfdEvidence,
    ProcessConformanceError, ProcessIdentityDigest, ProcessLaunchEffectPort, StopClass,
    WaitReapOwner,
};
use d2b_provider_system_systemd::controller::{
    SystemdProcessController, SystemdReconcileAction, SystemdReconcileResult,
};
use d2b_provider_system_systemd::{SystemdProcessProvider, SystemdProviderConfig};

#[derive(Debug)]
struct PendingEffectPort {
    launch_entered: Arc<tokio::sync::Notify>,
}

impl PendingEffectPort {
    fn new() -> Self {
        Self {
            launch_entered: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

impl ProcessLaunchEffectPort for PendingEffectPort {
    async fn launch(
        &self,
        _ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        self.launch_entered.notify_one();
        std::future::pending().await
    }

    async fn observe(
        &self,
        _ticket: &LaunchTicket,
    ) -> Result<Option<AdoptionCandidate>, ProcessConformanceError> {
        std::future::pending().await
    }

    async fn open_pidfd(
        &self,
        _candidate: &AdoptionCandidate,
    ) -> Result<PidfdEvidence, ProcessConformanceError> {
        std::future::pending().await
    }

    async fn stop(
        &self,
        _identity: &ProcessIdentityDigest,
        _class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        std::future::pending().await
    }
}

fn required() -> Vec<IdentityBinding> {
    vec![
        IdentityBinding::UnitInvocationId,
        IdentityBinding::Cgroup,
        IdentityBinding::UnitMainPid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]
}

#[test]
fn controller_dispatches_start_adopt_and_stop() {
    let ticket = fixtures::ticket_builder()
        .expected_identity(required())
        .build()
        .expect("valid ticket");

    let start_port = ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager);
    let start_controller =
        SystemdProcessController::new(SystemdProcessProvider::new(start_port), Default::default());
    let started = block_on(start_controller.reconcile(SystemdReconcileAction::Start(&ticket)))
        .expect("start result");
    assert!(matches!(started, SystemdReconcileResult::Started(_)));
    assert_eq!(
        start_controller.provider().port().calls(),
        vec![PortCall::Launch]
    );

    let adopt_port = ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
        .with_candidate(required(), WaitReapOwner::ServiceManager);
    let adopt_controller =
        SystemdProcessController::new(SystemdProcessProvider::new(adopt_port), Default::default());
    let adopted = block_on(adopt_controller.reconcile(SystemdReconcileAction::Adopt(&ticket)))
        .expect("adoption result");
    assert!(matches!(adopted, SystemdReconcileResult::Adoption(_)));
    assert_eq!(
        adopt_controller.provider().port().calls(),
        vec![PortCall::Observe, PortCall::OpenPidfd]
    );

    let stop_port = ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager);
    let stop_controller =
        SystemdProcessController::new(SystemdProcessProvider::new(stop_port), Default::default());
    let identity = ProcessIdentityDigest::from_bytes([0x22; 32]);
    let stopped = block_on(
        stop_controller.reconcile(SystemdReconcileAction::Stop(&identity, StopClass::Drain)),
    )
    .expect("stop result");
    assert!(matches!(stopped, SystemdReconcileResult::Stopped));
    assert_eq!(
        stop_controller.provider().port().calls(),
        vec![PortCall::Stop(StopClass::Drain)]
    );
}

#[test]
fn controller_rejects_launches_when_the_bounded_permit_is_saturated() {
    let ticket = fixtures::ticket_builder()
        .expected_identity(required())
        .build()
        .expect("valid ticket");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let port = PendingEffectPort::new();
        let entered = Arc::clone(&port.launch_entered);
        let controller = Arc::new(SystemdProcessController::new(
            SystemdProcessProvider::new(port),
            SystemdProviderConfig::new(1, 1, 1, 1).expect("bounded config"),
        ));
        let first_controller = Arc::clone(&controller);
        let first_ticket = ticket.clone();
        let first = tokio::spawn(async move {
            first_controller
                .reconcile(SystemdReconcileAction::Start(&first_ticket))
                .await
        });
        entered.notified().await;

        assert_eq!(
            controller
                .reconcile(SystemdReconcileAction::Start(&ticket))
                .await
                .unwrap_err(),
            ProcessConformanceError::DeadlineExceeded
        );
        first.abort();
    });
}

#[test]
fn controller_times_out_pending_start_adopt_and_stop_operations() {
    let ticket = fixtures::ticket_builder()
        .expected_identity(required())
        .build()
        .expect("valid ticket");
    let identity = ProcessIdentityDigest::from_bytes([0x22; 32]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let start_controller = SystemdProcessController::new(
            SystemdProcessProvider::new(PendingEffectPort::new()),
            SystemdProviderConfig {
                launch_timeout_sec: 0,
                termination_grace_sec: 0,
                user_manager_check_timeout: 0,
                max_concurrent_launches: 1,
            },
        );
        assert_eq!(
            start_controller
                .reconcile(SystemdReconcileAction::Start(&ticket))
                .await
                .unwrap_err(),
            ProcessConformanceError::DeadlineExceeded
        );
        assert_eq!(
            start_controller
                .reconcile(SystemdReconcileAction::Adopt(&ticket))
                .await
                .unwrap_err(),
            ProcessConformanceError::DeadlineExceeded
        );

        let stop_controller = SystemdProcessController::new(
            SystemdProcessProvider::new(PendingEffectPort::new()),
            SystemdProviderConfig {
                launch_timeout_sec: 1,
                termination_grace_sec: 1,
                user_manager_check_timeout: 1,
                max_concurrent_launches: 1,
            },
        );
        assert_eq!(
            stop_controller
                .reconcile(SystemdReconcileAction::Stop(&identity, StopClass::Drain))
                .await
                .unwrap_err(),
            ProcessConformanceError::DeadlineExceeded
        );
    });
}
