use d2b_provider_notification_desktop::{
    ActionNonceStore, ActionSpec, AdmissionPurpose, Category, DesktopNotificationPort,
    NotificationController, NotificationProviderDescriptor, NotificationRequest,
    NotificationResult, NotificationSink, NotificationUrgency, SessionEvidence, TransportClass,
};

#[derive(Default)]
struct RecordingSink {
    accepted: Vec<String>,
}

impl DesktopNotificationPort for RecordingSink {
    fn notify(
        &mut self,
        notification: &d2b_provider_notification_desktop::SanitizedNotification,
    ) -> Result<u32, d2b_provider_notification_desktop::SinkError> {
        self.accepted.push(notification.summary().to_owned());
        Ok(self.accepted.len() as u32)
    }
}

struct FailingSink;

impl DesktopNotificationPort for FailingSink {
    fn notify(
        &mut self,
        _notification: &d2b_provider_notification_desktop::SanitizedNotification,
    ) -> Result<u32, d2b_provider_notification_desktop::SinkError> {
        Err(d2b_provider_notification_desktop::SinkError::Unavailable)
    }
}

fn request() -> NotificationRequest {
    NotificationRequest::new(
        "hello\nworld",
        "body\twith content",
        Category::SecurityEvent,
    )
    .unwrap()
    .with_urgency(NotificationUrgency::Critical)
    .unwrap()
}

#[test]
fn request_bounds_and_sanitization_are_closed() {
    let request = request();
    let sanitized = request.sanitize().unwrap();
    assert_eq!(sanitized.summary(), "hello world");
    assert_eq!(sanitized.body(), "body with content");
    assert!(
        NotificationRequest::new("x", "y", Category::SecurityEvent)
            .unwrap()
            .with_icon_ref("../secret")
            .is_err()
    );
}

#[test]
fn action_ids_use_the_machine_id_bound() {
    assert!(ActionSpec::new("a".repeat(32), "label").is_ok());
    assert!(ActionSpec::new("a".repeat(33), "label").is_err());
    assert!(ActionSpec::new("open", "l".repeat(64)).is_ok());
    assert!(ActionSpec::new("open", "l".repeat(65)).is_err());
}

#[test]
fn wire_defaults_match_the_notification_contract() {
    let request: NotificationRequest = serde_json::from_value(serde_json::json!({
        "summary": "hello",
        "category": "system-info"
    }))
    .unwrap();
    assert_eq!(request.urgency(), NotificationUrgency::Normal);
    assert_eq!(request.expire_timeout_secs(), 0);
    assert!(request.actions().is_empty());
    assert!(request.icon_ref().is_none());
}

#[test]
fn action_nonces_are_single_use_ttl_bound_and_opaque() {
    let mut store = ActionNonceStore::new(2, 10);
    let nonce = store.register("session", "cancel", 100).unwrap();
    assert!(format!("{nonce:?}").contains("REDACTED"));
    let key = nonce.action_key();
    assert!(store.consume(&key, "session", 101).is_ok());
    assert!(store.consume(&key, "session", 101).is_err());
    let expired = store.register("session", "open", 100).unwrap();
    assert!(
        store
            .consume(&expired.action_key(), "session", 110)
            .is_err()
    );
    assert!(store.is_empty());
}

#[test]
fn action_id_mismatch_does_not_consume_a_live_capability() {
    let mut store = ActionNonceStore::new(2, 10);
    let nonce = store.register("session", "cancel", 100).unwrap();
    assert!(
        store
            .consume_for_action(&nonce.action_key(), "session", Some("open"), 101)
            .is_err()
    );
    assert!(
        store
            .consume_for_action(&nonce.action_key(), "session", Some("cancel"), 101)
            .is_ok()
    );
}

#[test]
fn stream_admission_rejects_wrong_profile_and_accepts_enrolled_source() {
    let source = SessionEvidence::new(
        true,
        true,
        "d2b.notification.v3",
        AdmissionPurpose::NotificationSource,
        TransportClass::EnrolledNoiseKk,
    );
    assert!(source.admit().is_ok());
    let observer = SessionEvidence::new(
        true,
        true,
        "d2b.notification.v2",
        AdmissionPurpose::DesktopObserver,
        TransportClass::UnixSeqpacket,
    );
    assert!(observer.admit().is_err());
}

#[test]
fn host_sink_delivers_redacted_content_and_returns_nonce_metadata_only() {
    let mut sink = NotificationSink::new(4, 2, 10);
    let mut port = RecordingSink::default();
    let result = sink
        .deliver(&mut port, "observer-a", request(), 100)
        .unwrap();
    assert!(matches!(result, NotificationResult::Accepted { .. }));
    assert_eq!(port.accepted, vec!["hello world"]);
    assert!(!format!("{sink:?}").contains("hello"));
}

