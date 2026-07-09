//! Workload identity foundation for realm-native bundle artifacts.
//!
//! Every workload is addressed by a [`WorkloadTarget`]
//! (`<workload>.<realmPath>.d2b`) and carries a [`WorkloadIdentity`] that
//! separates the universal realm-scoped identity from the backend runtime
//! config in [`WorkloadBackend`].
//!
//! Use [`WorkloadRuntimeIntent`] in process-intent and SpawnRunner-adjacent
//! DTOs where both the realm identity and the local backend config must travel
//! together.
//!
//! # DTO version policy
//!
//! **Additive changes** — adding a new `Option<T>` field to an existing struct
//! with `#[serde(default, skip_serializing_if = "Option::is_none")]`, or adding
//! a new variant to [`WorkloadBackend`] — do not require a `bundleVersion` or
//! `schemaVersion` bump, because:
//! - New code reading old JSON: the missing field deserializes as `None` / the
//!   missing variant is unknown-but-skipped (if the consumer uses `deny_unknown_fields`
//!   + `default`, callers still need to update their emitters promptly).
//! - Old code reading new JSON: `deny_unknown_fields` means the old code will
//!   reject the new field, so Nix emitter and Rust DTO **must be updated
//!   together** in the same commit.
//!
//! **Breaking changes** — removing or renaming a required field, changing the
//! type of an existing field, or removing a variant — require **both** a
//! `bundleVersion` bump (in `packages/xtask/`) and a `schemaVersion` bump (in
//! `docs/reference/schemas/`) together with a `CHANGELOG.md` entry under
//! `## [Unreleased]`.

use d2b_realm_core::ids::{RealmId, WorkloadId};
use d2b_realm_core::realm::RealmPath;
use d2b_realm_core::target::RealmTarget;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contract_id::ContractId;

/// Canonical workload target address: `<workload>.<realmPath>.d2b`.
///
/// This is a transparent type alias for [`RealmTarget`] for use in bundle
/// artifacts. Parse via [`WorkloadTarget::parse`] (equivalently,
/// `RealmTarget::parse`); do not split the string manually past the parse
/// boundary.
///
/// # Examples
///
/// ```
/// use d2b_core::workload_identity::WorkloadTarget;
///
/// let t = WorkloadTarget::parse("builder.dev.d2b").unwrap();
/// assert_eq!(t.to_canonical(), "builder.dev.d2b");
/// assert_eq!(t.workload.as_str(), "builder");
/// ```
pub type WorkloadTarget = RealmTarget;

/// Universal workload identity, independent of any runtime or provider backend.
///
/// Separates the stable, realm-scoped identity of a workload from the backend
/// configuration in [`WorkloadBackend`]. Any code that routes, logs, or audits
/// a workload should carry a `WorkloadIdentity` rather than raw VM-name strings
/// so the realm context is always available.
///
/// # DTO version policy
///
/// See the [module-level note][crate::workload_identity] on additive vs.
/// breaking changes. Adding new `Option<T>` fields here is additive; renaming
/// or removing fields requires a `bundleVersion` + `schemaVersion` bump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadIdentity {
    /// Stable operator-facing workload identifier (the leading label of the
    /// canonical target address).
    pub workload_id: WorkloadId,
    /// Human-readable workload name if it differs from the id. Omitted when
    /// the workload name and id are identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_name: Option<String>,
    /// Realm identifier of the owning realm controller.
    pub realm_id: RealmId,
    /// Realm path (most-specific first) — the label sequence after the
    /// workload in the canonical target address.
    pub realm_path: RealmPath,
    /// Fully-qualified canonical target address, kept pre-rendered to avoid
    /// repeated formatting and to make it audit-log safe.
    pub canonical_target: WorkloadTarget,
    /// Legacy `d2b.vms.<vm>` name for workloads that exist as a classical VM
    /// entry while the realm-native model is being adopted. `None` for
    /// workloads declared directly inside a realm without a legacy VM entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_vm_name: Option<ContractId>,
    /// Opaque runtime kind identifier (e.g. `nixos`, `qemu-media`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_kind: Option<ContractId>,
    /// Stable provider identifier within the realm (e.g.
    /// `local-cloud-hypervisor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ContractId>,
}

