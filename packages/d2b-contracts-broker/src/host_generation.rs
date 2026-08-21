//! Typed source-to-target host-generation handoff state.
//!
//! This module contains only the pure contract and replay-safe state machine.
//! The broker remains the only owner of durable records and host mutation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use d2b_contracts_resource::v3::{ActivationMode, ArtifactId, ResourceRef};

/// Canonical broker operation name for an activation handoff.
pub const APPLY_HOST_GENERATION_HANDOFF: &str = "ApplyHostGenerationHandoff";

/// Stable protocol identifier for the source-generation handoff.
pub const SOURCE_HANDOFF_PROTOCOL: &str = "source-handoff-v1";

/// Derive the broker-visible closure identity for one authenticated target.
///
/// The digest is deliberately derived from opaque contract identities rather
/// than a host path. Both sides of the handoff use this helper, so a target,
/// artifact, or generation substitution cannot pass validation.
pub fn target_fingerprint(
    target: &ResourceRef,
    artifact: &ArtifactId,
    generation: u64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(target.to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(artifact.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(generation.to_be_bytes());
    hasher.finalize().into()
}

/// Compatibility floor negotiated before a generation handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceGenerationCompatibilityFloorV1 {
    minimum_generation: u64,
    target_fingerprint: [u8; 32],
}

impl SourceGenerationCompatibilityFloorV1 {
    /// Construct a non-empty compatibility floor.
    pub fn new(
        minimum_generation: u64,
        target_fingerprint: [u8; 32],
    ) -> Result<Self, HandoffError> {
        if minimum_generation == 0 || target_fingerprint == [0; 32] {
            return Err(HandoffError::CompatibilityFloorInvalid);
        }
        Ok(Self {
            minimum_generation,
            target_fingerprint,
        })
    }

    /// Return the stable protocol identifier.
    pub const fn protocol(&self) -> &'static str {
        SOURCE_HANDOFF_PROTOCOL
    }

    /// Return the minimum source generation.
    pub const fn minimum_generation(&self) -> u64 {
        self.minimum_generation
    }

    /// Return the authenticated target closure fingerprint.
    pub const fn target_fingerprint(&self) -> [u8; 32] {
        self.target_fingerprint
    }

    /// Validate a target generation and closure fingerprint.
    pub fn validate_target(
        &self,
        generation: u64,
        fingerprint: [u8; 32],
    ) -> Result<(), HandoffError> {
        if generation < self.minimum_generation {
            return Err(HandoffError::GenerationTooOld);
        }
        if fingerprint != self.target_fingerprint {
            return Err(HandoffError::TargetFingerprintMismatch);
        }
        Ok(())
    }

    /// Begin a replay-safe handoff from source to target.
    pub fn begin_handoff(
        self,
        source_generation: u64,
        target_generation: u64,
    ) -> Result<HandoffCoordinator, HandoffError> {
        if source_generation == 0
            || target_generation <= source_generation
            || source_generation < self.minimum_generation
        {
            return Err(HandoffError::GenerationAncestryInvalid);
        }
        Ok(HandoffCoordinator {
            floor: self,
            source_generation,
            target_generation,
            state: HandoffState::Recorded,
            source_remains_usable: true,
        })
    }
}

/// Typed activation intent recorded by the source broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostGenerationHandoffIntent {
    /// Source generation ordinal.
    pub source_generation: u64,
    /// Target generation ordinal.
    pub target_generation: u64,
    /// Private-catalog artifact identifier.
    pub system_artifact_id: ArtifactId,
    /// Target activation mode.
    pub activation_mode: ActivationMode,
    /// Negotiated compatibility floor.
    pub compatibility: SourceGenerationCompatibilityFloorV1,
}

/// Caller-derived broker role for the handoff operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffCallerRole {
    /// Lifecycle-authorized operator.
    Lifecycle,
    /// Administrator.
    Admin,
}

/// Typed broker request. The broker derives its target from this authenticated
/// request and never accepts a path, command, or authority token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyHostGenerationHandoff {
    /// Caller-derived role.
    pub caller_role: HandoffCallerRole,
    /// Authenticated target resource.
    pub target: ResourceRef,
    /// Durable handoff intent.
    pub intent: HostGenerationHandoffIntent,
}

