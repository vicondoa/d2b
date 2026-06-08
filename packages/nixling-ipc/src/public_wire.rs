use crate::{FeatureFlag, Version};
use nixling_core::{error::Error, host::IfName};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "payload")]
pub enum PublicRequest {
    #[serde(rename = "capabilities")]
    Capabilities,
    #[serde(rename = "auth status")]
    AuthStatus,
    #[serde(rename = "list")]
    List(ListRequest),
    #[serde(rename = "status")]
    Status(StatusRequest),
    #[serde(rename = "audit")]
    Audit(AuditRequest),
    #[serde(rename = "host check")]
    HostCheck(HostCheckRequest),
    // Mutating-verb wire surface. Each variant carries the dry-run /
    // apply / json flag tuple + per-verb args. The daemon's
    // `dispatch_request` routes each to a per-verb handler that drives
    // nixlingd → broker. When the per-verb native backend has not yet
    // landed, the daemon returns `MutatingVerb::NotYetImplemented {
    // target_wave, remediation }`; the CLI surfaces the typed envelope
    // and exits 78 (v1.0 daemon-only contract per ADR 0015; the
    // historical bash fallback was retired in v1.0).
    #[serde(rename = "vm start")]
    VmStart(VmLifecycleRequest),
    #[serde(rename = "vm stop")]
    VmStop(VmLifecycleRequest),
    #[serde(rename = "vm restart")]
    VmRestart(VmLifecycleRequest),
    #[serde(rename = "switch")]
    Switch(ActivationRequest),
    #[serde(rename = "boot")]
    Boot(ActivationRequest),
    #[serde(rename = "test")]
    Test(ActivationRequest),
    #[serde(rename = "rollback")]
    Rollback(ActivationRequest),
    #[serde(rename = "gc")]
    Gc(GcRequest),
    #[serde(rename = "keys list")]
    KeysList,
    #[serde(rename = "keys show")]
    KeysShow(KeysShowRequest),
    #[serde(rename = "keys rotate")]
    KeysRotate(KeysRotateRequest),
    #[serde(rename = "trust")]
    Trust(TrustRequest),
    #[serde(rename = "rotate-known-host")]
    RotateKnownHost(RotateKnownHostRequest),
    #[serde(rename = "usb attach")]
    UsbipBind(UsbipBindCliRequest),
    #[serde(rename = "usb detach")]
    UsbipUnbind(UsbipUnbindCliRequest),
    #[serde(rename = "usb probe")]
    UsbipProbe,
    #[serde(rename = "migrate")]
    Migrate(MigrateRequest),
    #[serde(rename = "host prepare")]
    HostPrepare(HostPrepareRequest),
    #[serde(rename = "host destroy")]
    HostDestroy(HostDestroyRequest),
    #[serde(rename = "host install")]
    HostInstall(HostInstallRequest),
    /// Dedicated reconcile verb that re-runs the daemon-side net-route
    /// preflight + the broker-side per-env nftables / route / sysctl
    /// reconcile
    /// without starting any VM. On success it resets the
    /// operator-only-mode counter so future daemon startups are
    /// no longer locked out of autostart. The CLI exposes this as
    /// `nixling host reconcile --network --apply`.
    #[serde(rename = "host reconcile")]
    HostReconcile(HostReconcileRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "payload")]
