//! Private configured-item contract for unsafe-local workloads.

use crate::{configured_argv::ConfiguredArgv, workload_identity::WorkloadIdentity};
use d2b_realm_core::{LauncherIcon, LauncherItemKind, ProtocolToken};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const UNSAFE_LOCAL_WORKLOADS_SCHEMA_VERSION: &str = "v2";
pub const MAX_UNSAFE_LOCAL_WORKLOADS: usize = 16;
pub const MAX_LOCAL_VM_CONFIGURED_WORKLOADS: usize = 256;
pub const MAX_PRIVATE_CONFIGURED_WORKLOADS: usize =
    MAX_UNSAFE_LOCAL_WORKLOADS + MAX_LOCAL_VM_CONFIGURED_WORKLOADS;
pub const MAX_LAUNCHER_ITEMS_PER_WORKLOAD: usize = 64;
pub const MAX_UNSAFE_LOCAL_SHELL_SESSIONS: u16 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalWorkloadsJson {
    pub schema_version: String,
    #[schemars(length(max = 16))]
    pub workloads: Vec<UnsafeLocalWorkload>,
    /// Configured launcher items for local VM workloads. They share this
    /// private, bundle-hashed artifact so argv never enters public launcher
    /// metadata or the public request protocol.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 256))]
    pub local_vm_workloads: Vec<LocalVmConfiguredWorkload>,
}

