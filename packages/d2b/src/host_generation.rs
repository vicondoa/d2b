//! CLI-side typed host-generation request projection.

use d2b_contracts_zone_session::v3::{ActivationMode, ArtifactId, ResourceRef};

/// Request constructed after CLI parsing and before daemon authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGenerationRequest {
    /// Host or Guest execution target.
    pub target: ResourceRef,
    /// Private artifact identifier.
    pub artifact_id: ArtifactId,
    /// Closed activation mode.
    pub mode: ActivationMode,
}

/// Build one bounded request. Caller identity is deliberately absent.
pub fn build_request(
    target: ResourceRef,
    artifact_id: impl Into<String>,
    mode: ActivationMode,
) -> Result<HostGenerationRequest, HostGenerationRequestError> {
    if !matches!(target.resource_type().as_str(), "Host" | "Guest") {
        return Err(HostGenerationRequestError::TargetInvalid);
    }
    let artifact_id = ArtifactId::parse(artifact_id.into())
        .map_err(|_| HostGenerationRequestError::ArtifactInvalid)?;
    Ok(HostGenerationRequest {
        target,
        artifact_id,
        mode,
    })
}

/// CLI request validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGenerationRequestError {
    /// Target was not a Host or Guest reference.
    TargetInvalid,
    /// Artifact ID was malformed.
    ArtifactInvalid,
}

impl core::fmt::Display for HostGenerationRequestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::TargetInvalid => "host-generation-target-invalid",
            Self::ArtifactInvalid => "host-generation-artifact-invalid",
        })
    }
}

impl std::error::Error for HostGenerationRequestError {}
