//! Contracts for the activation-nixos Provider.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ArtifactId, ConfigurationGeneration, ObservedGeneration, ResourceRef, ResourceTypeName,
    execution_policy::require_execution_ref,
};
use d2b_contracts::wire_deserialize;

/// The canonical activation generation ResourceType.
pub const NIXOS_GENERATION_RESOURCE_TYPE: &str = "activation-nixos.d2bus.org.NixosGeneration";
/// The only Provider admitted by the activation generation schema.
pub const ACTIVATION_PROVIDER_REF: &str = "Provider/activation-nixos";
/// Maximum serialized stdin envelope delivered to the activation helper.
pub const MAX_ACTIVATION_RUNNER_INPUT_BYTES: usize = 2048;

/// Target-local activation mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationMode {
    /// Activate immediately for the running target.
    Switch,
    /// Select the generation for the next boot.
    Boot,
    /// Activate temporarily until the next boot.
    Test,
    /// Record an already active generation without running a helper.
    Adopt,
}

/// Closed input delivered to an activation-runner `EphemeralProcess`.
///
/// The broker serializes this value into the bounded stdin envelope consumed
/// by `d2b-activation-helper`. It deliberately carries no executable,
/// store path, target path, or caller-controlled bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationRunnerInput {
    /// Private-catalog artifact identifier.
    pub system_artifact_id: ArtifactId,
    /// Target generation ordinal bound to the owning `NixosGeneration`.
    pub target_generation: ConfigurationGeneration,
    /// Closed activation mode.
    pub activation_mode: ActivationMode,
}

impl ActivationRunnerInput {
    /// Construct one runner input.
    pub fn new(
        system_artifact_id: ArtifactId,
        target_generation: ConfigurationGeneration,
        activation_mode: ActivationMode,
    ) -> Self {
        Self {
            system_artifact_id,
            target_generation,
            activation_mode,
        }
    }
}

/// Provider-specific progress detail.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationDetail {
    /// Controller is preparing the runner.
    Planning,
    /// Runner resource has been staged.
    Staged,
    /// Helper is applying the target generation.
    Applying,
    /// Generation was applied.
    Applied,
    /// Generation is the next boot default.
    BootDefault,
    /// Existing active generation was recorded.
    Adopted,
    /// Rollback activation completed.
    RolledBack,
    /// A newer generation superseded this one.
    Superseded,
}

/// Stable terminal activation outcome codes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationOutcomeCode {
    /// Activation completed.
    Succeeded,
    /// Existing generation was adopted.
    Adopted,
    /// Caller or target was not authorized.
    Unauthorized,
    /// Source or target generation was stale.
    StaleGeneration,
    /// Target closure did not match the authenticated intent.
    TargetMismatch,
    /// Structured helper refused the request.
    HelperRefused,
    /// Helper returned a bounded failure.
    HelperFailed,
    /// Operation was rolled back while preserving the source.
    RolledBack,
}

impl ActivationOutcomeCode {
    /// Whether this is a successful terminal outcome.
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Succeeded | Self::Adopted)
    }
}

/// Validation failures for the public generation spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NixosGenerationSpecError {
    /// Provider reference was not the activation Provider.
    ProviderRefMismatch,
    /// Execution target was not a Host or Guest.
    ExecutionRefInvalid,
    /// Prior generation reference had the wrong ResourceType.
    PriorGenerationRefInvalid,
    /// Artifact identifier was invalid or empty.
    ArtifactIdInvalid,
    /// Target generation ordinal was zero.
    GenerationInvalid,
}

impl core::fmt::Display for NixosGenerationSpecError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::ProviderRefMismatch => "activation-provider-ref-mismatch",
            Self::ExecutionRefInvalid => "activation-execution-ref-invalid",
            Self::PriorGenerationRefInvalid => "activation-prior-generation-ref-invalid",
            Self::ArtifactIdInvalid => "activation-artifact-id-invalid",
            Self::GenerationInvalid => "activation-generation-invalid",
        })
    }
}

