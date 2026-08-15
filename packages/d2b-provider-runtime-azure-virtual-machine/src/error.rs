//! Stable Azure VM Provider errors.

use std::fmt;

/// Closed Azure VM failure codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmError {
    /// Azure quota rejected the operation.
    ArmQuotaExceeded,
    /// A matching-name resource has foreign d2b ownership tags.
    ArmResourceConflict,
    /// Azure reported a provisioning failure.
    ArmProvisioningFailed,
    /// Network placement was unavailable.
    ArmNetworkUnavailable,
    /// ARM credentials were rejected.
    ArmCredentialDenied,
    /// ARM throttled the request.
    ArmThrottled,
    /// The bootstrap PSK expired before enrollment.
    BootstrapPskExpired,
    /// The bootstrap PSK was already consumed.
    BootstrapPskReplayed,
    /// The bootstrap handshake failed.
    BootstrapEnrollmentFailed,
    /// The bootstrap deadline elapsed.
    BootstrapFailed,
    /// Credential delivery was unavailable.
    CredentialUnavailable,
    /// An opaque effect handle was invalid.
    InvalidOperationHandle,
    /// The request was invalid.
    InvalidConfiguration,
    /// The operation is transient and safe to retry.
    Transient,
    /// The operation was cancelled.
    Cancelled,
    /// The operation deadline elapsed.
    DeadlineExpired,
    /// The effect outcome is ambiguous and must not be replayed.
    Ambiguous,
}

impl AzureVmError {
    /// Return the stable wire code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ArmQuotaExceeded => "arm-quota-exceeded",
            Self::ArmResourceConflict => "arm-resource-conflict",
            Self::ArmProvisioningFailed => "arm-provisioning-failed",
            Self::ArmNetworkUnavailable => "arm-network-unavailable",
            Self::ArmCredentialDenied => "arm-credential-denied",
            Self::ArmThrottled => "arm-throttled",
            Self::BootstrapPskExpired => "bootstrap-psk-expired",
            Self::BootstrapPskReplayed => "bootstrap-psk-replayed",
            Self::BootstrapEnrollmentFailed => "bootstrap-enrollment-failed",
            Self::BootstrapFailed => "bootstrap-failed",
            Self::CredentialUnavailable => "credential-unavailable",
            Self::InvalidOperationHandle => "azure-operation-handle-invalid",
            Self::InvalidConfiguration => "azure-vm-config-invalid",
            Self::Transient => "transient",
            Self::Cancelled => "cancelled",
            Self::DeadlineExpired => "deadline-expired",
            Self::Ambiguous => "azure-operation-ambiguous",
        }
    }

    /// Return whether retrying the same operation is safe.
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Transient | Self::ArmThrottled)
    }
}

impl fmt::Display for AzureVmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AzureVmError {}
