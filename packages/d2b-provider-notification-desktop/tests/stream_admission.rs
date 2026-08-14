use d2b_provider_notification_desktop::{
    AdmissionError, AdmissionPurpose, OBSERVER_STREAM, SINK_STREAM, TransportClass,
};

#[test]
fn notification_stream_admission_contract_is_closed() {
    assert_eq!(OBSERVER_STREAM, "DesktopNotificationObserver");
    assert_eq!(SINK_STREAM, "DesktopNotificationSink");
    assert_eq!(
        AdmissionError::TransportMismatch.to_string(),
        "session-untrusted-transport"
    );
    assert_ne!(
        AdmissionPurpose::NotificationSource,
        AdmissionPurpose::DesktopObserver
    );
    assert_ne!(
        TransportClass::EnrolledNoiseKk,
        TransportClass::UnixSeqpacket
    );
}
