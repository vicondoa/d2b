//! Structured activation-runner boundary.

use d2b_contracts::v3::{ActivationMode, ActivationOutcomeCode, ArtifactId, ResourceRef};
use serde::{Deserialize, Serialize};

/// Target-local request with no executable or store path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationRunnerRequest {
    /// Private-catalog artifact identifier.
    pub system_artifact_id: ArtifactId,
    /// Host or Guest target.
    pub execution_ref: ResourceRef,
    /// Requested activation mode.
    pub activation_mode: ActivationMode,
}

/// Typed helper boundary used by the runner.
pub trait ActivationHelper {
    /// Apply one request using the target-local integrity-bound helper.
    fn activate(
        &self,
        request: &ActivationRunnerRequest,
    ) -> Result<RunnerOutcomeCode, ActivationRunnerError>;
}

/// Structured helper outcome code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerOutcomeCode {
    /// Activation completed.
    Succeeded,
    /// Existing generation was adopted.
    Adopted,
    /// Helper refused the request.
    HelperRefused,
    /// Helper failed after preserving the source.
    HelperFailed,
}

impl RunnerOutcomeCode {
    /// Convert to the public activation outcome.
    pub const fn public(self) -> ActivationOutcomeCode {
        match self {
            Self::Succeeded => ActivationOutcomeCode::Succeeded,
            Self::Adopted => ActivationOutcomeCode::Adopted,
            Self::HelperRefused => ActivationOutcomeCode::HelperRefused,
            Self::HelperFailed => ActivationOutcomeCode::HelperFailed,
        }
    }
}

/// Runner failures that do not expose paths or helper text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationRunnerError {
    /// Request was malformed.
    InvalidRequest,
    /// Helper output was not a closed JSON outcome.
    InvalidHelperOutput,
    /// Target-local helper was unavailable.
    HelperUnavailable,
}

impl core::fmt::Display for ActivationRunnerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "activation-runner-request-invalid",
            Self::InvalidHelperOutput => "activation-runner-helper-output-invalid",
            Self::HelperUnavailable => "activation-runner-helper-unavailable",
        })
    }
}

impl std::error::Error for ActivationRunnerError {}

/// Bounded runner result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivationRunnerResult {
    /// Closed outcome.
    pub outcome: RunnerOutcomeCode,
    /// Failed effects preserve the source generation.
    pub source_generation_preserved: bool,
}

/// Target-local runner.
#[derive(Debug, Default, Clone, Copy)]
pub struct ActivationRunner;

impl ActivationRunner {
    /// Run one structured request through the typed helper.
    pub fn run<H: ActivationHelper>(
        &self,
        request: &ActivationRunnerRequest,
        helper: &H,
    ) -> Result<ActivationRunnerResult, ActivationRunnerError> {
        if !matches!(
            request.execution_ref.resource_type().as_str(),
            "Host" | "Guest"
        ) {
            return Err(ActivationRunnerError::InvalidRequest);
        }
        let outcome = helper.activate(request)?;
        Ok(ActivationRunnerResult {
            outcome,
            source_generation_preserved: !matches!(
                outcome,
                RunnerOutcomeCode::Succeeded | RunnerOutcomeCode::Adopted
            ),
        })
    }

    /// Parse the helper's bounded JSON response.
    pub fn parse_helper_output(bytes: &[u8]) -> Result<RunnerOutcomeCode, ActivationRunnerError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            outcome: RunnerOutcomeCode,
        }
        if bytes.len() > 512 || String::from_utf8_lossy(bytes).contains('/') {
            return Err(ActivationRunnerError::InvalidHelperOutput);
        }
        let wire: Wire = serde_json::from_slice(bytes)
            .map_err(|_| ActivationRunnerError::InvalidHelperOutput)?;
        Ok(wire.outcome)
    }
}
