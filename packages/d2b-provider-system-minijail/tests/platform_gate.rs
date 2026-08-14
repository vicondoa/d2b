use d2b_process_conformance::ProcessConformanceError;
use d2b_process_conformance::testing::{ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{IdentityBinding, ProcessProvider, WaitReapOwner};
use d2b_provider_system_minijail::{MinijailProcessProvider, PROVIDER_NAME, launch::PlatformGate};

#[test]
fn unsupported_kernel_or_cgroup_kill_refuses_before_launch() {
    assert_eq!(
        PlatformGate::new_for_test(5, 13, true)
            .validate()
            .unwrap_err(),
        ProcessConformanceError::PlatformGateRejected
    );
    assert_eq!(
        PlatformGate::new_for_test(6, 1, false)
            .validate()
            .unwrap_err(),
        ProcessConformanceError::PlatformGateRejected
    );
}

#[test]
fn injected_platform_gate_rejects_before_effect_dispatch() {
    let provider = MinijailProcessProvider::with_platform_gate(
        ScriptedEffectPort::launching(
            vec![
                IdentityBinding::Pid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Cgroup,
                IdentityBinding::Executable,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ],
            WaitReapOwner::Local,
        ),
        PlatformGate::new_for_test(5, 13, true),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(vec![
            IdentityBinding::Pid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Cgroup,
            IdentityBinding::Executable,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ])
        .build()
        .expect("conformant ticket");

    assert_eq!(
        block_on(provider.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::PlatformGateRejected
    );
    assert!(provider.port().calls().is_empty());
}
