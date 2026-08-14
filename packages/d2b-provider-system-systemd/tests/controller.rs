use d2b_process_conformance::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{IdentityBinding, ProcessIdentityDigest, StopClass, WaitReapOwner};
use d2b_provider_system_systemd::SystemdProcessProvider;
use d2b_provider_system_systemd::controller::{
    SystemdProcessController, SystemdReconcileAction, SystemdReconcileResult,
};

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
