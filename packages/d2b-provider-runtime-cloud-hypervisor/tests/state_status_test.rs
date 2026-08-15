use d2b_provider_runtime_cloud_hypervisor::state::GuestRuntimeStatus;

#[test]
fn status_is_bounded_and_does_not_carry_process_identity() {
    let status = GuestRuntimeStatus {
        phase: "ready",
        runtime_ready: true,
        bootstrap_ready: true,
        active_process_count: 1,
    };
    let debug = format!("{status:?}");
    assert!(debug.contains("ready"));
    assert!(!debug.contains("pid"));
    assert!(!debug.contains("argv"));
}
