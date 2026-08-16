use d2b_provider_runtime_azure_virtual_machine::{
    AzureVmError, BootstrapAdmission, BootstrapAdmissionState, BootstrapPsk, BootstrapService,
    BootstrapServiceState,
};

#[test]
fn bootstrap_is_single_use_and_expiry_is_fail_closed() {
    let mut admission = BootstrapAdmission::new(BootstrapPsk::from_bytes(b"psk").unwrap(), 10);
    let mut service = BootstrapService::default();
    service
        .complete_enrollment(&mut admission, b"psk", 1)
        .unwrap();
    assert_eq!(service.state(), BootstrapServiceState::Enrolled);
    assert_eq!(admission.state(), BootstrapAdmissionState::Consumed);
    assert!(matches!(
        admission.consume(b"psk", 2),
        Err(AzureVmError::BootstrapPskReplayed)
    ));
}

#[test]
fn bootstrap_expiry_does_not_initialize_state() {
    let mut admission = BootstrapAdmission::new(BootstrapPsk::from_bytes(b"psk").unwrap(), 10);
    assert!(matches!(
        admission.consume(b"psk", 10),
        Err(AzureVmError::BootstrapPskExpired)
    ));
    assert_eq!(admission.state(), BootstrapAdmissionState::Expired);
}

#[test]
fn incorrect_psk_attempt_consumes_admission() {
    let mut admission = BootstrapAdmission::new(BootstrapPsk::from_bytes(b"psk").unwrap(), 10);
    let mut service = BootstrapService::default();
    assert!(matches!(
        service.complete_enrollment(&mut admission, b"wrong", 1),
        Err(AzureVmError::BootstrapEnrollmentFailed)
    ));
    assert_eq!(service.state(), BootstrapServiceState::Failed);
    assert_eq!(admission.state(), BootstrapAdmissionState::Consumed);
    assert!(matches!(
        service.complete_enrollment(&mut admission, b"psk", 2),
        Err(AzureVmError::BootstrapPskReplayed)
    ));
}
