use d2b_provider_notification_desktop::{AdmissionPurpose, SessionEvidence, TransportClass};

#[test]
fn observer_requires_local_pre_authorized_transport() {
    let evidence = SessionEvidence::new(
        true,
        true,
        "d2b.notification.v3",
        AdmissionPurpose::DesktopObserver,
        TransportClass::EnrolledNoiseKk,
    );
    assert!(evidence.admit().is_err());
}
