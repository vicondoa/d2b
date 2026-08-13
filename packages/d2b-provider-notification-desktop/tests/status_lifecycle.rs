use d2b_provider_notification_desktop::NotificationController;

#[test]
fn notification_provider_has_no_state_volume() {
    let controller = NotificationController::new("Provider/notification-desktop").unwrap();
    assert!(controller.provider_state_set_empty());
}