impl ApplyHostGenerationHandoff {
    /// Validate the closed operation boundary.
    pub fn validate(&self) -> Result<(), HandoffError> {
        if self.target.resource_type().as_str() != "Host"
            && self.target.resource_type().as_str() != "Guest"
        {
            return Err(HandoffError::InvalidTransition);
        }
        if self.intent.system_artifact_id.as_str().contains('/') {
            return Err(HandoffError::TargetFingerprintMismatch);
        }
        if !matches!(
            self.caller_role,
            HandoffCallerRole::Lifecycle | HandoffCallerRole::Admin
        ) {
            return Err(HandoffError::InvalidTransition);
        }
        Ok(())
    }
}

/// Durable handoff phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffState {
    /// Intent was durably recorded.
    Recorded,
    /// Target closure and ancestry passed validation.
    Validated,
    /// Target mutation is in progress.
    Mutating,
    /// Ownership transferred to the target broker.
    Transferred,
    /// Target completed and source may be retired.
    Completed,
    /// Source was retained after refusal or rollback.
    RolledBack,
    /// Request was refused before mutation.
    Refused,
}

/// Stable handoff refusal/failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    /// The floor or fingerprint was empty.
    CompatibilityFloorInvalid,
    /// The target generation predates the floor.
    GenerationTooOld,
    /// The closure fingerprint was not the authenticated target.
    TargetFingerprintMismatch,
    /// The observed generation was not the authenticated target generation.
    TargetGenerationMismatch,
    /// Source and target are not a strict generation transition.
    GenerationAncestryInvalid,
    /// The handoff was not in the required phase.
    InvalidTransition,
}

impl core::fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::CompatibilityFloorInvalid => "handoff-compatibility-floor-invalid",
            Self::GenerationTooOld => "handoff-generation-too-old",
            Self::TargetFingerprintMismatch => "handoff-target-fingerprint-mismatch",
            Self::TargetGenerationMismatch => "handoff-target-generation-mismatch",
            Self::GenerationAncestryInvalid => "handoff-generation-ancestry-invalid",
            Self::InvalidTransition => "handoff-invalid-transition",
        })
    }
}

impl std::error::Error for HandoffError {}

/// Pure replay-safe coordinator used by the broker adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffCoordinator {
    floor: SourceGenerationCompatibilityFloorV1,
    source_generation: u64,
    target_generation: u64,
    state: HandoffState,
    source_remains_usable: bool,
}

impl HandoffCoordinator {
    /// Return the current durable phase.
    pub const fn state(&self) -> HandoffState {
        self.state
    }

    /// Return the source generation.
    pub const fn source_generation(&self) -> u64 {
        self.source_generation
    }

    /// Return the target generation.
    pub const fn target_generation(&self) -> u64 {
        self.target_generation
    }

    /// Whether source remains usable after this phase.
    pub const fn source_remains_usable(&self) -> bool {
        self.source_remains_usable
    }

    /// Validate the authenticated target before mutation.
    pub fn validate_target(
        &mut self,
        generation: u64,
        fingerprint: [u8; 32],
    ) -> Result<(), HandoffError> {
        if self.state != HandoffState::Recorded {
            return Err(HandoffError::InvalidTransition);
        }
        if generation != self.target_generation {
            self.state = HandoffState::Refused;
            return Err(HandoffError::TargetGenerationMismatch);
        }
        if let Err(error) = self.floor.validate_target(generation, fingerprint) {
            self.state = HandoffState::Refused;
            return Err(error);
        }
        self.state = HandoffState::Validated;
        Ok(())
    }

    /// Enter the mutation phase.
    pub fn begin_mutation(&mut self) -> Result<(), HandoffError> {
        if self.state != HandoffState::Validated {
            return Err(HandoffError::InvalidTransition);
        }
        self.state = HandoffState::Mutating;
        Ok(())
    }

    /// Transfer the durable coordinator to the target broker.
    pub fn transfer(&mut self) -> Result<(), HandoffError> {
        if self.state != HandoffState::Mutating {
            return Err(HandoffError::InvalidTransition);
        }
        self.state = HandoffState::Transferred;
        Ok(())
    }

    /// Complete the target and retire the source.
    pub fn complete(&mut self) -> Result<(), HandoffError> {
        if self.state != HandoffState::Transferred {
            return Err(HandoffError::InvalidTransition);
        }
        self.state = HandoffState::Completed;
        self.source_remains_usable = false;
        Ok(())
    }

    /// Roll back or preserve the source after a failed effect.
    pub fn rollback(&mut self) -> Result<(), HandoffError> {
        if matches!(
            self.state,
            HandoffState::Completed | HandoffState::RolledBack
        ) {
            return Err(HandoffError::InvalidTransition);
        }
        self.state = HandoffState::RolledBack;
        self.source_remains_usable = true;
        Ok(())
    }
}
