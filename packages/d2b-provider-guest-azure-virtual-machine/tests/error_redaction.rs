use d2b_provider_guest_azure_virtual_machine::{
    AzureOperationHandle, AzureVmError, BootstrapPsk,
};

#[test]
fn errors_and_handles_do_not_render_remote_values() {
    let handle = AzureOperationHandle::from_core(b"opaque-operation").unwrap();
    assert!(!format!("{:?}", handle).contains("opaque-operation"));
    assert!(!format!("{:?}", BootstrapPsk::from_bytes(b"secret").unwrap()).contains("secret"));
    assert_eq!(
        AzureVmError::ArmCredentialDenied.code(),
        "arm-credential-denied"
    );
}

#[test]
fn every_controller_error_has_a_documented_stable_code() {
    assert_eq!(AzureVmError::ArmQuotaExceeded.code(), "arm-quota-exceeded");
    assert_eq!(AzureVmError::ArmResourceConflict.code(), "arm-resource-conflict");
    assert_eq!(AzureVmError::ArmProvisioningFailed.code(), "arm-provisioning-failed");
    assert_eq!(AzureVmError::ArmNetworkUnavailable.code(), "arm-network-unavailable");
    assert_eq!(AzureVmError::ArmCredentialDenied.code(), "arm-credential-denied");
    assert_eq!(AzureVmError::ArmThrottled.code(), "arm-throttled");
    assert_eq!(AzureVmError::BootstrapPskExpired.code(), "bootstrap-psk-expired");
    assert_eq!(AzureVmError::BootstrapPskReplayed.code(), "bootstrap-psk-replayed");
    assert_eq!(AzureVmError::BootstrapEnrollmentFailed.code(), "bootstrap-enrollment-failed");
    assert_eq!(AzureVmError::BootstrapFailed.code(), "bootstrap-failed");
    assert_eq!(AzureVmError::CredentialUnavailable.code(), "credential-unavailable");
    assert_eq!(AzureVmError::InvalidOperationHandle.code(), "azure-operation-handle-invalid");
    assert_eq!(AzureVmError::InvalidConfiguration.code(), "azure-vm-config-invalid");
    assert_eq!(AzureVmError::Transient.code(), "transient");
    assert_eq!(AzureVmError::Cancelled.code(), "cancelled");
    assert_eq!(AzureVmError::DeadlineExpired.code(), "deadline-expired");
    assert_eq!(AzureVmError::Ambiguous.code(), "azure-operation-ambiguous");
}