pub enum PublicResponse {
    #[serde(rename = "capabilities")]
    Capabilities(CapabilitiesResponse),
    #[serde(rename = "auth status")]
    AuthStatus(AuthStatusResponse),
    #[serde(rename = "list")]
    List(ListResponse),
    #[serde(rename = "status")]
    Status(StatusResponse),
    #[serde(rename = "audit")]
    Audit(AuditResponse),
    #[serde(rename = "host check")]
    HostCheck(HostCheckResponse),
    #[serde(rename = "keys list")]
    KeysList(KeysListResponse),
    #[serde(rename = "keys show")]
    KeysShow(KeysShowResponse),
    #[serde(rename = "usb probe")]
    UsbipProbe(UsbipProbeResponse),
    #[serde(rename = "mutating verb")]
    MutatingVerb(MutatingVerbResponse),
    #[serde(rename = "error")]
    Error(Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListRequest {
    pub env: Option<String>,
    pub vm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusRequest {
    #[serde(default)]
    pub check_bridges: bool,
    pub vm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditRequest {
    pub filter: Option<AuditSelector>,
    #[serde(default)]
    pub format: AuditFormat,
    pub since: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckRequest {
    #[serde(default = "default_true")]
    pub read_only: bool,
    #[serde(default)]
    pub strict: bool,
}

// ---------------------------------------------------------------
// Mutating-verb request payloads.
// ---------------------------------------------------------------

/// Common flags every mutating-verb request carries. The daemon
/// rejects requests that set neither `dry_run` nor `apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutationFlags {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub apply: bool,
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmLifecycleRequest {
    pub vm: String,
    #[serde(default, flatten)]
    pub flags: MutationFlags,
    /// When true, exit 0 on process-alive success without waiting for api-ready.
    /// Default false (strict mode: wait for both process-alive and api-ready).
    #[serde(default)]
    pub no_wait_api: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationRequest {
    pub vm: String,
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GcRequest {
    #[serde(default, flatten)]
    pub flags: MutationFlags,
    #[serde(default)]
    pub keep_generations: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeysShowRequest {
    pub vm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeysRotateRequest {
    pub vm: String,
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustRequest {
    pub vm: String,
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RotateKnownHostRequest {
    pub vm: String,
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipBindCliRequest {
    pub vm: String,
    pub bus_id: String,
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipUnbindCliRequest {
    pub vm: String,
    pub bus_id: String,
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrateRequest {
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPrepareRequest {
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostDestroyRequest {
    #[serde(default, flatten)]
    pub flags: MutationFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostInstallRequest {
    #[serde(default, flatten)]
    pub flags: MutationFlags,
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub start: bool,
    #[serde(default)]
    pub no_start: bool,
}

/// `host reconcile` request payload. Today the only scope is
/// `--network`; future versions may add additional scopes (e.g.
/// `--ownership`) carved out of `host prepare`. The daemon rejects
/// requests with no scope selected with a typed `invalid-request`
/// envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostReconcileRequest {
    #[serde(default, flatten)]
    pub flags: MutationFlags,
    /// Re-run the per-env nftables / route / sysctl reconcile and
    /// clear the operator-only-mode counter on success.
    #[serde(default)]
    pub network: bool,
}

/// Mutating-verb daemon response shape.
///
/// `outcome = "dry-run-planned"` returns a human-readable plan
/// description in `summary` (the native CLI's dry-run planner output
/// is preserved verbatim by the daemon).
///
/// `outcome = "applied"` is returned only when the daemon has a
/// native handler that genuinely executed the verb against the
/// broker.
///
/// `outcome = "not-yet-implemented"` is the v1.0 daemon-only
/// contract (ADR 0015): the daemon has the wire variant + handler
/// dispatch row but the per-verb native backend has not yet landed.
/// The CLI surfaces the typed envelope (exit 78) unconditionally;
/// the historical `NIXLING_LEGACY_BASH_OPT_IN` escape hatch and the
/// bash-fallback shim were both retired in v1.0.
///
/// `outcome = "broker-error"` means the daemon reached the live
/// broker executor, but the broker refused or failed the request. The
/// CLI surfaces the redacted broker remediation to the operator with
/// exit 78; the raw broker `message` / `action` details MUST stay on
/// the broker audit + admin-only log surfaces. There is no bash
/// fallback in v1.0 daemon-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutatingVerbResponse {
    pub verb: String,
    pub outcome: MutatingVerbOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_wave: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_ready: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MutatingVerbOutcome {
    DryRunPlanned,
    Applied,
    ApiReadyTimeout,
    NotYetImplemented,
    BrokerError,
    InvalidRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitiesResponse {
    pub broker_socket: String,
    pub capabilities: Vec<FeatureFlag>,
    pub public_socket: String,
    pub server_version: Version,
    pub selected_version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthStatusResponse {
    pub allowed_subcommands: Vec<String>,
    pub denied_subcommands: Vec<DeniedCommandHint>,
    pub role: AuthRole,
    pub sockets: Vec<SocketReachability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListResponse {
    pub vms: Vec<ListEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusResponse {
    pub vm: VmStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditResponse {
    pub entries: Vec<AuditEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckResponse {
    pub exit_code: u8,
    pub findings: Vec<HostFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyEntry {
    pub vm: String,
    pub env: Option<String>,
    pub managed_key_path: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts_entry: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeysListResponse {
    pub entries: Vec<KeyEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeysShowResponse {
    pub vm: String,
    pub env: Option<String>,
    pub managed_key_path: String,
    pub public_key: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts_entry: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProbeStatus {
    Bound,
    Unbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipProbeEntry {
    pub vm: String,
    pub env: String,
    pub bus_id: String,
    pub lock_path: String,
    pub status: UsbipProbeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_vm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipProbeResponse {
    pub entries: Vec<UsbipProbeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditSelector {
    pub env: Option<String>,
    pub severity: Option<String>,
    pub vm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AuditFormat {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthRole {
    None,
    Launcher,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeniedCommandHint {
    pub command: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocketReachability {
    pub reachable: bool,
    pub socket: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListEntry {
    pub env: Option<String>,
    pub lifecycle: VmLifecycle,
    pub runtime: RuntimeSummary,
    pub ssh_user: Option<String>,
    pub vm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmStatus {
    pub bridge_checks: Vec<BridgeCheck>,
    pub env: Option<String>,
    pub lifecycle: VmLifecycle,
    pub runtime: RuntimeSummary,
    pub ssh_user: Option<String>,
    pub static_ip: Option<String>,
    pub vm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeCheck {
    pub bridge: IfName,
    pub present: bool,
    pub tap: Option<IfName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmLifecycle {
    pub pending_restart: bool,
    pub state: VmLifecycleState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum VmLifecycleState {
    Stopped,
    Starting,
    Booted,
    Running,
    Stopping,
    Restarting,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSummary {
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditEntry {
    pub action: String,
    pub result: String,
    pub scope: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostFinding {
    pub check: String,
    pub message: String,
    pub remediation: String,
    pub severity: HostFindingSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum HostFindingSeverity {
    Pass,
    Warn,
    Fail,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{PublicRequest, VmLifecycleState};
    use crate::{decode_frame, encode_frame};

    #[test]
    fn vm_lifecycle_keeps_booted_variant() {
        let encoded = serde_json::to_string(&VmLifecycleState::Booted).expect("serializes");
        assert_eq!(encoded, "\"Booted\"");
    }

    #[test]
    fn status_payload_rejects_unknown_fields() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "status",
            "payload": {
                "vm": "corp-vm",
                "checkBridges": true,
                "extra": true
            }
        }))
        .expect("encodes");
        let error = decode_frame::<PublicRequest>("PublicRequest", &frame)
            .expect_err("unknown field fails");
        assert!(error.message().contains("extra"));
    }
}
