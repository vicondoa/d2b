use d2b_process_conformance::ProcessConformanceError;
use d2b_provider_system_minijail::launch::PlatformGate;

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
