use d2b_provider_runtime_azure_virtual_machine::{
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
    for error in [
        AzureVmError::ArmQuotaExceeded,
        AzureVmError::ArmResourceConflict,
        AzureVmError::ArmProvisioningFailed,
        AzureVmError::ArmNetworkUnavailable,
        AzureVmError::ArmCredentialDenied,
        AzureVmError::ArmThrottled,
        AzureVmError::BootstrapPskExpired,
        AzureVmError::BootstrapPskReplayed,
        AzureVmError::BootstrapEnrollmentFailed,
        AzureVmError::BootstrapFailed,
        AzureVmError::CredentialUnavailable,
        AzureVmError::InvalidOperationHandle,
        AzureVmError::InvalidConfiguration,
        AzureVmError::Transient,
        AzureVmError::Cancelled,
        AzureVmError::DeadlineExpired,
        AzureVmError::Ambiguous,
    ] {
        assert!(!error.code().is_empty());
    }
}
