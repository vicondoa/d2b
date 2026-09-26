use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
/// `d2b vm list` output: one row per VM.
pub struct ListOutputV2(pub Vec<ListItemOutputV2>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One `d2b vm list` row.
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
/// `usb probe` output: the command echo plus one entry per probed device.
pub struct UsbProbeOutputV1 {
    pub command: String,
    pub entries: Vec<crate::public_wire::UsbipProbeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// `realm list` output: one policy summary per realm.
pub struct RealmListOutputV1 {
    pub command: String,
    pub realms: Vec<RealmPolicyOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
/// `realm inspect` output: the flattened policy summary of one realm.
pub struct RealmInspectOutputV1 {
    pub command: String,
    #[serde(flatten)]
    pub realm: RealmPolicyOutputV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// `op inspect` output: trace, local, and per-realm views.
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
/// Trace identifiers for one `op inspect` run.
pub struct OpInspectTraceOutputV1 {
    pub trace_id: String,
    pub span_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Local counts and source for one `op inspect` run.
pub struct OpInspectLocalOutputV1 {
    pub vm_count: u32,
    pub gateway_count: u32,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One realm's view in `op inspect` output.
pub struct OpInspectRealmOutputV1 {
    pub realm: String,
    pub mode: RealmMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_vm: Option<String>,
    pub state: RealmGatewayState,
    pub cross_realm_policy: String,
}

/// How a realm's entrypoint is dispatched.
///
/// `mode` used to be a free-form `String`; the realm entrypoint table admits
/// exactly two modes, so both the wire protocol and the generated CLI schema
/// carry enum constraints rather than a free-form string. Serde names match
/// the canonical wire strings exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmMode {
    /// The realm entrypoint runs on the local daemon.
    HostResident,
    /// A gateway guest fronts the realm and owns its policy.
    GatewayBacked,
}

/// The gateway-side state of a realm row.
///
/// `gateway_state` and `state` used to be free-form `String`s carrying the
/// gateway guest's lifecycle label; the field is `local-only` for a
/// host-resident realm, the daemon's lifecycle state for a gateway-backed
/// realm whose gateway the daemon listed, and the preserved sentinel string
/// when the daemon did not list the gateway at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmGatewayState {
    /// The realm has no gateway hop; it is dispatched on this host.
    LocalOnly,
    /// The gateway guest is stopped.
    Stopped,
    /// The gateway guest is starting.
    Starting,
    /// The gateway guest has booted.
    Booted,
    /// The gateway guest is running.
    Running,
    /// The gateway guest is stopping.
    Stopping,
    /// The gateway guest is restarting.
    Restarting,
    /// The gateway guest failed its last lifecycle transition.
    Failed,
    /// The daemon reported the gateway guest's lifecycle as unknown.
    Unknown,
    /// The daemon's list response did not include the gateway VM.
    #[serde(rename = "not reported by d2bd")]
    NotReported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One degraded scope in `op inspect` output.
pub struct OpInspectDegradedOutputV1 {
    pub scope: String,
    pub reason: String,
    pub remediation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One realm's policy summary, shared by list and inspect output.
pub struct RealmPolicyOutputV1 {
    pub realm: String,
    pub mode: RealmMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_vm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_target: Option<String>,
    pub gateway_state: RealmGatewayState,
    pub cross_realm_policy: String,
    pub credential_boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
/// `d2b status` output: one of the VM, inventory, or bridge-check shapes.
pub enum StatusOutputV2 {
    /// Per-VM status.
    Vm(Box<StatusVmOutputV2>),
    /// Whole-inventory status.
    Inventory(Box<StatusInventoryOutputV2>),
    /// Bridge isolation check status.
    CheckBridges(Box<StatusBridgeCheckOutputV2>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// `d2b status --inventory` output: runtime plus one row per VM.
pub struct StatusInventoryOutputV2 {
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_model: Option<crate::public_wire::PublicReadModelMetadata>,
    pub vms: Vec<StatusVmOutputV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
/// api-ready state of the last VM start in split mode.
pub enum ApiReadyStatusV1 {
    /// A simple closed state.
    Simple(ApiReadySimple),
    /// A terminal error state.
    WithError(ApiReadyErrorV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// The error text of a failed api-ready wait.
pub struct ApiReadyErrorV1 {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
/// Closed api-ready states without an error payload.
pub enum ApiReadySimple {
    /// The API became ready.
    Yes,
    /// The API is still starting.
    Pending,
    /// The wait timed out.
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One VM's status row.
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
/// Live-pool integrity verdict for one VM.
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
/// Legacy per-VM service-state map (V2).
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Runner-parity evidence for one VM.
pub struct RunnerParityOutputV2 {
    pub declared_runner: String,
    pub runner_parity_path: String,
    pub runner_parity_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Bridge isolation check output for one runtime.
pub struct StatusBridgeCheckOutputV2 {
    pub mode: String,
    pub status: String,
    pub message: String,
    pub runtime: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Full `d2b audit` output: host posture plus per-VM sidecar evidence.
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
/// One VM's virtiofsd audit evidence.
pub struct AuditVirtiofsdOutputV2 {
    pub user: String,
    pub caps_dropped: Vec<String>,
    pub readonly_flag: bool,
    pub marker_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// One VM's sshd password-authentication audit evidence.
pub struct AuditSshOutputV2 {
    #[serde(rename = "PasswordAuthentication")]
    pub password_authentication: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// One bridge's isolation audit evidence.
pub struct AuditBridgeIsolationOutputV2 {
    pub bridge: String,
    pub tap: String,
    pub state: String,
    pub isolated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// One VM's gpu/snd sidecar audit evidence.
pub struct AuditSidecarsOutputV2 {
    pub gpu_active: bool,
    pub snd_active: bool,
    pub gpu_user: String,
    pub snd_user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// One environment's usbipd audit evidence.
pub struct AuditUsbipEnvOutputV2 {
    pub socket_active: bool,
    pub backend_active: bool,
    pub lock_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// `d2b auth status` output for the caller.
pub struct AuthStatusOutputV2 {
    pub role: AuthRoleV2,
    pub effective_uid: u32,
    pub sockets: Vec<AuthSocketStatusV2>,
    pub allowed_subcommands: Vec<String>,
    pub denied_subcommands: Vec<AuthDeniedSubcommandV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
/// The caller's authenticated role.
pub enum AuthRoleV2 {
    /// No role is held.
    None,
    /// Launcher scope only.
    Launcher,
    /// Admin scope.
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One admin socket's reachability evidence.
pub struct AuthSocketStatusV2 {
    pub name: String,
    pub path: String,
    pub reachable: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One subcommand denied to the caller, with the refusal reason.
pub struct AuthDeniedSubcommandV2 {
    pub name: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_wire::{VmAutostartMode, VmAutostartPosture};
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
                mode: VmAutostartMode::ManualOnly,
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
    fn audit_output_pins_its_explicit_legacy_renames() {
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

