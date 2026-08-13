//! Device TPM child-resource lifecycle controller.

use d2b_contracts::v3::{ResourceRef, ResourceUid};
use serde::Serialize;

use crate::resource_effect::{TpmResourceEffectError, TpmResourceEffectPort};

/// Lifecycle phase of the resource-backed TPM controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TpmResourcePhase {
    /// No child resources have been admitted.
    Pending,
    /// Child resources are being created or adopted.
    Reconciling,
    /// The endpoint is ready for Guest consumers.
    Ready,
    /// A retryable effect failed.
    Degraded,
    /// The state or schema failed closed.
    Failed,
    /// The finalizer has completed.
    Finalized,
}

/// Stable result of a resource-backed TPM reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmResourceOutcome {
    /// The endpoint is ready.
    Ready,
    /// The Device must be retried.
    Retry,
    /// The state was refused without replacement.
    Failed,
    /// Finalization stopped workers and retained the Volume.
    VolumeRetained,
}

impl TpmResourceOutcome {
    /// Stable status code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Retry => "retry",
            Self::Failed => "failed",
            Self::VolumeRetained => "volume-retained",
        }
    }
}

/// Controller error with no path or broker detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmResourceControllerError {
    /// Core effect failed.
    Effect(TpmResourceEffectError),
    /// Finalization was requested before reconcile.
    InvalidState,
}

impl core::fmt::Display for TpmResourceControllerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Effect(error) => error.fmt(formatter),
            Self::InvalidState => formatter.write_str("device-tpm-resource-invalid-state"),
        }
    }
}

impl std::error::Error for TpmResourceControllerError {}

/// Resource-backed Device TPM controller.
pub struct TpmResourceController {
    device_uid: ResourceUid,
    execution_ref: ResourceRef,
    phase: TpmResourcePhase,
    finalizer: bool,
    volume_ref: Option<ResourceRef>,
    process_ref: Option<ResourceRef>,
    flush_ref: Option<ResourceRef>,
    endpoint_ref: Option<ResourceRef>,
}

impl TpmResourceController {
    /// Construct a controller for one emulated Device.
    pub fn new(
        device_uid: ResourceUid,
        execution_ref: ResourceRef,
    ) -> Result<Self, TpmResourceControllerError> {
        if execution_ref.resource_type().as_str() != "Host" {
            return Err(TpmResourceControllerError::Effect(
                TpmResourceEffectError::InvalidExecutionRef,
            ));
        }
        Ok(Self {
            device_uid,
            execution_ref,
            phase: TpmResourcePhase::Pending,
            finalizer: true,
            volume_ref: None,
            process_ref: None,
            flush_ref: None,
            endpoint_ref: None,
        })
    }

    /// Return the current lifecycle phase.
    pub const fn phase(&self) -> TpmResourcePhase {
        self.phase
    }

    /// Return whether the state-preserving finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Borrow the observed TPM Endpoint, when ready.
    pub const fn endpoint_ref(&self) -> Option<&ResourceRef> {
        self.endpoint_ref.as_ref()
    }

    /// Reconcile children in Volume -> Process -> flush -> Endpoint order.
    pub async fn reconcile<P: TpmResourceEffectPort>(
        &mut self,
        port: &P,
    ) -> Result<TpmResourceOutcome, TpmResourceControllerError> {
        if !self.finalizer || self.phase == TpmResourcePhase::Finalized {
            return Err(TpmResourceControllerError::InvalidState);
        }
        self.phase = TpmResourcePhase::Reconciling;
        let volume = match port
            .ensure_state_volume(&self.device_uid, &self.execution_ref)
            .await
        {
            Ok(value) => value,
            Err(error) => return self.effect_failed(error),
        };
        let process = match port
            .request_swtpm_process(&self.device_uid, &volume, &self.execution_ref)
            .await
        {
            Ok(value) => value,
            Err(error) => return self.effect_failed(error),
        };
        let flush = match port
            .request_flush_process(&self.device_uid, &process, &self.execution_ref)
            .await
        {
            Ok(value) => value,
            Err(error) => return self.effect_failed(error),
        };
        let endpoint = match port.watch_tpm_endpoint(&process).await {
            Ok(value) => value,
            Err(error) => return self.effect_failed(error),
        };
        self.volume_ref = Some(volume);
        self.process_ref = Some(process);
        self.flush_ref = Some(flush);
        self.endpoint_ref = Some(endpoint);
        self.phase = TpmResourcePhase::Ready;
        Ok(TpmResourceOutcome::Ready)
    }

    /// Stop children and retain the Device-owned state Volume.
    pub async fn finalize<P: TpmResourceEffectPort>(
        &mut self,
        port: &P,
    ) -> Result<TpmResourceOutcome, TpmResourceControllerError> {
        if !self.finalizer {
            return Ok(TpmResourceOutcome::VolumeRetained);
        }
        let process = self
            .process_ref
            .as_ref()
            .ok_or(TpmResourceControllerError::InvalidState)?;
        port.stop_swtpm_process(process)
            .await
            .map_err(TpmResourceControllerError::Effect)?;
        if let Some(flush) = self.flush_ref.as_ref() {
            port.delete_flush_process(flush)
                .await
                .map_err(TpmResourceControllerError::Effect)?;
        }
        self.finalizer = false;
        self.phase = TpmResourcePhase::Finalized;
        Ok(TpmResourceOutcome::VolumeRetained)
    }

    fn effect_failed<T>(
        &mut self,
        error: TpmResourceEffectError,
    ) -> Result<T, TpmResourceControllerError> {
        self.phase = if error == TpmResourceEffectError::Transient {
            TpmResourcePhase::Degraded
        } else {
            TpmResourcePhase::Failed
        };
        Err(TpmResourceControllerError::Effect(error))
    }
}
