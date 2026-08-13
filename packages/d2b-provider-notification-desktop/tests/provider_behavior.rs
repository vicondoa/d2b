use d2b_provider_notification_desktop::{
    ActionNonceStore, AdmissionPurpose, Category, DesktopNotificationPort, NotificationController,
    NotificationProviderDescriptor, NotificationRequest, NotificationResult, NotificationSink,
    NotificationUrgency, SessionEvidence, TransportClass,
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
    let result = sink.deliver(&mut port, "source-a", request(), 100).unwrap();
    assert!(matches!(result, NotificationResult::Accepted { .. }));
    assert_eq!(port.accepted, vec!["hello world"]);
    assert!(!format!("{sink:?}").contains("hello"));
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
