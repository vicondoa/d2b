use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use d2b_contracts::audio::LevelPercent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ListOutputV2(pub Vec<ListItemOutputV2>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListItemOutputV2 {
    pub name: String,
    pub env: Option<String>,
    pub graphics: bool,
    pub tpm: bool,
    pub usbip: bool,
    pub static_ip: Option<String>,
    pub status: String,
    pub is_net_vm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_closure_out_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autostart: Option<crate::public_wire::VmAutostartPosture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<crate::public_wire::QemuMediaStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_parity_ok: Option<bool>,
    /// Canonical realm-native workload target address (`<workload>.<realm>.d2b`).
    /// Present when the daemon has associated this entry with a realm workload
    /// identity. Absent for classical `d2b.vms` entries not yet adopted into
    /// a realm. Additive - old CLI consumers must tolerate its absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbProbeOutputV1 {
    pub command: String,
    pub entries: Vec<crate::public_wire::UsbipProbeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmListOutputV1 {
    pub command: String,
    pub realms: Vec<RealmPolicyOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RealmInspectOutputV1 {
    pub command: String,
    #[serde(flatten)]
    pub realm: RealmPolicyOutputV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpInspectOutputV1 {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<OpInspectTraceOutputV1>,
    pub local: OpInspectLocalOutputV1,
    pub realms: Vec<OpInspectRealmOutputV1>,
    pub degraded: Vec<OpInspectDegradedOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpInspectTraceOutputV1 {
    pub trace_id: String,
    pub span_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpInspectLocalOutputV1 {
    pub vm_count: u32,
    pub gateway_count: u32,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpInspectRealmOutputV1 {
    pub realm: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_vm: Option<String>,
    pub state: String,
    pub cross_realm_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpInspectDegradedOutputV1 {
    pub scope: String,
    pub reason: String,
    pub remediation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmPolicyOutputV1 {
    pub realm: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_vm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_target: Option<String>,
    pub gateway_state: String,
    pub cross_realm_policy: String,
    pub credential_boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum StatusOutputV2 {
    Vm(Box<StatusVmOutputV2>),
    Inventory(Box<StatusInventoryOutputV2>),
    CheckBridges(Box<StatusBridgeCheckOutputV2>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusInventoryOutputV2 {
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_model: Option<crate::public_wire::PublicReadModelMetadata>,
    pub vms: Vec<StatusVmOutputV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ApiReadyStatusV1 {
    Simple(ApiReadySimple),
    WithError(ApiReadyErrorV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiReadyErrorV1 {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ApiReadySimple {
    Yes,
    Pending,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusVmOutputV2 {
    pub name: String,
    pub env: Option<String>,
    pub services: StatusServicesOutputV2,
    pub current: Option<String>,
    pub booted: Option<String>,
    pub pending_restart: bool,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autostart: Option<crate::public_wire::VmAutostartPosture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<crate::public_wire::QemuMediaStatus>,
    pub declared_roles: Vec<String>,
    pub readiness: Vec<String>,
    /// api-ready state from the last vm start in split mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_ready: Option<ApiReadyStatusV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_parity: Option<RunnerParityOutputV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_pool_integrity: Option<LivePoolIntegrityOutputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb: Option<crate::public_wire::UsbipVmStatus>,
    /// Canonical realm-native workload target address (`<workload>.<realm>.d2b`).
    /// Present when the daemon has associated this VM with a realm workload
    /// identity. Absent for classical VMs not yet adopted into a realm.
    /// Additive - old CLI consumers must tolerate its absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LivePoolIntegrityOutputV1 {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_ref: Option<String>,
    pub repair_attempted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusServicesOutputV2 {
    pub d2b: String,
    pub microvm: String,
    pub virtiofsd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<String>,
    pub gpu: Option<String>,
    pub video: Option<String>,
    pub snd: Option<String>,
    pub swtpm: Option<String>,
}

/// Per-VM service-state map (V3) -- broker-spawn-aware status output.
///
/// All fields are optional so emitters can omit a role when the VM
/// doesn't enable it. The wire shape uses camelCase
/// + `deny_unknown_fields` to keep schema-drift gates honest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusServicesOutputV3 {
    /// Cloud Hypervisor runner state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hypervisor: Option<String>,
    /// Per-share virtiofsd state, keyed by share `tag`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub virtiofsd_per_share: BTreeMap<String, String>,
    /// crosvm GPU sidecar state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// vhost-device-sound audio sidecar state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    /// swtpm sidecar state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swtpm: Option<String>,
    /// Per-VM OtelGuestRelay state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel_relay: Option<String>,
    /// Host-scoped OtelHostBridge state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel_host_bridge: Option<String>,
    /// Per-env USBIP backend state, keyed by env name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub usbip_backend_per_env: BTreeMap<String, String>,
    /// Per-env USBIP proxy state, keyed by env name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub usbip_proxy_per_env: BTreeMap<String, String>,
}

impl StatusServicesOutputV3 {
    /// Conversion shim: takes a V2 record and projects it into V3
    /// by applying the documented rename map. Used so callers
    /// consuming the legacy V2 shape can be migrated incrementally
    /// without breaking the bundle-resolver / status-output contract.
    pub fn from_v2(v2: &StatusServicesOutputV2) -> Self {
        let mut virtiofsd_per_share = BTreeMap::new();
        // V2 had a single `virtiofsd` slot; we expose it under the
        // synthetic share tag `default` so the V3 consumer can read
        // it without losing data. v1.1.2+ wire bumps populate the
        // map per-share via the broker's per-share spawn records.
        virtiofsd_per_share.insert("default".to_owned(), v2.virtiofsd.clone());
        Self {
            hypervisor: Some(v2.microvm.clone()),
            virtiofsd_per_share,
            gpu: v2.gpu.clone(),
            // V3 has no dedicated video field yet; keep V2 authoritative
            // until a negotiated schema revision adds one.
            audio: v2.snd.clone(),
            swtpm: v2.swtpm.clone(),
            otel_relay: None,
            otel_host_bridge: None,
            usbip_backend_per_env: BTreeMap::new(),
            usbip_proxy_per_env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerParityOutputV2 {
    pub declared_runner: String,
    pub runner_parity_path: String,
    pub runner_parity_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusBridgeCheckOutputV2 {
    pub mode: String,
    pub status: String,
    pub message: String,
    pub runtime: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditOutputV2 {
    pub kvm_dev_mode: String,
    pub wayland_user_in_kvm: bool,
    pub store_delivery: BTreeMap<String, String>,
    pub virtiofsd: BTreeMap<String, AuditVirtiofsdOutputV2>,
    pub ssh: BTreeMap<String, AuditSshOutputV2>,
    pub bridge_isolation: BTreeMap<String, AuditBridgeIsolationOutputV2>,
    #[serde(rename = "autoUpgrade_commits_lock")]
    pub auto_upgrade_commits_lock: bool,
    pub ch_version: String,
    pub crosvm_rev: String,
    pub seccomp_rev: String,
    pub ch_crosvm_pair_ok: bool,
    pub fail2ban_active: bool,
    pub sidecars_per_vm: BTreeMap<String, AuditSidecarsOutputV2>,
    pub usbipd_per_env_isolation: BTreeMap<String, AuditUsbipEnvOutputV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditVirtiofsdOutputV2 {
    pub user: String,
    pub caps_dropped: Vec<String>,
    pub readonly_flag: bool,
    pub marker_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditSshOutputV2 {
    #[serde(rename = "PasswordAuthentication")]
    pub password_authentication: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditBridgeIsolationOutputV2 {
    pub bridge: String,
    pub tap: String,
    pub state: String,
    pub isolated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditSidecarsOutputV2 {
    pub gpu_active: bool,
    pub snd_active: bool,
    pub gpu_user: String,
    pub snd_user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditUsbipEnvOutputV2 {
    pub socket_active: bool,
    pub backend_active: bool,
    pub lock_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthStatusOutputV2 {
    pub role: AuthRoleV2,
    pub effective_uid: u32,
    pub sockets: Vec<AuthSocketStatusV2>,
    pub allowed_subcommands: Vec<String>,
    pub denied_subcommands: Vec<AuthDeniedSubcommandV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuthRoleV2 {
    None,
    Launcher,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthSocketStatusV2 {
    pub name: String,
    pub path: String,
    pub reachable: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthDeniedSubcommandV2 {
    pub name: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_wire::VmAutostartPosture;
    use serde_json::json;

    fn full_list_item() -> ListItemOutputV2 {
        ListItemOutputV2 {
            name: "corp-vm".to_owned(),
            env: Some("work".to_owned()),
            graphics: true,
            tpm: false,
            usbip: true,
            static_ip: Some("10.0.0.5".to_owned()),
            status: "running".to_owned(),
            is_net_vm: true,
            guest_closure_out_path: Some("/nix/store/closure".to_owned()),
            runtime_kind: Some("cloud-hypervisor".to_owned()),
            autostart: Some(VmAutostartPosture {
                mode: "on".to_owned(),
                reason: "policy".to_owned(),
            }),
            runtime_capabilities: vec!["gpu".to_owned()],
            service_capabilities: vec!["audio".to_owned()],
            unsupported_capabilities: Vec::new(),
            qemu_media: None,
            runner_parity_ok: Some(true),
            canonical_target: Some("corp.work.d2b".to_owned()),
        }
    }

    #[test]
    fn list_item_output_uses_camel_case_and_omits_defaulted_fields() {
        let item = full_list_item();
        let value = serde_json::to_value(&item).unwrap();
        let object = value.as_object().unwrap();
        for key in [
            "name",
            "env",
            "graphics",
            "tpm",
            "usbip",
            "staticIp",
            "status",
            "isNetVm",
            "guestClosureOutPath",
            "runtimeKind",
            "autostart",
            "runtimeCapabilities",
            "serviceCapabilities",
            "runnerParityOk",
            "canonicalTarget",
        ] {
            assert!(object.contains_key(key), "missing camelCase key {key}");
        }
        // Empty vectors and absent optionals are omitted from the wire.
        assert!(!object.contains_key("unsupportedCapabilities"));
        assert!(!object.contains_key("qemuMedia"));
        assert_eq!(
            serde_json::from_value::<ListItemOutputV2>(value).unwrap(),
            item
        );
        // deny_unknown_fields: a drifted key fails closed on decode.
        let mut drifted = serde_json::to_value(&item).unwrap();
        drifted
            .as_object_mut()
            .unwrap()
            .insert("guestClosureOutPathX".to_owned(), json!("drift"));
        assert!(serde_json::from_value::<ListItemOutputV2>(drifted).is_err());
        // The V2 list wrapper is transparent: a bare array on the wire.
        let list = ListOutputV2(vec![item]);
        assert!(serde_json::to_value(&list).unwrap().is_array());
    }

    #[test]
    fn status_services_v3_and_audit_outputs_pin_exact_wire_keys() {
        let services = StatusServicesOutputV3 {
            hypervisor: Some("running".to_owned()),
            virtiofsd_per_share: BTreeMap::from([("default".to_owned(), "running".to_owned())]),
            gpu: None,
            audio: Some("running".to_owned()),
            swtpm: None,
            otel_relay: None,
            otel_host_bridge: Some("running".to_owned()),
            usbip_backend_per_env: BTreeMap::new(),
            usbip_proxy_per_env: BTreeMap::new(),
        };
        let value = serde_json::to_value(&services).unwrap();
        let object = value.as_object().unwrap();
        for key in ["hypervisor", "virtiofsdPerShare", "audio", "otelHostBridge"] {
            assert!(object.contains_key(key), "missing camelCase key {key}");
        }
        for key in ["gpu", "swtpm", "otelRelay", "usbipBackendPerEnv", "usbipProxyPerEnv"] {
            assert!(
                !object.contains_key(key),
                "empty or absent field {key} must be omitted"
            );
        }
        assert_eq!(
            serde_json::from_value::<StatusServicesOutputV3>(value).unwrap(),
            services
        );
        let mut drifted = serde_json::to_value(&services).unwrap();
        drifted
            .as_object_mut()
            .unwrap()
            .insert("hypervisorX".to_owned(), json!("drift"));
        assert!(serde_json::from_value::<StatusServicesOutputV3>(drifted).is_err());

        // The audit output pins its explicit legacy renames verbatim.
        let audit = AuditOutputV2 {
            kvm_dev_mode: "0666".to_owned(),
            wayland_user_in_kvm: false,
            store_delivery: BTreeMap::new(),
            virtiofsd: BTreeMap::new(),
            ssh: BTreeMap::from([(
                "host".to_owned(),
                AuditSshOutputV2 {
                    password_authentication: Some(true),
                },
            )]),
            bridge_isolation: BTreeMap::new(),
            auto_upgrade_commits_lock: true,
            ch_version: "1.2.3".to_owned(),
            crosvm_rev: "rev".to_owned(),
            seccomp_rev: "rev".to_owned(),
            ch_crosvm_pair_ok: true,
            fail2ban_active: false,
            sidecars_per_vm: BTreeMap::new(),
            usbipd_per_env_isolation: BTreeMap::new(),
        };
        let value = serde_json::to_value(&audit).unwrap();
        assert_eq!(value["autoUpgrade_commits_lock"], json!(true));
        assert_eq!(value["ssh"]["host"]["PasswordAuthentication"], json!(true));
        assert_eq!(serde_json::from_value::<AuditOutputV2>(value).unwrap(), audit);
        let mut drifted = serde_json::to_value(&audit).unwrap();
        drifted
            .as_object_mut()
            .unwrap()
            .insert("autoUpgrade_commits_lockX".to_owned(), json!(true));
        assert!(serde_json::from_value::<AuditOutputV2>(drifted).is_err());
    }
}

