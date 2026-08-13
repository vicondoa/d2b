//! Structured activation-helper JSON protocol.

use d2b_contracts::v3::ActivationMode;
use serde::{Deserialize, Serialize};

/// Maximum helper request bytes.
pub const MAX_HELPER_REQUEST_BYTES: usize = 2048;

/// Bounded JSON request accepted by the activation helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationHelperRequest {
    /// Private artifact identifier resolved by the target integrity channel.
    pub system_artifact_id: String,
    /// Closed activation mode.
    pub activation_mode: ActivationMode,
}

impl ActivationHelperRequest {
    /// Validate the request without resolving a path.
    pub fn validate(&self) -> Result<(), ActivationHelperProtocolError> {
        if self.system_artifact_id.is_empty()
            || self.system_artifact_id.len() > 128
            || self.system_artifact_id.contains('/')
            || !self
                .system_artifact_id
                .bytes()
                .enumerate()
                .all(|(index, byte)| {
                    (index == 0 && byte.is_ascii_lowercase())
                        || (index > 0
                            && (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
                })
        {
            return Err(ActivationHelperProtocolError::ArtifactIdInvalid);
        }
        Ok(())
    }
}

/// Bounded JSON response emitted by the helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationHelperResponse {
    /// Closed helper outcome.
    pub outcome: ActivationHelperOutcome,
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
    Refused,
    /// Target effect failed while source remained intact.
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
}

impl core::fmt::Display for ActivationHelperProtocolError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "activation-helper-request-too-large",
            Self::InvalidJson => "activation-helper-request-invalid",
            Self::ArtifactIdInvalid => "activation-helper-artifact-invalid",
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

/// Encode one bounded helper response.
pub fn encode_response(
    response: ActivationHelperResponse,
) -> Result<Vec<u8>, ActivationHelperProtocolError> {
    serde_json::to_vec(&response).map_err(|_| ActivationHelperProtocolError::InvalidJson)
}
