use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::public_wire::{
    AudioChannel, AudioEnforcementPosture, AudioErrorKind, AudioProviderKind, AudioSetApplied,
    LevelPercent,
};

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
    /// a realm. Additive — old CLI consumers must tolerate its absence.
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
    pub canonical_target: String,
    pub identity_source: VmDisplayIdentitySource,
    pub state: String,
    pub operation_id: String,
    pub principal: String,
    pub capability_preflight: VmDisplayCapabilityPreflight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VmDisplayIdentitySource {
    D2bRealmTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmDisplayCapabilityPreflight {
    pub status: VmDisplayCapabilityPreflightStatus,
    pub required_capabilities: Vec<String>,
    pub advertised_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VmDisplayCapabilityPreflightStatus {
    Satisfied,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmDisplayCloseOutputV1 {
    pub command: String,
    pub session_id: String,
    pub closed: bool,
}

#[cfg(test)]
mod display_output_tests {
    use super::*;

    #[test]
    fn display_session_output_carries_trusted_identity_and_preflight_metadata() {
        let output = VmDisplayListOutputV1 {
            command: "vm display list".to_owned(),
            target: Some("demo.work.d2b".to_owned()),
            sessions: vec![VmDisplaySessionOutputV1 {
                session_id: "s0".to_owned(),
                target: "demo.work.d2b".to_owned(),
                canonical_target: "demo.work.d2b".to_owned(),
                identity_source: VmDisplayIdentitySource::D2bRealmTarget,
                state: "running".to_owned(),
                operation_id: "op-1".to_owned(),
                principal: "uid-1000".to_owned(),
                capability_preflight: VmDisplayCapabilityPreflight {
                    status: VmDisplayCapabilityPreflightStatus::Satisfied,
                    required_capabilities: vec!["window-forwarding".to_owned()],
                    advertised_capabilities: vec!["window-forwarding".to_owned()],
                    missing_capabilities: Vec::new(),
                },
            }],
        };

        let json = serde_json::to_value(output).expect("display output serializes");
        let session = &json["sessions"][0];
        assert_eq!(session["canonicalTarget"], "demo.work.d2b");
        assert_eq!(session["identitySource"], "d2b-realm-target");
        assert_eq!(session["capabilityPreflight"]["status"], "satisfied");
        assert_eq!(
            session["capabilityPreflight"]["requiredCapabilities"][0],
            "window-forwarding"
        );
    }
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
    /// Additive — old CLI consumers must tolerate its absence.
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

// ---- Audio CLI output (ADR 0041) --------------------------------------------

/// Output for `d2b audio status --json` (version 1).
///
/// Per-VM entries are sorted by VM name. Per-VM errors are emitted separately
/// so a single misconfigured provider does not suppress all other entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmAudioStatusOutputV1 {
    /// CLI command string for display/logging.
    pub command: String,
    /// Per-VM state for targets that resolved successfully.
    pub entries: Vec<VmAudioStatusEntryOutputV1>,
    /// Per-VM errors for targets that could not be resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<VmAudioErrorOutputV1>,
}

/// Per-VM audio status entry in [`VmAudioStatusOutputV1`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmAudioStatusEntryOutputV1 {
    /// VM name.
    pub vm: String,
    /// Speaker level (0–100), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_level: Option<LevelPercent>,
    /// Whether the speaker is muted.
    pub speaker_muted: bool,
    /// Microphone gain (0–100), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_level: Option<LevelPercent>,
    /// Whether the microphone is muted.
    pub mic_muted: bool,
    /// Provider kind for this VM.
    pub provider_kind: AudioProviderKind,
    /// Enforcement posture for this VM.
    pub enforcement: AudioEnforcementPosture,
}

/// Per-VM error entry in [`VmAudioStatusOutputV1`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmAudioErrorOutputV1 {
    /// VM that failed.
    pub vm: String,
    /// Low-cardinality error kind.
    pub kind: AudioErrorKind,
    /// Optional operator-facing remediation hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

/// Output for `d2b audio set-volume` and `d2b audio mute --json` (version 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmAudioSetOutputV1 {
    /// CLI command string for display/logging.
    pub command: String,
    /// Target VM.
    pub vm: String,
    /// Channel that was changed.
    pub channel: AudioChannel,
    /// Whether and how the change was applied.
    pub applied: AudioSetApplied,
    /// Channel level after the operation (0–100), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<LevelPercent>,
    /// Whether the channel is muted after the operation.
    pub muted: bool,
}

// ---- USB security-key proxy CLI output types (version 1) ----
//
// These types are the stable JSON contract for `d2b usb security-key ...`
// output. They use `deny_unknown_fields` to catch schema drift early.
// The daemon runtime that populates them will ship in a later workstream.

/// Output for `d2b usb security-key status --json` (version 1).
///
/// While the daemon handler is not yet implemented, the CLI emits the
/// `not-yet-implemented` error envelope instead of this type. This type
/// defines the expected shape for when the handler ships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbSkStatusOutputV1 {
    pub command: String,
    pub host_proxy_enabled: bool,
    pub physical_keys: Vec<crate::public_wire::UsbSkPhysicalKeyStatus>,
    pub vm_devices: Vec<crate::public_wire::UsbSkVirtualDeviceStatus>,
    pub lease: crate::public_wire::UsbSkLeaseStatus,
}

/// Output for `d2b usb security-key sessions --json` (version 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbSkSessionsOutputV1 {
    pub command: String,
    pub sessions: Vec<crate::public_wire::UsbSkSession>,
}

/// Output for `d2b usb security-key cancel --dry-run --json` (version 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbSkCancelDryRunOutputV1 {
    pub command: String,
    pub mode: String,
    /// `"current"` or the explicit session ID.
    pub target: String,
    pub planned: Vec<String>,
    pub notes: String,
}

/// Output for `d2b usb security-key test --dry-run --json` (version 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbSkTestDryRunOutputV1 {
    pub command: String,
    pub mode: String,
    pub vm: String,
    pub planned: Vec<String>,
    pub notes: String,
}