impl UnsafeLocalWorkloadsJson {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != UNSAFE_LOCAL_WORKLOADS_SCHEMA_VERSION {
            return Err(format!(
                "unsafe-local-workloads schemaVersion must be {UNSAFE_LOCAL_WORKLOADS_SCHEMA_VERSION}"
            ));
        }
        if self.workloads.len() > MAX_UNSAFE_LOCAL_WORKLOADS {
            return Err(format!(
                "unsafe-local workload count exceeds {MAX_UNSAFE_LOCAL_WORKLOADS}"
            ));
        }
        if self.local_vm_workloads.len() > MAX_LOCAL_VM_CONFIGURED_WORKLOADS {
            return Err(format!(
                "local-vm configured workload count exceeds {MAX_LOCAL_VM_CONFIGURED_WORKLOADS}"
            ));
        }
        if self.workloads.len() + self.local_vm_workloads.len() > MAX_PRIVATE_CONFIGURED_WORKLOADS {
            return Err(format!(
                "private configured workload count exceeds {MAX_PRIVATE_CONFIGURED_WORKLOADS}"
            ));
        }
        let mut targets = BTreeSet::new();
        for workload in &self.workloads {
            let target = workload.identity.canonical_target.to_canonical();
            if !targets.insert(target.clone()) {
                return Err(format!("duplicate unsafe-local workload target {target}"));
            }
            workload.validate()?;
        }
        for workload in &self.local_vm_workloads {
            let target = workload.identity.canonical_target.to_canonical();
            if !targets.insert(target.clone()) {
                return Err(format!("duplicate configured workload target {target}"));
            }
            workload.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalVmConfiguredWorkload {
    pub identity: WorkloadIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_item_id: Option<ProtocolToken>,
    #[schemars(length(min = 1, max = 64))]
    pub items: Vec<UnsafeLocalLauncherItem>,
}

impl LocalVmConfiguredWorkload {
    pub fn validate(&self) -> Result<(), String> {
        if self.identity.runtime_kind.as_ref().map(|id| id.as_str()) != Some("nixos") {
            return Err("local-vm configured workload must use nixos runtimeKind".to_owned());
        }
        validate_items(&self.items, self.default_item_id.as_ref(), true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalWorkload {
    pub identity: WorkloadIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_item_id: Option<ProtocolToken>,
    #[schemars(length(min = 1, max = 64))]
    pub items: Vec<UnsafeLocalLauncherItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<UnsafeLocalShellPolicy>,
}

impl UnsafeLocalWorkload {
    pub fn validate(&self) -> Result<(), String> {
        if self.identity.legacy_vm_name.is_some() {
            return Err("unsafe-local workload must not carry legacyVmName".to_owned());
        }
        if self.identity.runtime_kind.as_ref().map(|id| id.as_str()) != Some("unsafe-local")
            || self.identity.provider_id.as_ref().map(|id| id.as_str()) != Some("unsafe-local")
        {
            return Err(
                "unsafe-local workload identity must use unsafe-local runtimeKind and providerId"
                    .to_owned(),
            );
        }
        validate_items(
            &self.items,
            self.default_item_id.as_ref(),
            self.shell.is_some(),
        )?;
        if let Some(shell) = &self.shell {
            shell.validate()?;
        }
        Ok(())
    }
}

fn validate_items(
    items: &[UnsafeLocalLauncherItem],
    default_item_id: Option<&ProtocolToken>,
    shell_enabled: bool,
) -> Result<(), String> {
    if items.is_empty() {
        return Err("configured workload must declare at least one launcher item".to_owned());
    }
    if items.len() > MAX_LAUNCHER_ITEMS_PER_WORKLOAD {
        return Err(format!(
            "configured launcher item count exceeds {MAX_LAUNCHER_ITEMS_PER_WORKLOAD}"
        ));
    }
    let mut ids = BTreeSet::new();
    for item in items {
        if !ids.insert(item.id()) {
            return Err(format!(
                "duplicate configured launcher item id {}",
                item.id().as_str()
            ));
        }
        if matches!(item, UnsafeLocalLauncherItem::Shell(_)) && !shell_enabled {
            return Err("shell launcher item requires shell policy".to_owned());
        }
    }
    if let Some(default_item_id) = default_item_id
        && !ids.contains(default_item_id)
    {
        return Err(format!(
            "defaultItem {} does not name a declared launcher item",
            default_item_id.as_str()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UnsafeLocalLauncherItem {
    Exec(UnsafeLocalExecItem),
    Shell(UnsafeLocalShellItem),
}

impl UnsafeLocalLauncherItem {
    pub fn id(&self) -> &ProtocolToken {
        match self {
            Self::Exec(item) => &item.id,
            Self::Shell(item) => &item.id,
        }
    }

    pub fn kind(&self) -> LauncherItemKind {
        match self {
            Self::Exec(_) => LauncherItemKind::Exec,
            Self::Shell(_) => LauncherItemKind::Shell,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalExecItem {
    pub id: ProtocolToken,
    pub name: String,
    #[serde(default)]
    pub icon: LauncherIcon,
    pub argv: ConfiguredArgv,
    #[serde(default)]
    pub graphical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalShellItem {
    pub id: ProtocolToken,
    pub name: String,
    #[serde(default)]
    pub icon: LauncherIcon,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalShellPolicy {
    pub default_name: String,
    pub max_sessions: u16,
}

impl std::fmt::Debug for UnsafeLocalShellPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnsafeLocalShellPolicy")
            .field("default_name", &"<redacted>")
            .field("max_sessions", &self.max_sessions)
            .finish()
    }
}

impl UnsafeLocalShellPolicy {
    fn validate(&self) -> Result<(), String> {
        if self.default_name.is_empty() || self.default_name.contains('\0') {
            return Err("unsafe-local shell defaultName must be non-empty and NUL-free".to_owned());
        }
        if self.max_sessions == 0 || self.max_sessions > MAX_UNSAFE_LOCAL_SHELL_SESSIONS {
            return Err(format!(
                "unsafe-local shell maxSessions must be between 1 and {MAX_UNSAFE_LOCAL_SHELL_SESSIONS}"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::{
        ids::{RealmId, WorkloadId},
        realm::RealmPath,
    };

    fn identity() -> WorkloadIdentity {
        let realm_id = RealmId::parse("host").unwrap();
        let mut identity = WorkloadIdentity::new(
            WorkloadId::parse("tools").unwrap(),
            realm_id.clone(),
            RealmPath::new(vec![realm_id]).unwrap(),
            crate::workload_identity::WorkloadTarget::parse("tools.host.d2b").unwrap(),
        );
        identity.runtime_kind =
            Some(crate::contract_id::ContractId::parse("unsafe-local").unwrap());
        identity.provider_id = Some(crate::contract_id::ContractId::parse("unsafe-local").unwrap());
        identity
    }

    fn exec_item() -> UnsafeLocalLauncherItem {
        UnsafeLocalLauncherItem::Exec(UnsafeLocalExecItem {
            id: ProtocolToken::parse("browser").unwrap(),
            name: "Browser".to_owned(),
            icon: LauncherIcon::default(),
            argv: ConfiguredArgv::new(vec!["firefox".to_owned()]).unwrap(),
            graphical: true,
        })
    }

    fn valid_workload() -> UnsafeLocalWorkload {
        UnsafeLocalWorkload {
            identity: identity(),
            default_item_id: Some(ProtocolToken::parse("browser").unwrap()),
            items: vec![exec_item()],
            shell: None,
        }
    }

    fn valid_local_vm_workload() -> LocalVmConfiguredWorkload {
        let mut identity = identity();
        identity.runtime_kind = Some(crate::contract_id::ContractId::parse("nixos").unwrap());
        identity.provider_id =
            Some(crate::contract_id::ContractId::parse("local-cloud-hypervisor").unwrap());
        LocalVmConfiguredWorkload {
            identity,
            default_item_id: Some(ProtocolToken::parse("browser").unwrap()),
            items: vec![exec_item()],
        }
    }

    #[test]
    fn artifact_validates_default_and_redacts_argv_debug() {
        let artifact = UnsafeLocalWorkloadsJson {
            schema_version: "v2".to_owned(),
            workloads: vec![UnsafeLocalWorkload {
                identity: identity(),
                default_item_id: Some(ProtocolToken::parse("browser").unwrap()),
                items: vec![exec_item()],
                shell: None,
            }],
            local_vm_workloads: Vec::new(),
        };
        artifact.validate().unwrap();
        assert!(!format!("{artifact:?}").contains("firefox"));
    }

    #[test]
    fn shell_item_requires_policy() {
        let workload = UnsafeLocalWorkload {
            identity: identity(),
            default_item_id: None,
            items: vec![UnsafeLocalLauncherItem::Shell(UnsafeLocalShellItem {
                id: ProtocolToken::parse("terminal").unwrap(),
                name: "Terminal".to_owned(),
                icon: LauncherIcon::default(),
            })],
            shell: None,
        };
        assert!(workload.validate().is_err());
    }

    #[test]
    fn shell_policy_debug_hides_default_name() {
        let canary = "private-shell-name-canary";
        let shell = UnsafeLocalShellPolicy {
            default_name: canary.to_owned(),
            max_sessions: 8,
        };
        assert!(!format!("{shell:?}").contains(canary));
    }

    #[test]
    fn first_class_local_vm_workload_does_not_require_legacy_name() {
        let mut workload = valid_local_vm_workload();
        workload.identity.legacy_vm_name = None;

        workload.validate().unwrap();
    }

    #[test]
    fn artifact_rejects_wrong_schema_version() {
        let artifact = UnsafeLocalWorkloadsJson {
            schema_version: "v1".to_owned(),
            workloads: vec![valid_workload()],
            local_vm_workloads: Vec::new(),
        };
        assert!(artifact.validate().is_err());
    }

    #[test]
    fn artifact_enforces_separate_workload_and_item_bounds() {
        let unsafe_overflow = UnsafeLocalWorkloadsJson {
            schema_version: "v2".to_owned(),
            workloads: vec![valid_workload(); MAX_UNSAFE_LOCAL_WORKLOADS + 1],
            local_vm_workloads: Vec::new(),
        };
        assert_eq!(
            unsafe_overflow.validate().unwrap_err(),
            format!("unsafe-local workload count exceeds {MAX_UNSAFE_LOCAL_WORKLOADS}")
        );

        let local_vm_overflow = UnsafeLocalWorkloadsJson {
            schema_version: "v2".to_owned(),
            workloads: Vec::new(),
            local_vm_workloads: vec![
                valid_local_vm_workload();
                MAX_LOCAL_VM_CONFIGURED_WORKLOADS + 1
            ],
        };
        assert_eq!(
            local_vm_overflow.validate().unwrap_err(),
            format!(
                "local-vm configured workload count exceeds {MAX_LOCAL_VM_CONFIGURED_WORKLOADS}"
            )
        );

        let mut workload = valid_workload();
        workload.items = vec![exec_item(); MAX_LAUNCHER_ITEMS_PER_WORKLOAD + 1];
        assert!(workload.validate().is_err());
    }

    #[test]
    fn schema_exposes_private_artifact_allocation_bounds() {
        let schema = serde_json::to_value(schemars::schema_for!(UnsafeLocalWorkloadsJson)).unwrap();
        assert_eq!(
            schema["properties"]["workloads"]["maxItems"],
            MAX_UNSAFE_LOCAL_WORKLOADS
        );
        assert_eq!(
            schema["properties"]["localVmWorkloads"]["maxItems"],
            MAX_LOCAL_VM_CONFIGURED_WORKLOADS
        );
        assert_eq!(
            schema["definitions"]["UnsafeLocalWorkload"]["properties"]["items"]["minItems"],
            1
        );
        assert_eq!(
            schema["definitions"]["UnsafeLocalWorkload"]["properties"]["items"]["maxItems"],
            MAX_LAUNCHER_ITEMS_PER_WORKLOAD
        );
        assert_eq!(
            schema["definitions"]["LocalVmConfiguredWorkload"]["properties"]["items"]["minItems"],
            1
        );
        assert_eq!(
            schema["definitions"]["LocalVmConfiguredWorkload"]["properties"]["items"]["maxItems"],
            MAX_LAUNCHER_ITEMS_PER_WORKLOAD
        );
    }

    #[test]
    fn artifact_rejects_shell_quota_overflow() {
        let mut workload = valid_workload();
        workload.shell = Some(UnsafeLocalShellPolicy {
            default_name: "host".to_owned(),
            max_sessions: MAX_UNSAFE_LOCAL_SHELL_SESSIONS + 1,
        });
        assert!(workload.validate().is_err());
    }

    #[test]
    fn artifact_rejects_mismatched_runtime_identity() {
        let mut workload = valid_workload();
        workload.identity.runtime_kind =
            Some(crate::contract_id::ContractId::parse("nixos").unwrap());
        assert!(workload.validate().is_err());

        let mut workload = valid_workload();
        workload.identity.provider_id =
            Some(crate::contract_id::ContractId::parse("local-cloud-hypervisor").unwrap());
        assert!(workload.validate().is_err());
    }

    #[test]
    fn artifact_rejects_missing_default_item() {
        let mut workload = valid_workload();
        workload.default_item_id = Some(ProtocolToken::parse("missing").unwrap());
        assert!(workload.validate().is_err());
    }

    #[test]
    fn local_vm_items_share_private_artifact_without_public_argv() {
        let mut local_identity = identity();
        local_identity.legacy_vm_name =
            Some(crate::contract_id::ContractId::parse("corp-vm").unwrap());
        local_identity.runtime_kind = Some(crate::contract_id::ContractId::parse("nixos").unwrap());
        local_identity.provider_id =
            Some(crate::contract_id::ContractId::parse("local-cloud-hypervisor").unwrap());
        let artifact = UnsafeLocalWorkloadsJson {
            schema_version: "v2".to_owned(),
            workloads: Vec::new(),
            local_vm_workloads: vec![LocalVmConfiguredWorkload {
                identity: local_identity,
                default_item_id: Some(ProtocolToken::parse("browser").unwrap()),
                items: vec![exec_item()],
            }],
        };
        artifact.validate().unwrap();
        let json = serde_json::to_value(&artifact).unwrap();
        assert_eq!(
            json["localVmWorkloads"][0]["items"][0]["argv"][0],
            "firefox"
        );
        assert!(!format!("{artifact:?}").contains("firefox"));
    }
}
