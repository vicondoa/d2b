//! Neutral asynchronous effect port for the Device TPM controller.

use core::fmt;
use std::future::Future;

use d2b_contracts::v3::{ResourceRef, ResourceUid};

/// Closed effect failure for controller-created TPM resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmResourceEffectError {
    /// The Device UID was not canonical.
    InvalidDevice,
    /// The controller was not placed on a Host.
    InvalidExecutionRef,
    /// Core refused an opaque child-resource effect.
    EffectRejected,
    /// The effect may be retried without deleting retained state.
    Transient,
    /// The trusted marker or adoption proof failed closed.
    StateIntegrity,
}

impl TpmResourceEffectError {
    /// Return the stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidDevice => "device-tpm-device-invalid",
            Self::InvalidExecutionRef => "device-tpm-execution-ref-invalid",
            Self::EffectRejected => "device-tpm-effect-rejected",
            Self::Transient => "device-tpm-effect-transient",
            Self::StateIntegrity => "device-tpm-state-integrity-failure",
        }
    }
}

impl fmt::Display for TpmResourceEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for TpmResourceEffectError {}

/// Typed Core effect port for TPM child resources.
pub trait TpmResourceEffectPort: Send + Sync {
    /// Ensure or adopt the Device-owned persistent state Volume.
    fn ensure_state_volume(
        &self,
        device_uid: &ResourceUid,
        device_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> impl Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send;

    /// Ensure the long-lived swtpm Process.
    fn request_swtpm_process(
        &self,
        device_uid: &ResourceUid,
        volume_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> impl Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send;

    /// Always request the mandatory pre-start flush EphemeralProcess.
    fn request_flush_process(
        &self,
        device_uid: &ResourceUid,
        execution_ref: &ResourceRef,
    ) -> impl Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send;

    /// Stop the owned swtpm Process.
    fn stop_swtpm_process(
        &self,
        process_ref: &ResourceRef,
    ) -> impl Future<Output = Result<(), TpmResourceEffectError>> + Send;

    /// Delete the completed flush EphemeralProcess.
    fn delete_flush_process(
        &self,
        process_ref: &ResourceRef,
    ) -> impl Future<Output = Result<(), TpmResourceEffectError>> + Send;

    /// Observe the Device-owned TPM Endpoint.
    fn watch_tpm_endpoint(
        &self,
        process_ref: &ResourceRef,
    ) -> impl Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send;
}
