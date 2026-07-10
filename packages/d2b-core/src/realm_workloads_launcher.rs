use crate::workload_identity::WorkloadIdentity;
use d2b_realm_core::{
    CapabilitySet, LauncherIcon, LauncherItemSummary, ProtocolToken, WorkloadExecutionPosture,
    WorkloadProviderKind,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmWorkloadsLauncherV2Json {
    pub schema_version: String,
    pub runtime_state: LauncherMetadataRuntimeState,
    pub workloads: Vec<LauncherWorkloadSummary>,
    pub invariants: LauncherMetadataInvariants,
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
}
