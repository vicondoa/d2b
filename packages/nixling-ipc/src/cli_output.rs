use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecCreateOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub state: crate::guest_wire::ExecState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecListOutputV1 {
    pub command: String,
    pub vm: String,
    pub execs: Vec<VmExecListEntryOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecListEntryOutputV1 {
    pub exec_id: String,
    pub state: crate::guest_wire::ExecState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<u32>,
    pub started_at: String,
    pub start_offset: u64,
    pub end_offset: u64,
    pub stdout_start_offset: u64,
    pub stdout_end_offset: u64,
    pub stderr_start_offset: u64,
    pub stderr_end_offset: u64,
    pub dropped_bytes: u64,
    pub stdout_dropped_bytes: u64,
    pub stderr_dropped_bytes: u64,
    pub truncated: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecStatusOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub state: crate::guest_wire::ExecState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<u32>,
    pub start_offset: u64,
    pub end_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecLogsOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub stdout_base64: String,
    pub stderr_base64: String,
    pub start_offset: u64,
    pub end_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub stdout_start_offset: u64,
    pub stdout_end_offset: u64,
    pub stdout_next_offset: u64,
    pub stdout_eof: bool,
    pub stdout_dropped_bytes: u64,
    pub stdout_truncated: bool,
    pub stderr_start_offset: u64,
    pub stderr_end_offset: u64,
    pub stderr_next_offset: u64,
    pub stderr_eof: bool,
    pub stderr_dropped_bytes: u64,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecKillOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub result: crate::public_wire::ExecDetachedKillOutcome,
    pub state: crate::guest_wire::ExecState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellListOutputV1 {
    pub command: String,
    pub vm: String,
    #[serde(rename = "default_name")]
    pub default_name: String,
    pub sessions: Vec<ShellListSessionOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellListSessionOutputV1 {
    pub name: String,
    pub state: String,
    pub attached: bool,
    #[serde(rename = "is_default")]
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellDetachOutputV1 {
    pub command: String,
    pub vm: String,
    pub name: String,
    pub result: String,
    pub cause: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellKillOutputV1 {
    pub command: String,
    pub vm: String,
    pub name: String,
    pub result: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmDisplayListOutputV1 {
    pub command: String,
    pub target: Option<String>,
    pub sessions: Vec<VmDisplaySessionOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmDisplaySessionOutputV1 {
    pub session_id: String,
    pub target: String,
    pub state: String,
    pub operation_id: String,
    pub principal: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmDisplayCloseOutputV1 {
    pub command: String,
    pub session_id: String,
    pub closed: bool,
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
    pub nixling: String,
    pub microvm: String,
    pub virtiofsd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<String>,
    pub gpu: Option<String>,
    pub video: Option<String>,
    pub snd: Option<String>,
    pub swtpm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusServicesOutputV3 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hypervisor: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub virtiofsd_per_share: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swtpm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel_relay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel_host_bridge: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub usbip_backend_per_env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub usbip_proxy_per_env: BTreeMap<String, String>,
}

impl StatusServicesOutputV3 {
    pub fn from_v2(v2: &StatusServicesOutputV2) -> Self {
        let mut virtiofsd_per_share = BTreeMap::new();
        virtiofsd_per_share.insert("default".to_owned(), v2.virtiofsd.clone());
        Self {
            hypervisor: Some(v2.microvm.clone()),
            virtiofsd_per_share,
            gpu: v2.gpu.clone(),
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
pub struct HostCheckOutputV2 {
    pub mode: String,
    pub strict: bool,
    pub summary: HostCheckSummaryV2,
    pub exit_code: u8,
    pub findings: Vec<HostCheckFindingV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckSummaryV2 {
    pub pass: u32,
    pub warn: u32,
    pub fail: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckFindingV2 {
    pub id: String,
    pub severity: HostCheckSeverityV2,
    pub message: String,
    pub remediation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HostCheckSeverityV2 {
    Pass,
    Warn,
    Fail,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoreVerifyOutputV2 {
    pub vm: String,
    pub status: crate::broker_wire::StoreVerifyStatus,
    pub checked: u32,
    pub drifted: u32,
    pub repaired: u32,
    pub unknown_reason: Option<String>,
    pub audit_ref: Option<String>,
    pub remediation: Option<String>,
}
