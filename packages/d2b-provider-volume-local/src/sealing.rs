//! Provider-side sealing coordination without key custody.
//!
//! The Provider observes only Credential readiness and generation. A trusted
//! adapter resolves the admitted sealing-policy binding and performs wrapping.
//! No request or state here can select key authority or carry key bytes.

use std::fmt;

use d2b_contracts::v3::credential::CredentialLeaseState;
use d2b_contracts::v3::{SealingStatus, StateEnvelope, VolumeStateError};
use serde::Serialize;

use crate::audit::VolumeAuditKind;

/// Current provider-side sealing rotation phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationPhase {
    /// The admitted generation is active and no rotation is pending.
    Sealed,
    /// A `rotation-pending` status mutation must commit before an effect.
    StatusCommitRequired,
    /// The status-first commit completed and the bound adapter effect may run.
    EffectReady,
    /// Data committed but the durable broker audit still needs recovery.
    CommitPendingAudit,
    /// A terminal failure preserved the prior generation.
    Failed,
}

impl RotationPhase {
    /// Project this phase into the Volume state status contract.
    pub const fn sealing_status(self) -> SealingStatus {
        match self {
            Self::Sealed => SealingStatus::Sealed,
            Self::StatusCommitRequired | Self::EffectReady | Self::CommitPendingAudit => {
                SealingStatus::RotationPending
            }
            Self::Failed => SealingStatus::RotationFailed,
        }
    }
}

/// Closed action emitted by the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealingAction {
    /// Commit `rotation-pending` with optimistic revision control.
    CommitPendingStatus,
    /// Invoke the closed adapter effect using its admitted policy binding.
    InvokeBoundRotation,
    /// Retry the identical bound request without selecting a new authority.
    RetryIdenticalRotation,
    /// Commit sealed status after the adapter and durable audit complete.
    CommitSealedStatus,
    /// Commit terminal `rotation-failed` status while preserving old data.
    CommitFailedStatus,
    /// No action is required.
    None,
}

/// Closed result classes returned by the trusted sealing adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundRotationResult {
    /// This call committed a fresh rotation and its audit.
    Rotated,
    /// An identical request was already fully committed.
    AlreadyCommitted,
    /// Restart recovery completed a prior committed request.
    RecoveredCommitted,
    /// The identical request may be retried after bounded backoff.
    Retryable,
    /// Data committed and only durable success audit remains.
    CommitPendingAudit,
    /// A non-retryable policy, integrity, conflict, or revocation failure.
    TerminalFailure,
}

/// One sealing transition and its existing lifecycle audit kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealingTransition {
    /// New rotation phase.
    pub phase: RotationPhase,
    /// Next controller operation.
    pub action: SealingAction,
    /// Existing path-free lifecycle audit event kind.
    pub audit: Option<VolumeAuditKind>,
}

/// Closed, key-free sealing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealingError {
    /// Credential status is unavailable, expired, or revoked.
    CredentialUnavailable,
    /// A generation was zero, stale, or not strictly forward.
    GenerationInvalid,
    /// An event was applied in the wrong protocol phase.
    InvalidTransition,
    /// Provider payload digesting is unavailable, so sealing must not proceed.
    DigestDomainUnavailable,
    /// The state envelope failed a closed contract check.
    StateEnvelopeInvalid,
}

impl SealingError {
    /// Return the stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::CredentialUnavailable => "volume-sealing-credential-unavailable",
            Self::GenerationInvalid => "volume-sealing-generation-invalid",
            Self::InvalidTransition => "volume-sealing-transition-invalid",
            Self::DigestDomainUnavailable => "volume-state-digest-domain-unavailable",
            Self::StateEnvelopeInvalid => "volume-sealing-envelope-invalid",
        }
    }
}

impl fmt::Display for SealingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SealingError {}

/// Provider-side rotation state containing generation numbers only.
pub struct SealingState {
    active_generation: u64,
    target_generation: Option<u64>,
    phase: RotationPhase,
}

impl SealingState {
    /// Construct observed sealed state after the adapter reports its active generation.
    pub fn sealed(active_generation: u64) -> Result<Self, SealingError> {
        if active_generation == 0 {
            return Err(SealingError::GenerationInvalid);
        }
        Ok(Self {
            active_generation,
            target_generation: None,
            phase: RotationPhase::Sealed,
        })
    }

    /// Return the active authenticated generation.
    pub const fn active_generation(&self) -> u64 {
        self.active_generation
    }