impl std::error::Error for NixosGenerationSpecError {}

/// Immutable public desired state of one NixOS generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NixosGenerationSpec {
    provider_ref: ResourceRef,
    execution_ref: ResourceRef,
    system_artifact_id: ArtifactId,
    activation_mode: ActivationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    prior_generation_ref: Option<ResourceRef>,
}

impl NixosGenerationSpec {
    /// Construct and validate a generation spec.
    pub fn new(
        provider_ref: ResourceRef,
        execution_ref: ResourceRef,
        system_artifact_id: impl Into<String>,
        activation_mode: ActivationMode,
        prior_generation_ref: Option<ResourceRef>,
    ) -> Result<Self, NixosGenerationSpecError> {
        if provider_ref.to_canonical_string() != ACTIVATION_PROVIDER_REF {
            return Err(NixosGenerationSpecError::ProviderRefMismatch);
        }
        require_execution_ref(&execution_ref)
            .map_err(|_| NixosGenerationSpecError::ExecutionRefInvalid)?;
        if let Some(reference) = &prior_generation_ref
            && reference.resource_type().as_str() != NIXOS_GENERATION_RESOURCE_TYPE
        {
            return Err(NixosGenerationSpecError::PriorGenerationRefInvalid);
        }
        let system_artifact_id = ArtifactId::parse(system_artifact_id.into())
            .map_err(|_| NixosGenerationSpecError::ArtifactIdInvalid)?;
        Ok(Self {
            provider_ref,
            execution_ref,
            system_artifact_id,
            activation_mode,
            prior_generation_ref,
        })
    }

    /// Borrow the fixed Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the Host or Guest execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the private-catalog artifact identifier.
    pub const fn system_artifact_id(&self) -> &ArtifactId {
        &self.system_artifact_id
    }

    /// Return the selected activation mode.
    pub const fn activation_mode(&self) -> ActivationMode {
        self.activation_mode
    }

    /// Borrow the optional superseded generation reference.
    pub const fn prior_generation_ref(&self) -> Option<&ResourceRef> {
        self.prior_generation_ref.as_ref()
    }

    /// Return the qualified ResourceType name.
    pub fn resource_type() -> ResourceTypeName {
        ResourceTypeName::parse(NIXOS_GENERATION_RESOURCE_TYPE)
            .expect("the frozen ResourceType name is valid")
    }
}

wire_deserialize!(
    NixosGenerationSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        provider_ref: ResourceRef,
        execution_ref: ResourceRef,
        system_artifact_id: String,
        activation_mode: ActivationMode,
        #[serde(default)]
        prior_generation_ref: Option<ResourceRef>,
    },
    wire,
    Self::new(
        wire.provider_ref,
        wire.execution_ref,
        wire.system_artifact_id,
        wire.activation_mode,
        wire.prior_generation_ref,
    )
    .map_err(serde::de::Error::custom)
);

/// Typed activation status below the universal ResourceStatus layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NixosGenerationStatus {
    /// Universal lifecycle phase.
    pub phase: super::ResourcePhase,
    /// Provider-specific progress detail.
    pub activation_detail: ActivationDetail,
    /// Bounded terminal result, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ActivationOutcomeCode>,
    /// Store generation revision observed by the controller.
    pub observed_generation: ObservedGeneration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_input_refuses_a_zero_target_generation() {
        let input = |target_generation: u64| {
            serde_json::json!({
                "systemArtifactId": "system-artifact",
                "targetGeneration": target_generation,
                "activationMode": "switch",
            })
        };
        assert!(
            serde_json::from_value::<ActivationRunnerInput>(input(1)).is_ok(),
            "a nonzero target generation decodes"
        );
        assert!(
            serde_json::from_value::<ActivationRunnerInput>(input(0)).is_err(),
            "a zero target generation is refused at decode"
        );
    }
}
