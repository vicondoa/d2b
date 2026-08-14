use d2b_provider_transport_azure_relay::{
    RelayEnrollmentProof, RelayEnrollmentVerifier, RelaySessionPhase, RelayTransportError,
};

struct ValidEnrollment;

impl RelayEnrollmentVerifier for ValidEnrollment {
    fn verify_enrollment(
        &self,
        transcript: &[u8],
        _: &d2b_provider_transport_azure_relay::RelayEnrollmentChallenge,
    ) -> bool {
        transcript == b"enrollment"
    }
}

#[test]
fn bootstrap_continuation_is_rejected_before_enrollment_commit() {
    let challenge =
        d2b_provider_transport_azure_relay::RelayEnrollmentChallenge::from_bytes([7; 32]);
    let proof =
        RelayEnrollmentProof::authenticate(&ValidEnrollment, b"enrollment", &challenge).unwrap();
    assert_eq!(
        RelaySessionPhase::Bootstrap.establish_enrolled_kk(proof, true),
        Err(RelayTransportError::InvalidSessionTransition)
    );
    let proof =
        RelayEnrollmentProof::authenticate(&ValidEnrollment, b"enrollment", &challenge).unwrap();
    assert_eq!(
        RelaySessionPhase::EnrollmentCommitted
            .establish_enrolled_kk(proof, false)
            .unwrap(),
        RelaySessionPhase::EnrolledKk
    );
}

#[test]
fn invalid_enrollment_proof_is_rejected() {
    assert_eq!(
        RelayEnrollmentProof::authenticate(
            &ValidEnrollment,
            b"forged",
            &d2b_provider_transport_azure_relay::RelayEnrollmentChallenge::from_bytes([7; 32]),
        ),
        Err(RelayTransportError::AuthenticationFailed)
    );
}

struct ChallengeBoundEnrollment {
    challenge: [u8; 32],
}

impl RelayEnrollmentVerifier for ChallengeBoundEnrollment {
    fn verify_enrollment(
        &self,
        transcript: &[u8],
        challenge: &d2b_provider_transport_azure_relay::RelayEnrollmentChallenge,
    ) -> bool {
        transcript == b"enrollment" && challenge.as_bytes() == &self.challenge
    }
}

#[test]
fn enrollment_transcript_is_bound_to_its_connection_challenge() {
    let first = d2b_provider_transport_azure_relay::RelayEnrollmentChallenge::from_bytes([1; 32]);
    let second =
        d2b_provider_transport_azure_relay::RelayEnrollmentChallenge::from_bytes([2; 32]);
    let verifier = ChallengeBoundEnrollment { challenge: [1; 32] };

    assert!(RelayEnrollmentProof::authenticate(&verifier, b"enrollment", &first).is_ok());
    assert_eq!(
        RelayEnrollmentProof::authenticate(&verifier, b"enrollment", &second),
        Err(RelayTransportError::AuthenticationFailed)
    );
}
