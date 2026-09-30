use std::collections::BTreeMap;

use d2b_provider_notification_desktop::{
    ActionSpec, Category, NotificationError, NotificationOutcome, NotificationProjection,
    NotificationRequest, NotificationResult, NotificationTelemetryField,
    NotificationTelemetryFrame, ProviderError,
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
        Err(ProviderError::TelemetryFieldRejected)
    );
}

// --- U28: content never becomes authority-bearing invocation metadata ---

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_notification_desktop::{
    NOTIFICATION_SERVICE, NotificationEndpointRole, NotificationHostEndpoints,
    notification_endpoint_bindings,
};

#[test]
fn notification_content_cannot_become_an_endpoint_or_service_authority() {
    const CANARY: &str = "notif-authority-canary-9be2";
    let endpoints = NotificationHostEndpoints::new(
        ResourceRef::parse("Endpoint/notification-guest-source").expect("endpoint"),
        ResourceRef::parse("Endpoint/notification-desktop-sink").expect("endpoint"),
    )
    .expect("declared endpoints");
    let consumer = ResourceRef::parse("Process/notification-sink").expect("consumer");

    // A request whose every content field spells out an authority token the
    // Provider owns.
    let request = NotificationRequest::new(
        NotificationEndpointRole::DesktopSink.purpose(),
        NotificationEndpointRole::GuestSource.slot(),
        Category::SystemInfo,
    )
    .unwrap()
    .with_actions(vec![
        ActionSpec::new("open", NotificationEndpointRole::GuestSource.purpose()).unwrap(),
    ])
    .unwrap()
    .with_idempotency_key(NotificationEndpointRole::DesktopSink.slot())
    .unwrap();
    let sanitized = request.sanitize().unwrap();
    assert!(sanitized.summary().contains(NotificationEndpointRole::DesktopSink.purpose()));

    for declared in notification_endpoint_bindings(&endpoints, &consumer).expect("bindings") {
        let rendered = format!("{:?}", declared.request());
        assert!(!rendered.contains(CANARY), "payload leaked into {rendered}");
        assert_eq!(declared.request().slot().as_str(), declared.role().slot());
        assert_eq!(
            declared.request().purpose().as_str(),
            declared.role().purpose()
        );
        assert!(NOTIFICATION_SERVICE.streams.contains(&declared.role().stream()));
    }
    for role in NotificationEndpointRole::ALL {
        assert!(!role.slot().contains(CANARY));
        assert!(!role.purpose().contains(CANARY));
    }
    // A source that is not an `Endpoint` row is refused rather than admitted
    // as a presentation channel.
    assert!(
        NotificationHostEndpoints::new(
            ResourceRef::parse("Process/notification-sink").expect("consumer"),
            ResourceRef::parse("Endpoint/notification-desktop-sink").expect("endpoint"),
        )
        .is_err()
    );
}
