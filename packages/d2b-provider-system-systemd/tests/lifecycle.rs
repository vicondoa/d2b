use d2b_process_conformance::{ProcessExitClass, ProcessOutcome};
use d2b_provider_system_systemd::{
    EphemeralProcessController, RestartPolicy, SystemdProviderConfig,
};

#[test]
fn provider_config_is_bounded_and_transient() {
    let config = SystemdProviderConfig::default();
    assert_eq!(config.launch_timeout_sec, 30);
    assert!(SystemdProviderConfig::new(0, 30, 5, 64).is_err());
    assert!(SystemdProviderConfig::new(30, 30, 5, 256).is_ok());
    assert!(config.no_persistent_unit());
}

#[test]
fn restart_policy_is_bounded_and_does_not_restart_clean_exit() {
    let mut policy = RestartPolicy::on_failure(2, 1000);
    assert!(!policy.should_restart(ProcessOutcome::exited(0).unwrap()));
    assert!(policy.should_restart(ProcessOutcome::crashed()));
    assert!(policy.should_restart(ProcessOutcome::signaled()));
    assert!(!policy.should_restart(ProcessOutcome::crashed()));
    policy.tick(1000);
    assert_eq!(policy.attempts(), 0);
    assert_eq!(policy.healthy_ticks(), 0);
    assert!(policy.should_restart(ProcessOutcome::crashed()));
}

#[test]
fn ephemeral_process_maps_exit_and_ttl_without_pid1_ownership() {
    let mut process = EphemeralProcessController::new(10, false);
    assert_eq!(
        process.observe(ProcessOutcome::exited(0).unwrap()),
        ProcessExitClass::CleanExit
    );
    assert_eq!(process.ttl_remaining(), Some(10));
    process.tick(10);
    assert!(process.cleanup_eligible());
    assert!(!process.owns_persistent_unit());
}

#[test]
fn repeated_terminal_observation_does_not_extend_cleanup_ttl() {
    let mut process = EphemeralProcessController::new(10, false);
    process.observe(ProcessOutcome::exited(0).unwrap());
    process.tick(4);
    process.observe(ProcessOutcome::crashed());
    assert_eq!(process.ttl_remaining(), Some(6));
}
