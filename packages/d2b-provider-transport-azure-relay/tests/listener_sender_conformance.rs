use d2b_provider_transport_azure_relay::{
    RelayEnrollmentProof, RelayEnrollmentVerifier, RelaySessionPhase, RelayTransportError,
};

struct ValidEnrollment;

impl RelayEnrollmentVerifier for ValidEnrollment {
    fn verify_enrollment(&self, transcript: &[u8]) -> bool {
        transcript == b"enrollment"
    }
}

#[test]
fn bootstrap_continuation_is_rejected_before_enrollment_commit() {
    let proof = RelayEnrollmentProof::authenticate(&ValidEnrollment, b"enrollment").unwrap();
    assert_eq!(
        RelaySessionPhase::Bootstrap.establish_enrolled_kk(&proof, true),
        Err(RelayTransportError::InvalidSessionTransition)
    );
    assert_eq!(
        RelaySessionPhase::EnrollmentCommitted
            .establish_enrolled_kk(&proof, false)
            .unwrap(),
        RelaySessionPhase::EnrolledKk
    );
}

#[test]
fn invalid_enrollment_proof_is_rejected() {
    assert_eq!(
        RelayEnrollmentProof::authenticate(&ValidEnrollment, b"forged"),
        Err(RelayTransportError::AuthenticationFailed)
    );
}