impl WorkloadIdentity {
    /// Build a `WorkloadIdentity` from the required core fields, with all
    /// optional fields left as `None`.
    pub fn new(
        workload_id: WorkloadId,
        realm_id: RealmId,
        realm_path: RealmPath,
        canonical_target: WorkloadTarget,
    ) -> Self {
        Self {
            workload_id,
            workload_name: None,
            realm_id,
            realm_path,
            canonical_target,
            legacy_vm_name: None,
            runtime_kind: None,
            provider_id: None,
        }
    }

    /// Return the canonical target address as a borrowed [`WorkloadTarget`].
    pub fn target(&self) -> &WorkloadTarget {
        &self.canonical_target
    }
}

/// Backend-specific configuration envelope for a workload's local runtime.
///
/// The universal identity in [`WorkloadIdentity`] is always present; this enum
/// carries whatever the owning provider or runner needs in addition.
///
/// # DTO version policy
///
/// Adding a new variant here is additive — consumers that do not recognise the
/// variant can reject it explicitly or skip it depending on context. Removing
/// or renaming a variant is breaking and requires a `bundleVersion` +
/// `schemaVersion` bump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WorkloadBackend {
    /// Local Cloud Hypervisor / NixOS runner.
    LocalVm(LocalVmBackendConfig),
    /// Local QEMU-media runner.
    LocalQemuMedia(LocalQemuMediaBackendConfig),
}

/// Backend config for a locally supervised Cloud Hypervisor / NixOS workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalVmBackendConfig {
    /// VM id as carried in process-intent and broker ops.
    pub vm_id: ContractId,
    /// Network env this workload is assigned to.
    pub env: ContractId,
}

/// Backend config for a locally supervised QEMU-media workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalQemuMediaBackendConfig {
    /// VM id as carried in process-intent and broker ops.
    pub vm_id: ContractId,
    /// Network env this workload is assigned to.
    pub env: ContractId,
}

/// A workload's runtime intent: universal identity plus the backend envelope.
///
/// Use this struct in place of bare VM-id + env strings wherever a DTO needs
/// to carry both the realm-native identity and the local runner context. It
/// provides the structural separation between the universal identity and the
/// provider-specific config that the `SpawnRunner` and related process-intent
/// DTOs need.
///
/// # DTO version policy
///
/// See the [module-level note][crate::workload_identity] on additive vs.
/// breaking changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadRuntimeIntent {
    /// Universal realm-scoped identity.
    pub identity: WorkloadIdentity,
    /// Backend-specific runtime configuration.
    pub backend: WorkloadBackend,
}

#[cfg(test)]
mod tests {
    use d2b_realm_core::ids::{RealmId, WorkloadId};
    use d2b_realm_core::realm::RealmPath;

    use super::*;

