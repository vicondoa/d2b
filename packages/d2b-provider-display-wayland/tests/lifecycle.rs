#[test]
fn display_finalizer_is_fixed_and_state_volume_free() {
    assert_eq!(
        d2b_provider_display_wayland::DisplayController::finalizer(),
        "display-wayland.d2bus.org/proxy-stopped"
    );
    assert!(d2b_provider_display_wayland::DisplayController::provider_state_set_empty());
}
