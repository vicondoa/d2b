use d2b_provider_notification_desktop::{Category, NotificationRequest};

#[test]
fn unknown_icon_paths_are_not_notification_ids() {
    assert!(
        NotificationRequest::new("summary", "body", Category::SystemInfo)
            .unwrap()
            .with_icon_ref("/run/user/1000/icon")
            .is_err()
    );
}
