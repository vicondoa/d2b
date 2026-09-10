//! Authenticated daemon projection for host-generation handoff.

use d2b_contracts_broker::host_generation::{HandoffCoordinator, HandoffError, HandoffState};
use d2b_contracts_resource::v3::{ActivationMode, ArtifactId, ResourceRef};

/// Authenticated caller role for activation requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGenerationCallerRole {
    /// Lifecycle group member.
    Lifecycle,
    /// Administrator.
    Admin,
    /// Ordinary user.
    User,
}

/// Caller-derived request passed to the broker adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedHostGenerationRequest {
    /// Authenticated target.
    pub target: ResourceRef,
    /// Private-catalog artifact identifier.
    pub artifact_id: ArtifactId,
    /// Closed activation mode.
    pub mode: ActivationMode,
}

/// Refuse caller/target mismatches before broker dispatch.
pub fn authorize_request(
    role: HostGenerationCallerRole,
    caller_target: &ResourceRef,
    request: &AuthenticatedHostGenerationRequest,
) -> Result<(), HandoffError> {
    if !matches!(
        role,
        HostGenerationCallerRole::Lifecycle | HostGenerationCallerRole::Admin
    ) {
        tracing::debug!(
            role = ?role,
            "host-generation handoff request rejected: caller lacks lifecycle authority",
        );
        return Err(HandoffError::InvalidTransition);
    }
    if caller_target != &request.target {
        tracing::debug!(
            target = %caller_target.to_canonical_string(),
            expected = %request.target.to_canonical_string(),
            "host-generation handoff request rejected: caller target mismatch",
        );
        return Err(HandoffError::TargetFingerprintMismatch);
    }
    Ok(())
}

/// Daemon-owned handoff handle. Durable records remain broker-owned.
#[derive(Debug)]
pub struct HostGenerationCoordinator {
    handoff: HandoffCoordinator,
}

impl HostGenerationCoordinator {
    /// Wrap the pure handoff state machine.
    pub const fn new(handoff: HandoffCoordinator) -> Self {
        Self { handoff }
    }

    /// Return the current durable phase.
    pub const fn state(&self) -> HandoffState {
        self.handoff.state()
    }

    /// Validate target closure evidence.
    pub fn validate_target(
        &mut self,
        generation: u64,
        fingerprint: [u8; 32],
    ) -> Result<(), HandoffError> {
        let result = self.handoff.validate_target(generation, fingerprint);
        if let Err(error) = &result {
            tracing::debug!(
                generation,
                %error,
                "host-generation target closure evidence rejected",
            );
        }
        result
    }

    /// Preserve the source after a refused or failed effect.
    pub fn rollback(&mut self) -> Result<(), HandoffError> {
        let result = self.handoff.rollback();
        if let Err(error) = &result {
            tracing::warn!(
                %error,
                "host-generation rollback refused; source preservation uncertain",
            );
        }
        result
    }

    /// Whether the source remains usable.
    pub const fn source_remains_usable(&self) -> bool {
        self.handoff.source_remains_usable()
    }
}