    /// Return the pending target generation, if one is committed in status.
    pub const fn target_generation(&self) -> Option<u64> {
        self.target_generation
    }

    /// Return the current rotation phase.
    pub const fn phase(&self) -> RotationPhase {
        self.phase
    }

    /// Observe Credential status and request a status-first forward rotation.
    pub fn observe_credential(
        &mut self,
        lease_state: CredentialLeaseState,
        observed_generation: u64,
    ) -> Result<SealingTransition, SealingError> {
        if lease_state == CredentialLeaseState::Revoked {
            self.phase = RotationPhase::Failed;
            return Ok(SealingTransition {
                phase: self.phase,
                action: SealingAction::CommitFailedStatus,
                audit: Some(VolumeAuditKind::VolumeSealingRotationFailed),
            });
        }
        if lease_state != CredentialLeaseState::Active {
            return Err(SealingError::CredentialUnavailable);
        }
        if observed_generation == self.active_generation {
            return Ok(SealingTransition {
                phase: self.phase,
                action: SealingAction::None,
                audit: None,
            });
        }
        if observed_generation < self.active_generation || observed_generation == 0 {
            return Err(SealingError::GenerationInvalid);
        }
        if self.phase != RotationPhase::Sealed {
            return Err(SealingError::InvalidTransition);
        }
        self.target_generation = Some(observed_generation);
        self.phase = RotationPhase::StatusCommitRequired;
        Ok(SealingTransition {
            phase: self.phase,
            action: SealingAction::CommitPendingStatus,
            audit: Some(VolumeAuditKind::VolumeSealingRotationStart),
        })
    }

    /// Confirm the optimistic status mutation before releasing any effect.
    pub fn pending_status_committed(&mut self) -> Result<SealingTransition, SealingError> {
        if self.phase != RotationPhase::StatusCommitRequired || self.target_generation.is_none() {
            return Err(SealingError::InvalidTransition);
        }
        self.phase = RotationPhase::EffectReady;
        Ok(SealingTransition {
            phase: self.phase,
            action: SealingAction::InvokeBoundRotation,
            audit: None,
        })
    }

    /// Apply one closed result from the trusted policy-bound adapter.
    pub fn apply_bound_result(
        &mut self,
        result: BoundRotationResult,
    ) -> Result<SealingTransition, SealingError> {
        if !matches!(
            self.phase,
            RotationPhase::EffectReady | RotationPhase::CommitPendingAudit
        ) {
            return Err(SealingError::InvalidTransition);
        }
        match result {
            BoundRotationResult::Rotated
            | BoundRotationResult::AlreadyCommitted
            | BoundRotationResult::RecoveredCommitted => {
                let target = self
                    .target_generation
                    .take()
                    .ok_or(SealingError::InvalidTransition)?;
                self.active_generation = target;
                self.phase = RotationPhase::Sealed;
                Ok(SealingTransition {
                    phase: self.phase,
                    action: SealingAction::CommitSealedStatus,
                    audit: Some(VolumeAuditKind::VolumeSealingRotationCommitted),
                })
            }
            BoundRotationResult::Retryable => Ok(SealingTransition {
                phase: self.phase,
                action: SealingAction::RetryIdenticalRotation,
                audit: None,
            }),
            BoundRotationResult::CommitPendingAudit => {
                self.phase = RotationPhase::CommitPendingAudit;
                Ok(SealingTransition {
                    phase: self.phase,
                    action: SealingAction::RetryIdenticalRotation,
                    audit: None,
                })
            }
            BoundRotationResult::TerminalFailure => {
                self.phase = RotationPhase::Failed;
                Ok(SealingTransition {
                    phase: self.phase,
                    action: SealingAction::CommitFailedStatus,
                    audit: Some(VolumeAuditKind::VolumeSealingRotationFailed),
                })
            }
        }
    }
}

impl fmt::Debug for SealingState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealingState")
            .field("phase", &self.phase)
            .field("has_target", &self.target_generation.is_some())
            .finish_non_exhaustive()
    }
}

/// Validate a state envelope before asking the trusted adapter to wrap it.
///
/// The shared contract currently has no frozen Provider-state digest domain,
/// so this function fails closed before any adapter effect can be requested.
pub fn validate_envelope_for_sealing<T: Serialize>(
    envelope: &StateEnvelope<T>,
) -> Result<(), SealingError> {
    envelope.validate_digest().map_err(|error| match error {
        VolumeStateError::DigestDomainUnavailable => SealingError::DigestDomainUnavailable,
        _ => SealingError::StateEnvelopeInvalid,
    })
}
