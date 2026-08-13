use d2b_provider_transport_azure_relay::{RelaySessionPhase, RelayTransportError};

#[test]
fn bootstrap_continuation_is_rejected_before_enrollment_commit() {
    assert_eq!(
        RelaySessionPhase::Bootstrap.establish_enrolled_kk(false, true),
        Err(RelayTransportError::InvalidSessionTransition)
    );
    assert_eq!(
        RelaySessionPhase::EnrollmentCommitted
            .establish_enrolled_kk(true, false)
            .unwrap(),
        RelaySessionPhase::EnrolledKk
    );
}