    fn make_realm_path(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    fn make_identity(workload: &str, realm: &str) -> WorkloadIdentity {
        let workload_id = WorkloadId::parse(workload).unwrap();
        let realm_id = RealmId::parse(realm).unwrap();
        let realm_path = make_realm_path(realm);
        let canonical_target =
            WorkloadTarget::parse(&format!("{}.{}.d2b", workload, realm)).unwrap();
        WorkloadIdentity::new(workload_id, realm_id, realm_path, canonical_target)
    }

    #[test]
    fn workload_target_parse_canonical() {
        let t = WorkloadTarget::parse("builder.dev.d2b").unwrap();
        assert_eq!(t.to_canonical(), "builder.dev.d2b");
        assert_eq!(t.workload.as_str(), "builder");
    }

    #[test]
    fn workload_target_parse_nested_realm() {
        let t = WorkloadTarget::parse("api.payments.work.d2b").unwrap();
        assert_eq!(t.to_canonical(), "api.payments.work.d2b");
        assert_eq!(t.workload.as_str(), "api");
        assert_eq!(t.realm.target_form(), "payments.work");
    }

    #[test]
    fn workload_target_rejects_no_dot() {
        assert!(WorkloadTarget::parse("builder").is_err());
    }

    #[test]
    fn workload_target_rejects_missing_d2b_suffix() {
        assert!(WorkloadTarget::parse("builder.dev.org").is_err());
    }

    #[test]
    fn workload_identity_new_has_none_optional_fields() {
        let id = make_identity("demo", "work");
        assert!(id.workload_name.is_none());
        assert!(id.legacy_vm_name.is_none());
        assert!(id.runtime_kind.is_none());
        assert!(id.provider_id.is_none());
        assert_eq!(id.canonical_target.to_canonical(), "demo.work.d2b");
    }

    #[test]
    fn workload_identity_target_accessor() {
        let id = make_identity("api", "dev");
        assert_eq!(id.target().to_canonical(), "api.dev.d2b");
    }

    #[test]
    fn workload_identity_round_trips_minimal() {
        let id = make_identity("ci", "build");
        let json = serde_json::to_string(&id).unwrap();
        let back: WorkloadIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn workload_identity_round_trips_with_optional_fields() {
        let mut id = make_identity("work-vm", "dev");
        id.workload_name = Some("Work Dev VM".to_owned());
        id.legacy_vm_name = Some(ContractId::parse("corp-vm").unwrap());
        id.runtime_kind = Some(ContractId::parse("nixos").unwrap());
        id.provider_id = Some(ContractId::parse("local-cloud-hypervisor").unwrap());

        let json = serde_json::to_string(&id).unwrap();
        let back: WorkloadIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
        assert_eq!(back.legacy_vm_name.as_ref().unwrap().as_str(), "corp-vm");
        assert_eq!(back.runtime_kind.as_ref().unwrap().as_str(), "nixos");
        assert_eq!(
            back.provider_id.as_ref().unwrap().as_str(),
            "local-cloud-hypervisor"
        );
    }

    #[test]
    fn workload_identity_skips_none_fields_in_json() {
        let id = make_identity("demo", "work");
        let json = serde_json::to_string(&id).unwrap();
        assert!(!json.contains("workloadName"));
        assert!(!json.contains("legacyVmName"));
        assert!(!json.contains("runtimeKind"));
        assert!(!json.contains("providerId"));
    }

    #[test]
    fn workload_identity_rejects_unknown_fields() {
        let json = r#"{"workloadId":"demo","realmId":"work","realmPath":["work"],"canonicalTarget":"demo.work.d2b","unexpected":"value"}"#;
        let result = serde_json::from_str::<WorkloadIdentity>(json);
        assert!(
            result.is_err(),
            "deny_unknown_fields must reject extra keys"
        );
    }

    #[test]
    fn workload_backend_local_vm_round_trips() {
        let backend = WorkloadBackend::LocalVm(LocalVmBackendConfig {
            vm_id: ContractId::parse("corp-vm").unwrap(),
            env: ContractId::parse("work").unwrap(),
        });
        let json = serde_json::to_string(&backend).unwrap();
        assert!(json.contains("\"kind\":\"local-vm\""));
        let back: WorkloadBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, backend);
    }

    #[test]
    fn workload_backend_local_qemu_media_round_trips() {
        let backend = WorkloadBackend::LocalQemuMedia(LocalQemuMediaBackendConfig {
            vm_id: ContractId::parse("iso-runner").unwrap(),
            env: ContractId::parse("personal").unwrap(),
        });
        let json = serde_json::to_string(&backend).unwrap();
        assert!(json.contains("\"kind\":\"local-qemu-media\""));
        let back: WorkloadBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, backend);
    }

    #[test]
    fn workload_runtime_intent_round_trips() {
        let intent = WorkloadRuntimeIntent {
            identity: make_identity("corp-vm", "work"),
            backend: WorkloadBackend::LocalVm(LocalVmBackendConfig {
                vm_id: ContractId::parse("corp-vm").unwrap(),
                env: ContractId::parse("work").unwrap(),
            }),
        };
        let json = serde_json::to_string(&intent).unwrap();
        let back: WorkloadRuntimeIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.identity.canonical_target.to_canonical(),
            "corp-vm.work.d2b"
        );
        assert!(matches!(back.backend, WorkloadBackend::LocalVm(_)));
    }

    #[test]
    fn workload_runtime_intent_rejects_unknown_fields() {
        let id = make_identity("demo", "dev");
        let backend = WorkloadBackend::LocalVm(LocalVmBackendConfig {
            vm_id: ContractId::parse("demo").unwrap(),
            env: ContractId::parse("dev").unwrap(),
        });
        let intent = WorkloadRuntimeIntent {
            identity: id,
            backend,
        };
        let mut json_val: serde_json::Value = serde_json::to_value(&intent).unwrap();
        json_val["extraField"] = serde_json::Value::String("bad".to_owned());
        let result = serde_json::from_value::<WorkloadRuntimeIntent>(json_val);
        assert!(
            result.is_err(),
            "deny_unknown_fields must reject extra keys"
        );
    }
}
