use d2b_process_conformance::ProcessOutcome;
use d2b_provider_system_minijail::ephemeral::EphemeralProcessState;

#[test]
fn ephemeral_cleanup_waits_for_terminal_ttl_and_no_incident_hold() {
    let mut state = EphemeralProcessState::new(false);
    assert!(!state.cleanup_eligible());
    state.observe(ProcessOutcome::exited(0).expect("valid exit"), 3);
    assert!(!state.cleanup_eligible());
    state.tick(2);
    assert!(!state.cleanup_eligible());
    state.tick(2);
    assert!(state.cleanup_eligible());
}

#[test]
fn incident_hold_and_saturating_ticks_keep_cleanup_blocked() {
    let mut state = EphemeralProcessState::new(true);
    state.observe(ProcessOutcome::exited(1).expect("valid exit"), 1);
    state.tick(u64::MAX);
    assert!(!state.cleanup_eligible());
}
