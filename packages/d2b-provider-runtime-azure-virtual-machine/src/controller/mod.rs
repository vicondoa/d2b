//! Azure VM lifecycle controller.

use std::{fmt, sync::Arc};

use sha2::{Digest, Sha256};

use crate::{
    bootstrap::BootstrapPsk,
    config::{AzureVmConfig, AzureVmGuestSettings},
    effect::{
        AzureEffectPort, AzureVmHandle, AzureVmState, LroStatus, PskExtensionPayload, TagDigest,
    },
    error::AzureVmError,
    idempotency,
};

/// Azure VM Provider phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmPhase {
    /// No correlated VM exists.
    Absent,
    /// VM provisioning is in progress.
    Provisioning,
    /// PSK extension delivery is in progress.
    PskDelivering,
    /// VM is awaiting the bootstrap session.
    Bootstrapping,
    /// VM and enrolled KK session are ready.
    Ready,
    /// VM is being reconfigured.
    Reconfiguring,
    /// VM is draining.
    Draining,
    /// VM deletion is in progress.
    Deleting,
    /// Provider failed closed.
    Failed,
    /// Finalizer completed.
    Finalized,
}

/// Non-blocking controller result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmReconcileOutcome {
    /// The VM is ready.
    Converged,
    /// Poll again after a bounded delay.
    Progressing {
        /// Delay in milliseconds.
        after_ms: u32,
    },
    /// Retry the same operation.
    Retry {
        /// Delay in milliseconds.
        after_ms: u32,
    },
}

/// Redacted Guest status projection.
#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmStatus {
    phase: AzureVmPhase,
    identity_digest: Option<[u8; 32]>,
    operation_digest: Option<[u8; 32]>,
}

impl AzureVmStatus {
    /// Return the current phase.
    pub const fn phase(&self) -> AzureVmPhase {
        self.phase
    }

    /// Return the enrolled identity digest.
    pub const fn identity_digest(&self) -> Option<[u8; 32]> {
        self.identity_digest
    }

    /// Return the opaque operation digest.
    pub const fn operation_digest(&self) -> Option<[u8; 32]> {
        self.operation_digest
    }
}

