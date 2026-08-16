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

#[test]
fn wire_deserialization_reuses_notification_validation() {
    let mut value = serde_json::to_value(
        NotificationRequest::new("summary", "body", Category::SystemInfo).unwrap(),
    )
    .unwrap();
    value["iconRef"] = serde_json::json!("/run/user/1000/icon");
    assert!(serde_json::from_value::<NotificationRequest>(value).is_err());
}
