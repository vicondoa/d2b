use d2b_provider_display_wayland::{DisplayController, FinalizationInput, Phase};

#[test]
fn ambiguous_process_termination_retains_the_finalizer() {
    let decision = DisplayController::finalize(FinalizationInput {
        stop_requested: true,
        proxy_terminal: false,
        proxy_deleted: false,
        volume_deleted: false,
        principal_released: false,
        portal_revoked: false,
        grace_expired: true,
    });
    assert_eq!(decision.phase, Phase::Degraded);
    assert!(decision.ambiguous);
    assert!(!decision.remove_finalizer);
}