impl fmt::Debug for AzureVmStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmStatus")
            .field("phase", &self.phase)
            .field(
                "identity_digest",
                &self.identity_digest.map(|_| "<redacted>"),
            )
            .field(
                "operation_digest",
                &self.operation_digest.map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Azure VM controller.
pub struct AzureVmController<E> {
    provider_config: AzureVmConfig,
    settings: AzureVmGuestSettings,
    effect: Arc<E>,
    phase: AzureVmPhase,
    finalizer: bool,
    operation: Option<crate::effect::AzureOperationHandle>,
    vm_handle: Option<AzureVmHandle>,
    tag_digest: Option<TagDigest>,
    identity_digest: Option<[u8; 32]>,
    bootstrap_psk: Option<BootstrapPsk>,
}

impl<E> AzureVmController<E>
where
    E: AzureEffectPort + 'static,
{
    /// Construct a controller after validating the two config layers.
    pub fn new(
        provider_config: AzureVmConfig,
        settings: AzureVmGuestSettings,
        effect: Arc<E>,
        bootstrap_psk: Option<BootstrapPsk>,
    ) -> Result<Self, AzureVmError> {
        provider_config.validate()?;
        settings.validate()?;
        Ok(Self {
            provider_config,
            settings,
            effect,
            phase: AzureVmPhase::Absent,
            finalizer: true,
            operation: None,
            vm_handle: None,
            tag_digest: None,
            identity_digest: None,
            bootstrap_psk,
        })
    }

    /// Return the current phase.
    pub const fn phase(&self) -> AzureVmPhase {
        self.phase
    }

    /// Return whether the finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Return the redacted status.
    pub fn status(&self) -> AzureVmStatus {
        AzureVmStatus {
            phase: self.phase,
            identity_digest: self.identity_digest,
            operation_digest: self.operation.as_ref().map(|operation| operation.digest()),
        }
    }

    /// Reconcile without blocking on ARM polling.
    pub async fn reconcile(
        &mut self,
        zone_uid: &str,
        guest_uid: &str,
        generation: u64,
    ) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if !self.finalizer {
            return Err(AzureVmError::InvalidConfiguration);
        }
        if let Some(operation) = self.operation.clone() {
            return self.poll_operation(operation).await;
        }
        let (state, handle, tags) = self.effect.get_vm_state(&self.settings).await?;
        match state {
            AzureVmState::Absent => {
                let operation_id =
                    idempotency::operation_id(zone_uid, guest_uid, generation, "provision");
                self.operation = Some(
                    self.effect
                        .start_vm_provision(&self.settings, &operation_id)
                        .await?,
                );
                self.phase = AzureVmPhase::Provisioning;
                Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
            }
            AzureVmState::Running => {
                let Some(handle) = handle else {
                    return Err(AzureVmError::Ambiguous);
                };
                let Some(tags) = tags else {
                    self.phase = AzureVmPhase::Failed;
                    return Err(AzureVmError::ArmResourceConflict);
                };
                self.vm_handle = Some(handle);
                self.tag_digest = Some(tags);
                self.phase = AzureVmPhase::Ready;
                self.identity_digest = Some(Sha256::digest(tags.as_bytes()).into());
                Ok(AzureVmReconcileOutcome::Converged)
            }
            AzureVmState::Provisioning => {
                self.phase = AzureVmPhase::Provisioning;
                Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
            }
            AzureVmState::Stopped => {
                self.phase = AzureVmPhase::Draining;
                Ok(AzureVmReconcileOutcome::Retry { after_ms: 1_000 })
            }
            AzureVmState::Failed | AzureVmState::Unknown => {
                self.phase = AzureVmPhase::Failed;
                Err(AzureVmError::ArmProvisioningFailed)
            }
        }
    }

    /// Adopt a running VM only when its d2b tag digest matches.
    pub async fn adopt(&mut self) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        let (state, handle, tags) = self.effect.get_vm_state(&self.settings).await?;
        if state != AzureVmState::Running {
            return Err(AzureVmError::Transient);
        }
        let Some(handle) = handle else {
            return Err(AzureVmError::Ambiguous);
        };
        let Some(tags) = tags else {
            self.phase = AzureVmPhase::Failed;
            return Err(AzureVmError::ArmResourceConflict);
        };
        self.vm_handle = Some(handle);
        self.tag_digest = Some(tags);
        self.identity_digest = Some(Sha256::digest(tags.as_bytes()).into());
        self.phase = AzureVmPhase::Ready;
        Ok(AzureVmReconcileOutcome::Converged)
    }

    /// Advance the current opaque long-running operation.
    pub async fn poll_operation(
        &mut self,
        operation: crate::effect::AzureOperationHandle,
    ) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        match self.effect.poll_lro(&operation).await? {
            LroStatus::InProgress { after_ms } => Ok(AzureVmReconcileOutcome::Progressing {
                after_ms: after_ms.max(1),
            }),
            LroStatus::Failed => {
                self.operation = None;
                self.phase = AzureVmPhase::Failed;
                Err(AzureVmError::ArmProvisioningFailed)
            }
            LroStatus::Succeeded => {
                self.operation = None;
                match self.phase {
                    AzureVmPhase::Provisioning => {
                        let handle = if let Some(handle) = self.vm_handle.clone() {
                            handle
                        } else {
                            let (_, handle, _) = self.effect.get_vm_state(&self.settings).await?;
                            let Some(handle) = handle else {
                                self.phase = AzureVmPhase::Failed;
                                return Err(AzureVmError::Ambiguous);
                            };
                            self.vm_handle = Some(handle.clone());
                            handle
                        };
                        if let Some(psk) = self.bootstrap_psk.take() {
                            let payload = PskExtensionPayload::from_secret(psk.consume().to_vec())?;
                            let _payload_len = payload.len();
                            self.operation =
                                Some(self.effect.put_vm_extension(&handle, payload).await?);
                            self.phase = AzureVmPhase::PskDelivering;
                            Ok(AzureVmReconcileOutcome::Progressing { after_ms: 250 })
                        } else {
                            self.phase = AzureVmPhase::Bootstrapping;
                            Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
                        }
                    }
                    AzureVmPhase::PskDelivering => {
                        self.phase = AzureVmPhase::Bootstrapping;
                        Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
                    }
                    AzureVmPhase::Deleting => {
                        self.finalizer = false;
                        self.phase = AzureVmPhase::Finalized;
                        Ok(AzureVmReconcileOutcome::Converged)
                    }
                    _ => Ok(AzureVmReconcileOutcome::Converged),
                }
            }
        }
    }

    /// Begin deletion. The finalizer is retained until the LRO succeeds.
    pub async fn finalize(
        &mut self,
        zone_uid: &str,
        guest_uid: &str,
        generation: u64,
    ) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if !self.finalizer {
            return Ok(AzureVmReconcileOutcome::Converged);
        }
        let Some(handle) = self.vm_handle.clone() else {
            self.finalizer = false;
            self.phase = AzureVmPhase::Finalized;
            return Ok(AzureVmReconcileOutcome::Converged);
        };
        if self.operation.is_none() {
            let operation_id = idempotency::operation_id(zone_uid, guest_uid, generation, "delete");
            self.operation = Some(self.effect.start_vm_delete(&handle, &operation_id).await?);
        }
        self.phase = AzureVmPhase::Deleting;
        Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
    }

    /// Return the configured gateway execution reference.
    pub fn controller_execution_ref(&self) -> &d2b_contracts::v3::ResourceRef {
        &self.provider_config.controller_execution_ref
    }
}
