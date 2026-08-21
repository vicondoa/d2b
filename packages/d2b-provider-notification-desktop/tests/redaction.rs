use std::collections::BTreeMap;

use d2b_provider_notification_desktop::{
    ActionSpec, Category, NotificationError, NotificationOutcome, NotificationProjection,
    NotificationRequest, NotificationResult, NotificationTelemetryField,
    NotificationTelemetryFrame,
};

#[test]
fn notification_canary_stays_out_of_debug_errors_and_telemetry() {
    const CANARY: &str = "notification-payload-canary-7f4a";
    let request = NotificationRequest::new(
        format!("summary-{CANARY}"),
        format!("body-{CANARY}"),
        Category::SystemInfo,
    )
    .unwrap()
    .with_actions(vec![
        ActionSpec::new("open", format!("action-{CANARY}")).unwrap(),
    ])
    .unwrap();
    let sanitized = request.sanitize().unwrap();
    let projection = NotificationProjection {
        request_id: "notification-1".to_owned(),
        notification: sanitized,
    };
    let result = NotificationResult::Accepted {
        notification_id: 1,
        action_nonces: BTreeMap::from([("open".to_owned(), "opaque-action-key".to_owned())]),
    };
    let frame = NotificationTelemetryFrame::new(
        "work",
        Category::SystemInfo,
        NotificationOutcome::Accepted,
    );

    for rendered in [
        format!("{projection:?}"),
        format!("{result:?}"),
        format!("{frame:?}"),
        NotificationError::InvalidActions.to_string(),
    ] {
        assert!(!rendered.contains(CANARY), "payload leaked into {rendered}");
    }
    assert_eq!(
        NotificationTelemetryFrame::validate_collector_fields([NotificationTelemetryField {
            key: "summary",
            value: CANARY.to_owned(),
        },]),
        Err("notification-telemetry-field-rejected")
    );
}