#[test]
fn expired_action_nonces_do_not_block_new_capacity() {
    let mut sink = NotificationSink::new(2, 1, 10);
    let mut port = RecordingSink::default();
    let first = NotificationRequest::new("first", "body", Category::SystemInfo)
        .unwrap()
        .with_actions(vec![ActionSpec::new("open", "Open").unwrap()])
        .unwrap();
    assert!(matches!(
        sink.deliver(&mut port, "observer-a", first, 100)
            .unwrap(),
        NotificationResult::Accepted { .. }
    ));

    let second = NotificationRequest::new("second", "body", Category::SystemInfo).unwrap();
    assert!(matches!(
        sink.deliver(&mut port, "observer-a", second, 111)
            .unwrap(),
        NotificationResult::Accepted { .. }
    ));
}

#[test]
fn idempotency_keys_return_the_original_delivery_result() {
    let mut sink = NotificationSink::new(4, 4, 10);
    let mut port = RecordingSink::default();
    let request = NotificationRequest::new("summary", "body", Category::SystemInfo)
        .unwrap()
        .with_actions(vec![ActionSpec::new("open", "Open").unwrap()])
        .unwrap()
        .with_idempotency_key("same-request")
        .unwrap();
    let first = sink
        .deliver(&mut port, "observer-a", request.clone(), 100)
        .unwrap();
    let second = sink
        .deliver(&mut port, "observer-a", request, 101)
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(port.accepted, vec!["summary"]);
}

#[test]
fn sink_failure_does_not_evict_the_existing_projection() {
    let mut sink = NotificationSink::new(1, 4, 10);
    let mut recording = RecordingSink::default();
    let first = NotificationRequest::new("first", "body", Category::SystemInfo).unwrap();
    assert!(matches!(
        sink.deliver(&mut recording, "observer-a", first, 100)
            .unwrap(),
        NotificationResult::Accepted { .. }
    ));

    let second = NotificationRequest::new("second", "body", Category::SystemInfo).unwrap();
    let mut failing = FailingSink;
    assert_eq!(
        sink.deliver(&mut failing, "observer-a", second, 101)
            .unwrap(),
        NotificationResult::SinkUnavailable
    );
    assert_eq!(sink.projection_len(), 1);
}

#[test]
fn action_capabilities_bind_to_observer_and_projection_lifecycle() {
    let request = NotificationRequest::new("summary", "body", Category::SystemInfo)
        .unwrap()
        .with_actions(vec![ActionSpec::new("open", "Open").unwrap()])
        .unwrap();
    let mut sink = NotificationSink::new(1, 4, 10);
    let mut port = RecordingSink::default();
    let result = sink.deliver(&mut port, "observer-a", request, 100).unwrap();
    let action_key = match result {
        NotificationResult::Accepted { action_nonces, .. } => action_nonces["open"].clone(),
        other => panic!("unexpected result: {other:?}"),
    };
    assert_eq!(
        sink.invoke_action(&action_key, "observer-b", 101),
        Err(d2b_provider_notification_desktop::ActionNonceError::SessionMismatch)
    );
    assert_eq!(
        sink.invoke_action_for(&action_key, "observer-a", "open", 101),
        Ok("open".to_owned())
    );

    let result = sink
        .deliver(
            &mut port,
            "observer-a",
            NotificationRequest::new("summary", "body", Category::SystemInfo)
                .unwrap()
                .with_actions(vec![ActionSpec::new("open", "Open").unwrap()])
                .unwrap(),
            102,
        )
        .unwrap();
    let closed_key = match result {
        NotificationResult::Accepted { action_nonces, .. } => action_nonces["open"].clone(),
        other => panic!("unexpected result: {other:?}"),
    };
    sink.close(2);
    assert_eq!(
        sink.invoke_action(&closed_key, "observer-a", 103),
        Err(d2b_provider_notification_desktop::ActionNonceError::Unavailable)
    );
}

#[test]
fn zero_pending_capacity_fails_closed() {
    let mut sink = NotificationSink::new(0, 2, 10);
    let mut port = RecordingSink::default();
    assert_eq!(
        sink.deliver(&mut port, "observer-a", request(), 100)
            .unwrap(),
        NotificationResult::CapacityExceeded
    );
}

#[test]
fn controller_has_no_provider_state_volume_and_tracks_display_dependency() {
    let controller = NotificationController::new("Provider/notification-desktop").unwrap();
    assert!(controller.provider_state_set_empty());
    assert!(controller.plan(false).is_err());
    assert!(controller.plan(true).is_ok());
}

#[test]
fn notification_descriptor_is_transient_and_stream_scoped() {
    let descriptor = NotificationProviderDescriptor::default();
    assert!(descriptor.validate().is_ok());
    assert_eq!(descriptor.streams().len(), 2);
    assert!(!descriptor.provider_state_volume);
}
