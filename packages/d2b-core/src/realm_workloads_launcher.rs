use crate::workload_identity::WorkloadIdentity;
use d2b_realm_core::{
    CapabilitySet, LauncherIcon, LauncherItemSummary, ProtocolToken, WorkloadExecutionPosture,
    WorkloadProviderKind,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const REALM_WORKLOADS_LAUNCHER_V2_SCHEMA_VERSION: &str = "v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmWorkloadsLauncherV2Json {
    pub schema_version: String,
    pub runtime_state: LauncherMetadataRuntimeState,
    pub workloads: Vec<LauncherWorkloadSummary>,
    pub invariants: LauncherMetadataInvariants,
}

impl RealmWorkloadsLauncherV2Json {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != REALM_WORKLOADS_LAUNCHER_V2_SCHEMA_VERSION {
            return Err(format!(
                "realm-workloads-launcher-v2 schemaVersion must be {REALM_WORKLOADS_LAUNCHER_V2_SCHEMA_VERSION}"
            ));
        }
        let invariants = &self.invariants;
        if !(invariants.argv_private
            && invariants.provider_neutral
            && invariants.typed_execution_posture
            && invariants.realm_accent_color_only
            && invariants.no_secrets_or_credentials)
        {
            return Err("realm-workloads-launcher-v2 invariants must all be true".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LauncherMetadataRuntimeState {
    ContractOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LauncherWorkloadSummary {
    pub identity: WorkloadIdentity,
    pub provider_kind: WorkloadProviderKind,
    pub execution_posture: WorkloadExecutionPosture,
    pub label: String,
    #[serde(default)]
    pub icon: LauncherIcon,
    pub realm_accent_color: String,
    pub launcher_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_item_id: Option<ProtocolToken>,
    #[serde(default)]
    pub capabilities: CapabilitySet,
    pub items: Vec<LauncherItemSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LauncherMetadataInvariants {
    pub argv_private: bool,
    pub provider_neutral: bool,
    pub typed_execution_posture: bool,
    pub realm_accent_color_only: bool,
    pub no_secrets_or_credentials: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_metadata_schema_has_no_argv_property() {
        let schema =
            serde_json::to_string(&schemars::schema_for!(RealmWorkloadsLauncherV2Json)).unwrap();
        assert!(!schema.contains("\"argv\""));
        assert!(schema.contains("\"providerKind\""));
        assert!(schema.contains("\"executionPosture\""));
    }

    #[test]
    fn launcher_metadata_requires_version_and_true_invariants() {
        let mut artifact: RealmWorkloadsLauncherV2Json =
            serde_json::from_value(serde_json::json!({
                "schemaVersion": "v2",
                "runtimeState": "contract-only",
                "workloads": [],
                "invariants": {
                    "argvPrivate": true,
                    "providerNeutral": true,
                    "typedExecutionPosture": true,
                    "realmAccentColorOnly": true,
                    "noSecretsOrCredentials": true
                }
            }))
            .unwrap();
        artifact.validate().unwrap();

        artifact.schema_version = "v1".to_owned();
        assert!(artifact.validate().is_err());
        artifact.schema_version = "v2".to_owned();
        artifact.invariants.argv_private = false;
        assert!(artifact.validate().is_err());
    }
}
