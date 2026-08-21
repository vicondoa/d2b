//! Structured activation-helper JSON protocol.

use d2b_contracts_resource::v3::{
    ActivationMode,
    ArtifactId,
};
use serde::{Deserialize, Serialize};

/// Maximum helper request bytes.
pub const MAX_HELPER_REQUEST_BYTES: usize = 2048;

/// Bounded JSON request accepted by the activation helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationHelperRequest {
    /// Private artifact identifier resolved by the target integrity channel.
    pub system_artifact_id: String,
    /// Target generation ordinal. The helper refuses an unbound activation
    /// request even when the artifact identifier is otherwise valid.
    pub target_generation: u64,
    /// Closed activation mode.
    pub activation_mode: ActivationMode,
}

impl ActivationHelperRequest {
    /// Validate the request without resolving a path.
    pub fn validate(&self) -> Result<(), ActivationHelperProtocolError> {
        if self.target_generation == 0 {
            return Err(ActivationHelperProtocolError::GenerationInvalid);
        }
        ArtifactId::parse(self.system_artifact_id.as_str())
            .map(|_| ())
            .map_err(|_| ActivationHelperProtocolError::ArtifactIdInvalid)
    }
}

/// Bounded JSON response emitted by the helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationHelperResponse {
    /// Closed helper outcome.
    pub outcome: ActivationHelperOutcome,
}

/// Bounded read-only artifact validation request accepted by the activation
/// helper. The helper resolves and verifies the private catalog entry without
/// invoking an activation script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationArtifactValidationRequest {
    /// Private artifact identifier to resolve and verify.
    pub system_artifact_id: ArtifactId,
}

/// Bounded read-only artifact validation response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationArtifactValidationResponse {
    /// Whether the private catalog entry and its store contents verified.
    pub valid: bool,
}

/// Closed helper result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationHelperOutcome {
    /// Target generation applied.
    Succeeded,
    /// Existing generation recorded.
    Adopted,
    /// Request refused before mutation.
    #[serde(rename = "helper-refused")]
    Refused,
    /// Target effect failed while source remained intact.
    #[serde(rename = "helper-failed")]
    Failed,
}

/// Protocol refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationHelperProtocolError {
    /// Input exceeded the fixed envelope bound.
    TooLarge,
    /// Input was not valid strict JSON.
    InvalidJson,
    /// Artifact ID did not satisfy the closed grammar.
    ArtifactIdInvalid,
    /// The target generation ordinal was missing or zero.
    GenerationInvalid,
}

impl core::fmt::Display for ActivationHelperProtocolError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "activation-helper-request-too-large",
            Self::InvalidJson => "activation-helper-request-invalid",
            Self::ArtifactIdInvalid => "activation-helper-artifact-invalid",
            Self::GenerationInvalid => "activation-helper-generation-invalid",
        })
    }
}

impl std::error::Error for ActivationHelperProtocolError {}

/// Parse one bounded helper request.
pub fn parse_request(
    bytes: &[u8],
) -> Result<ActivationHelperRequest, ActivationHelperProtocolError> {
    if bytes.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(ActivationHelperProtocolError::TooLarge);
    }
    let request: ActivationHelperRequest =
        serde_json::from_slice(bytes).map_err(|_| ActivationHelperProtocolError::InvalidJson)?;
    request.validate()?;
    Ok(request)
}

/// Parse one bounded read-only artifact validation request.
pub fn parse_validation_request(
    bytes: &[u8],
) -> Result<ActivationArtifactValidationRequest, ActivationHelperProtocolError> {
    if bytes.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(ActivationHelperProtocolError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ActivationHelperProtocolError::InvalidJson)
}

/// Encode one bounded helper response.
pub fn encode_response(
    response: ActivationHelperResponse,
) -> Result<Vec<u8>, ActivationHelperProtocolError> {
    serde_json::to_vec(&response).map_err(|_| ActivationHelperProtocolError::InvalidJson)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_request_is_bounded_and_typed() {
        let request = parse_validation_request(br#"{"systemArtifactId":"candidate-system"}"#)
            .expect("validation request");
        assert_eq!(request.system_artifact_id.as_str(), "candidate-system");
        assert!(parse_validation_request(br#"{"systemArtifactId":"bad_id"}"#).is_err());
    }
}
