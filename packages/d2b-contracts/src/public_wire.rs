use crate::types::MediaRef;
use crate::{FeatureFlag, Version, guest_wire::ExecState};
pub use d2b_core::audio_policy::LevelPercent;
use d2b_core::{
    error::Error,
    host::IfName,
    runtime::{RuntimeOperationCapabilities, RuntimeServiceSummary},
};
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Serialize};
use std::fmt;

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
    // d2bd → broker. When the per-verb native backend has not yet
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
    #[serde(rename = "store verify")]
    StoreVerify(StoreVerifyRequest),
    #[serde(rename = "migrate")]
    Migrate(MigrateRequest),
    #[serde(rename = "host prepare")]
    HostPrepare(HostPrepareRequest),
    #[serde(rename = "host destroy")]
    HostDestroy(HostDestroyRequest),
    #[serde(rename = "host install")]
    HostInstall(HostInstallRequest),
    /// Dedicated reconcile verb that re-runs the broker-side per-env
    /// nftables / route / sysctl reconcile without starting any VM.
    /// The CLI exposes this as `d2b host reconcile --network --apply`.
    #[serde(rename = "host reconcile")]
    HostReconcile(HostReconcileRequest),
    /// Read the editable guest config working copy of `vm` over the
    /// authenticated guest-control bridge and return it as a base64 string.
    /// ADMIN-ONLY (it crosses into the guest over the authenticated
    /// transport): the daemon enforces `PeerRole::Admin` BEFORE any probe /
    /// sign / read. The CLI's `config sync` uses this on guest-control VMs
    /// instead of an SSH transfer.
    #[serde(rename = "read guest config")]
    ReadGuestConfig(ReadGuestConfigRequest),
    /// Multiplexed, ADMIN-ONLY operation on a daemon-held authenticated
    /// guest-control exec session. A single owner connection issues a
    /// `Start` op then drives the session with the remaining ops
    /// (`WriteStdin`/`ReadOutput`/`Signal`/`Resize`/`Wait`/`Close`). The
    /// daemon enforces `PeerRole::Admin` BEFORE any session lookup, vsock
    /// connect, auth, or `ExecCreate`. `d2b vm exec` drives this verb;
    /// it never crosses SSH.
    #[serde(rename = "exec")]
    Exec(ExecOp),
    /// Persistent named guest-shell operation. The staged contract DTOs fail
    /// closed until guestd and d2bd runtime implementations are available.
    #[serde(rename = "shell")]
    Shell(ShellOp),
    /// Console streaming operation (ADR 0041).
    #[serde(rename = "console")]
    Console(ConsoleOp),
    /// Audio policy and status operation (ADR 0041).
    #[serde(rename = "audio")]
    Audio(AudioOp),
    /// Gateway-mode display-session operation. Host-mode daemons reject this
    /// with a typed gateway-unavailable error; gateway-mode d2bd handles it
    /// through the ADR 0032 orchestrator.
    #[serde(rename = "gateway display")]
    GatewayDisplay(GatewayDisplayOp),
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
    #[serde(rename = "store verify")]
    StoreVerify(StoreVerifyResponse),
    #[serde(rename = "mutating verb")]
    MutatingVerb(MutatingVerbResponse),
    #[serde(rename = "read guest config")]
    ReadGuestConfig(ReadGuestConfigResponse),
    #[serde(rename = "exec")]
    Exec(ExecOpResponse),
    #[serde(rename = "shell")]
    Shell(ShellOpResponse),
    #[serde(rename = "console")]
    Console(ConsoleOpResponse),
    #[serde(rename = "audio")]
    Audio(AudioOpResponse),
    #[serde(rename = "gateway display")]
    GatewayDisplay(GatewayDisplayOpResponse),
    #[serde(rename = "error")]
    Error(Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "args", rename_all = "kebab-case")]
pub enum GatewayDisplayOp {
    Start(GatewayDisplayStartArgs),
    Stop(GatewayDisplayStopArgs),
    Open(GatewayDisplayOpenArgs),
    Close(GatewayDisplayCloseArgs),
    List(GatewayDisplayListArgs),
    ListDetailed(GatewayDisplayListArgs),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayStartArgs {
    pub target: String,
    pub operation_id: String,
    pub principal: String,
    pub request_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayStopArgs {
    pub target: String,
    pub operation_id: String,
    pub principal: String,
    pub request_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayOpenArgs {
    pub target: String,
    pub operation_id: String,
    pub principal: String,
    pub app_argv: Vec<String>,
    pub request_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayCloseArgs {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayListArgs {
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "result", rename_all = "kebab-case")]
pub enum GatewayDisplayOpResponse {
    Start(GatewayDisplayStartResult),
    Stop(GatewayDisplayStopResult),
    Open(GatewayDisplayOpenResult),
    Close(GatewayDisplayCloseResult),
    List(GatewayDisplayListResult),
    ListDetailed(GatewayDisplayListDetailedResult),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayStartResult {
    pub target: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayStopResult {
    pub target: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayOpenResult {
    pub session_id: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayCloseResult {
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayListResult {
    pub sessions: Vec<GatewayDisplaySessionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplaySessionSummary {
    pub session_id: String,
    pub target: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplayListDetailedResult {
    pub sessions: Vec<GatewayDisplaySessionDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayDisplaySessionDetail {
    pub session_id: String,
    pub target: String,
    pub state: String,
    pub operation_id: String,
    pub principal: String,
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
    /// Bypass provider graceful-shutdown and use the existing forced cleanup path.
    #[schemars(default)]
    #[serde(default, skip_serializing_if = "is_false")]
    pub force: bool,
    /// When true, exit 0 on process-alive success without waiting for api-ready.
    /// Default false (strict mode: wait for both process-alive and api-ready).
    #[serde(default)]
    pub no_wait_api: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreVerifyRequest {
    pub vm: String,
    #[serde(default)]
    pub repair: bool,
}

pub type StoreVerifyResponse = crate::broker_wire::StoreVerifyResponse;

/// `read guest config` request payload. The daemon resolves the per-VM vsock
/// socket + peer credentials from the trusted bundle; the client supplies only
/// the VM name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadGuestConfigRequest {
    pub vm: String,
}

/// `read guest config` response payload. `contentBase64` is the standard
/// padded base64 of the RAW guest config bytes. The encoded payload is bounded
/// by `READ_GUEST_CONFIG_ENCODED_MAX_BYTES` (derived from
/// `READ_GUEST_FILE_MAX_BYTES`) so it always fits within the public.sock and
/// ttRPC frames. The CLI decodes it and computes size + sha256 from the
/// DECODED bytes — never from any guest-reported value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadGuestConfigResponse {
    pub content_base64: String,
}

/// Maximum decoded stdin chunk per `WriteStdin` op and decoded output chunk
/// per `ReadOutput` op (`DEFAULT_MAX_CHUNK_BYTES`). The base64 envelope of a
/// 64 KiB chunk (~87 KiB) stays well under the 1 MiB public.sock frame, so a
/// single exec op never approaches the frame cap.
pub const EXEC_MAX_CHUNK_BYTES: u64 = crate::guest_wire::DEFAULT_MAX_CHUNK_BYTES;

/// Output stream selector for `ReadOutput`. A closed enum — the daemon never
/// forwards an unspecified stream to the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ExecStream {
    Stdout,
    Stderr,
}

/// A single environment variable for `ExecOp::Start`. Values are forwarded
/// verbatim into the guest exec request and are NEVER logged, traced, or
/// audited (only the count is observable). `Debug` is redacted so a stray
/// `{:?}` can never leak a key or secret value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecEnvVar {
    pub key: String,
    pub value: String,
}

impl fmt::Debug for ExecEnvVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecEnvVar").finish_non_exhaustive()
    }
}

/// Terminal window dimensions for an interactive (`tty`) exec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecTermSize {
    pub rows: u32,
    pub cols: u32,
}

/// `Start` op args. The daemon resolves the per-VM vsock socket + peer
/// credentials from the trusted bundle; the client supplies only the VM name,
/// the command, and the session shape. `detached = false` starts the attached
/// interactive/non-interactive exec FSM; `detached = true` starts a detached
/// exec and returns an opaque exec id for later management verbs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecStartArgs {
    pub vm: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub tty: bool,
    #[serde(default)]
    pub detached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<ExecEnvVar>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_size: Option<ExecTermSize>,
}

impl fmt::Debug for ExecStartArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redaction: never show raw argv, env keys+values, cwd, or other
        // command shape beyond stable counts.
        f.debug_struct("ExecStartArgs")
            .field("argv_len", &self.argv.len())
            .field("env_len", &self.env.as_ref().map_or(0, Vec::len))
            .field("has_cwd", &self.cwd.is_some())
            .finish()
    }
}

/// `WriteStdin` op args. `chunkBase64` is the standard padded base64 of a raw
/// stdin chunk (≤ `EXEC_MAX_CHUNK_BYTES` decoded). `offset` is the client's
/// authoritative stdin byte cursor so a lost reply is idempotently retryable.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWriteStdinArgs {
    pub session: String,
    pub offset: u64,
    pub chunk_base64: String,
    #[serde(default)]
    pub eof: bool,
}

impl fmt::Debug for ExecWriteStdinArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redaction: the session handle + the raw stdin chunk (keystroke
        // bytes) never appear; show only the offset, eof, and encoded length.
        f.debug_struct("ExecWriteStdinArgs")
            .field("session", &"<redacted>")
            .field("offset", &self.offset)
            .field("chunk_base64_len", &self.chunk_base64.len())
            .field("eof", &self.eof)
            .finish()
    }
}

/// `ReadOutput` op args. A bounded long-poll: `wait` + `timeoutMs` let the
/// guest hold the request until data arrives or the (server-side bounded)
/// timeout elapses, so the CLI interleaves short output polls with stdin /
/// signal forwarding without busy-looping.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecReadOutputArgs {
    pub session: String,
    pub stream: ExecStream,
    pub offset: u64,
    pub max_len: u64,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub timeout_ms: u64,
}

impl fmt::Debug for ExecReadOutputArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redaction: the session handle never appears.
        f.debug_struct("ExecReadOutputArgs")
            .field("session", &"<redacted>")
            .field("stream", &self.stream)
            .field("offset", &self.offset)
            .field("max_len", &self.max_len)
            .field("wait", &self.wait)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

/// `Signal` op args. The signal is delivered to the foreground process group
/// of the exec; the CLI maps host SIGINT/SIGTSTP/SIGTERM to
/// the corresponding guest signal numbers. `opId` is a stable client-assigned
/// idempotency token: a retried Signal carries the same `opId` so the worker
/// replays the original ack instead of delivering the signal twice. `opId == 0`
/// means "no dedup" (legacy / unset).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecSignalArgs {
    pub session: String,
    pub signo: u32,
    #[serde(default)]
    pub op_id: u64,
}

impl fmt::Debug for ExecSignalArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecSignalArgs")
            .field("session", &"<redacted>")
            .field("signo", &self.signo)
            .field("op_id", &self.op_id)
            .finish()
    }
}

/// `Resize` op args (SIGWINCH → guest PTY window resize). `opId` is the same
/// stable client-assigned idempotency token as `ExecSignalArgs`: a retried
/// Resize carries the same `opId` so the worker replays the original ack
/// instead of re-delivering the resize. `opId == 0` means "no dedup".
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecResizeArgs {
    pub session: String,
    pub rows: u32,
    pub cols: u32,
    #[serde(default)]
    pub op_id: u64,
}

impl fmt::Debug for ExecResizeArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecResizeArgs")
            .field("session", &"<redacted>")
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .field("op_id", &self.op_id)
            .finish()
    }
}

/// `Wait` op args. A bounded poll for the terminal status; if the command is
/// still running after `timeoutMs` the response reports `running` so the CLI
/// keeps draining output and polling.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWaitArgs {
    pub session: String,
    #[serde(default)]
    pub timeout_ms: u64,
}

impl fmt::Debug for ExecWaitArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecWaitArgs")
            .field("session", &"<redacted>")
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

/// `Close` op args. Half-closes the session's stdin (the daemon issues a
/// guest stdin close, NOT a session teardown): the command keeps running and
/// its output keeps flowing until it exits or the owner disconnects. The
/// result reports `stdinClosed`. Idempotent: closing stdin on a session whose
/// stdin is already closed/torn-down returns success.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCloseArgs {
    pub session: String,
}

impl fmt::Debug for ExecCloseArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecCloseArgs")
            .field("session", &"<redacted>")
            .finish()
    }
}

/// `List` op args for detached execs in one VM.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedListArgs {
    pub vm: String,
}

impl fmt::Debug for ExecDetachedListArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedListArgs")
            .field("vm", &self.vm)
            .finish()
    }
}

/// `Status` op args for one detached exec.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedStatusArgs {
    pub vm: String,
    pub exec_id: String,
}

impl fmt::Debug for ExecDetachedStatusArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedStatusArgs")
            .field("vm", &self.vm)
            .field("exec_id", &self.exec_id)
            .finish()
    }
}

/// `Logs` op args for one detached exec. The daemon fetches retained stdout
/// and stderr bytes, optionally resuming each stream from a caller-provided
/// byte cursor, and returns them in one redacted-debug response.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedLogsArgs {
    pub vm: String,
    pub exec_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<u64>,
}

impl fmt::Debug for ExecDetachedLogsArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedLogsArgs")
            .field("vm", &self.vm)
            .field("exec_id", &self.exec_id)
            .field("has_stdout_offset", &self.stdout_offset.is_some())
            .field("has_stderr_offset", &self.stderr_offset.is_some())
            .field("has_max_len", &self.max_len.is_some())
            .finish()
    }
}

/// `Kill` op args for one detached exec.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedKillArgs {
    pub vm: String,
    pub exec_id: String,
}

impl fmt::Debug for ExecDetachedKillArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedKillArgs")
            .field("vm", &self.vm)
            .field("exec_id", &self.exec_id)
            .finish()
    }
}

/// Multiplexed exec operation. Closed adjacently-tagged enum (`op` + `args`);
/// each variant's args struct is `deny_unknown_fields`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "args", rename_all = "camelCase")]
pub enum ExecOp {
    Start(ExecStartArgs),
    WriteStdin(ExecWriteStdinArgs),
    ReadOutput(ExecReadOutputArgs),
    Signal(ExecSignalArgs),
    Resize(ExecResizeArgs),
    Wait(ExecWaitArgs),
    Close(ExecCloseArgs),
    List(ExecDetachedListArgs),
    Logs(ExecDetachedLogsArgs),
    Status(ExecDetachedStatusArgs),
    Kill(ExecDetachedKillArgs),
}

/// `Start` op result: the daemon-issued opaque session handle plus the initial
/// per-stream read cursors the CLI begins reading from. `tty` echoes the
/// negotiated interactive mode.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecStartResult {
    pub session: String,
    pub tty: bool,
    pub stdout_offset: u64,
    pub stderr_offset: u64,
}

impl fmt::Debug for ExecStartResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redaction: the issued session handle never appears.
        f.debug_struct("ExecStartResult")
            .field("session", &"<redacted>")
            .field("tty", &self.tty)
            .field("stdout_offset", &self.stdout_offset)
            .field("stderr_offset", &self.stderr_offset)
            .finish()
    }
}

/// Detached `Start` op result: an opaque exec id plus initial detached state.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedCreateResult {
    pub exec_id: String,
    pub state: ExecState,
}

impl fmt::Debug for ExecDetachedCreateResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedCreateResult")
            .field("exec_id", &self.exec_id)
            .field("state", &self.state)
            .finish()
    }
}

/// `WriteStdin` op result. `acceptedLen` is the number of bytes that actually
/// landed (partial-write aware); the CLI retries any remainder from
/// `nextOffset`. `backpressured` signals the guest stdin budget is full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWriteStdinResult {
    pub accepted_len: u64,
    pub next_offset: u64,
    #[serde(default)]
    pub backpressured: bool,
    #[serde(default)]
    pub stdin_closed: bool,
}

/// `ReadOutput` op result. `dataBase64` is the base64 of a bounded output
/// chunk; `nextOffset` advances the CLI read cursor; `eof` marks the stream
/// drained after the command went terminal; `timedOut` marks an empty
/// long-poll.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecReadOutputResult {
    pub data_base64: String,
    pub next_offset: u64,
    #[serde(default)]
    pub eof: bool,
    #[serde(default)]
    pub dropped_bytes: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub timed_out: bool,
}

impl fmt::Debug for ExecReadOutputResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redaction: the raw guest stdout/stderr bytes never appear; show
        // only the encoded length + cursor/flags.
        f.debug_struct("ExecReadOutputResult")
            .field("data_base64_len", &self.data_base64.len())
            .field("next_offset", &self.next_offset)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
}

/// `Signal` / `Resize` op result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecControlResult {
    #[serde(default)]
    pub delivered: bool,
}

/// Terminal disposition of the guest command. `Exited` carries the WIFEXITED
/// code (0–255); `Signaled` carries the terminating signal number; `Error`
/// carries a closed-enum guest error slug for an abnormal terminal state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ExecTerminalStatus {
    Exited { code: i32 },
    Signaled { signal: u32 },
    Error { slug: String },
}

/// `Wait` op result. Exactly one of `terminalStatus` (terminal) or
/// `running == true` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWaitResult {
    #[serde(default)]
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_status: Option<ExecTerminalStatus>,
}

/// `Close` op result. `stdinClosed` confirms the session's stdin is now
/// half-closed; the command continues running until it exits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCloseResult {
    #[serde(default)]
    pub stdin_closed: bool,
}

/// One detached exec summary for `List`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedListEntry {
    pub exec_id: String,
    pub state: ExecState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<u32>,
    pub started_at: String,
    #[serde(default)]
    pub start_offset: u64,
    #[serde(default)]
    pub end_offset: u64,
    #[serde(default)]
    pub stdout_start_offset: u64,
    #[serde(default)]
    pub stdout_end_offset: u64,
    #[serde(default)]
    pub stderr_start_offset: u64,
    #[serde(default)]
    pub stderr_end_offset: u64,
    pub dropped_bytes: u64,
    #[serde(default)]
    pub stdout_dropped_bytes: u64,
    #[serde(default)]
    pub stderr_dropped_bytes: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub stdout_truncated: bool,
    #[serde(default)]
    pub stderr_truncated: bool,
}

impl fmt::Debug for ExecDetachedListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedListEntry")
            .field("exec_id", &self.exec_id)
            .field("state", &self.state)
            .field("exit_code", &self.exit_code)
            .field("signal", &self.signal)
            .field("started_at", &self.started_at)
            .field("start_offset", &self.start_offset)
            .field("end_offset", &self.end_offset)
            .field("stdout_start_offset", &self.stdout_start_offset)
            .field("stdout_end_offset", &self.stdout_end_offset)
            .field("stderr_start_offset", &self.stderr_start_offset)
            .field("stderr_end_offset", &self.stderr_end_offset)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("stdout_dropped_bytes", &self.stdout_dropped_bytes)
            .field("stderr_dropped_bytes", &self.stderr_dropped_bytes)
            .field("truncated", &self.truncated)
            .field("stdout_truncated", &self.stdout_truncated)
            .field("stderr_truncated", &self.stderr_truncated)
            .finish()
    }
}

/// Detached `List` op result.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedListResult {
    pub execs: Vec<ExecDetachedListEntry>,
}

impl fmt::Debug for ExecDetachedListResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedListResult")
            .field("exec_count", &self.execs.len())
            .finish()
    }
}

/// Detached `Status` op result.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedStatusResult {
    pub exec_id: String,
    pub state: ExecState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<u32>,
    pub start_offset: u64,
    pub end_offset: u64,
    pub dropped_bytes: u64,
    #[serde(default)]
    pub truncated: bool,
}

impl fmt::Debug for ExecDetachedStatusResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedStatusResult")
            .field("exec_id", &self.exec_id)
            .field("state", &self.state)
            .field("has_reason", &self.reason.is_some())
            .field("exit_code", &self.exit_code)
            .field("signal", &self.signal)
            .field("start_offset", &self.start_offset)
            .field("end_offset", &self.end_offset)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .finish()
    }
}

/// Detached `Logs` op result. The base64 payloads carry raw guest output and
/// are redacted from `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedLogsResult {
    pub exec_id: String,
    pub stdout_base64: String,
    pub stderr_base64: String,
    pub start_offset: u64,
    pub end_offset: u64,
    pub dropped_bytes: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub stdout_start_offset: u64,
    #[serde(default)]
    pub stdout_end_offset: u64,
    #[serde(default)]
    pub stdout_next_offset: u64,
    #[serde(default)]
    pub stdout_eof: bool,
    #[serde(default)]
    pub stdout_dropped_bytes: u64,
    #[serde(default)]
    pub stdout_truncated: bool,
    #[serde(default)]
    pub stderr_start_offset: u64,
    #[serde(default)]
    pub stderr_end_offset: u64,
    #[serde(default)]
    pub stderr_next_offset: u64,
    #[serde(default)]
    pub stderr_eof: bool,
    #[serde(default)]
    pub stderr_dropped_bytes: u64,
    #[serde(default)]
    pub stderr_truncated: bool,
}

impl fmt::Debug for ExecDetachedLogsResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedLogsResult")
            .field(
                "data_len",
                &(self.stdout_base64.len() + self.stderr_base64.len()),
            )
            .field("dropped", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .finish()
    }
}

/// Closed result for idempotent detached `Kill`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecDetachedKillOutcome {
    Cancelling,
    AlreadyTerminal,
}

/// Detached `Kill` op result.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecDetachedKillResult {
    pub exec_id: String,
    pub result: ExecDetachedKillOutcome,
    pub state: ExecState,
}

impl fmt::Debug for ExecDetachedKillResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecDetachedKillResult")
            .field("exec_id", &self.exec_id)
            .field("result", &self.result)
            .field("state", &self.state)
            .finish()
    }
}

/// Multiplexed exec operation result. Closed adjacently-tagged enum mirroring
/// [`ExecOp`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "result", rename_all = "camelCase")]
pub enum ExecOpResponse {
    Start(ExecStartResult),
    DetachedCreate(ExecDetachedCreateResult),
    WriteStdin(ExecWriteStdinResult),
    ReadOutput(ExecReadOutputResult),
    Signal(ExecControlResult),
    Resize(ExecControlResult),
    Wait(ExecWaitResult),
    Close(ExecCloseResult),
    List(ExecDetachedListResult),
    Logs(ExecDetachedLogsResult),
    Status(ExecDetachedStatusResult),
    Kill(ExecDetachedKillResult),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ShellName(String);

impl ShellName {
    pub fn new(value: impl Into<String>) -> Result<Self, ShellNameError> {
        let value = value.into();
        if shell_name_valid(&value) {
            Ok(Self(value))
        } else {
            Err(ShellNameError)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellNameError;

impl fmt::Debug for ShellName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShellName(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for ShellName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| {
            serde::de::Error::custom("shell name must match ^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$")
        })
    }
}

impl JsonSchema for ShellName {
    fn schema_name() -> String {
        "ShellName".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let mut object = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                min_length: Some(1),
                max_length: Some(64),
                pattern: Some("^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$".to_owned()),
            })),
            ..Default::default()
        };
        object.metadata = Some(Box::new(Metadata {
            description: Some("Persistent shell session name.".to_owned()),
            ..Default::default()
        }));
        Schema::Object(object)
    }
}

fn shell_name_valid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    (bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellAttachArgs {
    pub vm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ShellName>,
    #[serde(default)]
    pub force: bool,
    pub initial_terminal_size: crate::terminal_wire::TerminalSize,
}

impl fmt::Debug for ShellAttachArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellAttachArgs")
            .field("vm", &self.vm)
            .field("has_name", &self.name.is_some())
            .field("force", &self.force)
            .field("initial_terminal_size", &self.initial_terminal_size)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellListArgs {
    pub vm: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellDetachArgs {
    pub vm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ShellName>,
}

impl fmt::Debug for ShellDetachArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellDetachArgs")
            .field("vm", &self.vm)
            .field("has_name", &self.name.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellKillArgs {
    pub vm: String,
    pub name: ShellName,
}

impl fmt::Debug for ShellKillArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellKillArgs")
            .field("vm", &self.vm)
            .field("name", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellCloseAttachArgs {
    pub session: String,
}

impl fmt::Debug for ShellCloseAttachArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellCloseAttachArgs")
            .field("session", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "args", rename_all = "camelCase")]
pub enum ShellOp {
    Attach(ShellAttachArgs),
    WriteStdin(crate::terminal_wire::TerminalWriteStdin),
    ReadOutput(crate::terminal_wire::TerminalReadOutput),
    Resize(crate::terminal_wire::TerminalResize),
    Wait(crate::terminal_wire::TerminalWait),
    CloseStdin(crate::terminal_wire::TerminalClose),
    CloseAttach(ShellCloseAttachArgs),
    List(ShellListArgs),
    Detach(ShellDetachArgs),
    Kill(ShellKillArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ShellSessionState {
    Attached,
    Detached,
    Killed,
    PoolUnavailable,
    FeatureDisabled,
    OutputGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ShellCloseCause {
    ClientDetach,
    EvictedByForce,
    EvictedByAdminDetach,
    KilledByAdmin,
    PoolUnavailable,
    OutputGap,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellAttachResult {
    pub session: String,
    pub resolved_name: ShellName,
    pub state: ShellSessionState,
    #[serde(default)]
    pub force_evicted: bool,
}

impl fmt::Debug for ShellAttachResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellAttachResult")
            .field("session", &"<redacted>")
            .field("resolved_name", &"<redacted>")
            .field("state", &self.state)
            .field("force_evicted", &self.force_evicted)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellListEntry {
    pub name: ShellName,
    pub state: ShellSessionState,
    #[serde(default)]
    pub attached: bool,
    #[serde(default)]
    pub is_default: bool,
}

impl fmt::Debug for ShellListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellListEntry")
            .field("name", &"<redacted>")
            .field("state", &self.state)
            .field("attached", &self.attached)
            .field("is_default", &self.is_default)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellListResult {
    pub default_name: ShellName,
    pub sessions: Vec<ShellListEntry>,
}

impl fmt::Debug for ShellListResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellListResult")
            .field("default_name", &"<redacted>")
            .field("sessions_len", &self.sessions.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellDetachResult {
    pub resolved_name: ShellName,
    pub detached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<ShellCloseCause>,
}

impl fmt::Debug for ShellDetachResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellDetachResult")
            .field("resolved_name", &"<redacted>")
            .field("detached", &self.detached)
            .field("cause", &self.cause)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellKillResult {
    pub name: ShellName,
    pub killed: bool,
    pub state: ShellSessionState,
}

impl fmt::Debug for ShellKillResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellKillResult")
            .field("name", &"<redacted>")
            .field("killed", &self.killed)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "result", rename_all = "camelCase")]
pub enum ShellOpResponse {
    Attach(ShellAttachResult),
    WriteStdin(crate::terminal_wire::TerminalWriteStdinResult),
    ReadOutput(crate::terminal_wire::TerminalReadOutputChunk),
    Resize(crate::terminal_wire::TerminalControlResult),
    Wait(crate::terminal_wire::TerminalWaitResult),
    CloseStdin(crate::terminal_wire::TerminalCloseResult),
    CloseAttach(ShellDetachResult),
    List(ShellListResult),
    Detach(ShellDetachResult),
    Kill(ShellKillResult),
}

// ---- Console operation (ADR 0041) -------------------------------------------

/// Which runtime provider handles the console for a given target VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ConsoleProviderKind {
    /// Cloud Hypervisor NixOS VM with a local hypervisor console backend and
    /// daemon/broker drainer.
    LocalHypervisor,
    /// qemu-media VM with a broker-owned fd-backed console backend.
    QemuMedia,
    /// ACA sandbox with a guestd-compatible agent over the provider peer
    /// transport (no local socket or broker fd involved).
    AcaSandbox,
}

/// Attach to a console session for a VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleAttachArgs {
    /// VM whose console is requested.
    pub vm: String,
    /// Initial terminal geometry.
    pub initial_terminal_size: crate::terminal_wire::TerminalSize,
}

/// Write stdin bytes to an active console session.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleWriteStdinArgs {
    /// Opaque console session handle (never logged or audited).
    pub session: String,
    /// Input byte offset for flow-control ordering.
    pub offset: u64,
    /// Base64-encoded input bytes.
    pub chunk_base64: String,
    /// Whether this write marks end-of-stdin.
    #[serde(default)]
    pub eof: bool,
}

impl fmt::Debug for ConsoleWriteStdinArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleWriteStdinArgs")
            .field("session", &"<redacted>")
            .field("offset", &self.offset)
            .field("chunk_base64_len", &self.chunk_base64.len())
            .field("eof", &self.eof)
            .finish()
    }
}

/// Read console output from a ring-buffer–backed session.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleReadOutputArgs {
    /// Opaque console session handle.
    pub session: String,
    /// Stream to read (`stdout` is the primary console stream; providers that
    /// expose a separate stderr channel report it here).
    pub stream: crate::terminal_wire::TerminalStream,
    /// Ring-buffer read offset. Compare against the `ring_buffer_start_offset`
    /// from the previous response to detect dropped output.
    pub offset: u64,
    /// Maximum output bytes to return in this response.
    pub max_len: u64,
    /// Compatibility flag retained on the wire. Console output reads are
    /// non-blocking; clients should back off between empty responses.
    #[serde(default)]
    pub wait: bool,
    /// Compatibility timeout retained on the wire. The daemon does not block
    /// on console output reads.
    #[serde(default)]
    pub timeout_ms: u64,
}

impl fmt::Debug for ConsoleReadOutputArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleReadOutputArgs")
            .field("session", &"<redacted>")
            .field("stream", &self.stream)
            .field("offset", &self.offset)
            .field("max_len", &self.max_len)
            .field("wait", &self.wait)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

/// Resize the terminal associated with a console session.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleResizeArgs {
    /// Opaque console session handle.
    pub session: String,
    /// New terminal geometry.
    pub size: crate::terminal_wire::TerminalSize,
}

impl fmt::Debug for ConsoleResizeArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleResizeArgs")
            .field("session", &"<redacted>")
            .field("size", &self.size)
            .finish()
    }
}

/// Non-blocking check for whether the console session has exited.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleWaitArgs {
    /// Opaque console session handle.
    pub session: String,
    /// Compatibility timeout retained on the wire. The daemon returns the
    /// current EOF state immediately; clients should back off between polls.
    #[serde(default)]
    pub timeout_ms: u64,
}

impl fmt::Debug for ConsoleWaitArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleWaitArgs")
            .field("session", &"<redacted>")
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

/// Close a console session, detaching the client without stopping the VM.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleCloseArgs {
    /// Opaque console session handle.
    pub session: String,
}

impl fmt::Debug for ConsoleCloseArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleCloseArgs")
            .field("session", &"<redacted>")
            .finish()
    }
}

/// Console operation sub-request dispatched inside [`PublicRequest::Console`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "args", rename_all = "camelCase")]
pub enum ConsoleOp {
    Attach(ConsoleAttachArgs),
    WriteStdin(ConsoleWriteStdinArgs),
    ReadOutput(ConsoleReadOutputArgs),
    Resize(ConsoleResizeArgs),
    Wait(ConsoleWaitArgs),
    Close(ConsoleCloseArgs),
}

/// Result of a successful console attach.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleAttachResult {
    /// Opaque session handle for subsequent ops (never logged or audited).
    pub session: String,
    /// Which provider handled the attach.
    pub provider_kind: ConsoleProviderKind,
    /// Ring-buffer start offset at attach time. Clients compare the
    /// `ring_buffer_start_offset` returned by subsequent `ReadOutput` calls
    /// against this value to detect dropped output.
    pub ring_buffer_start_offset: u64,
}

impl fmt::Debug for ConsoleAttachResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleAttachResult")
            .field("session", &"<redacted>")
            .field("provider_kind", &self.provider_kind)
            .field("ring_buffer_start_offset", &self.ring_buffer_start_offset)
            .finish()
    }
}

/// A chunk of console output from the ring buffer.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleReadOutputResult {
    /// Opaque session handle.
    pub session: String,
    /// Which output stream this chunk belongs to.
    pub stream: crate::terminal_wire::TerminalStream,
    /// Absolute offset of the first byte of `chunk_base64` in the logical
    /// output stream (includes any bytes already dropped from the ring buffer).
    pub offset: u64,
    /// Base64-encoded output bytes.
    pub chunk_base64: String,
    /// Current ring-buffer start offset. If this exceeds
    /// `offset + decoded_len(chunk_base64)`, bytes were dropped and the
    /// client should fast-forward.
    pub ring_buffer_start_offset: u64,
    /// Total bytes dropped from the ring buffer since VM start.
    pub dropped_bytes: u64,
    /// Whether the output stream has ended (VM exited or session closed).
    /// Clients must still consume any non-empty `chunk_base64` in this response
    /// and continue polling until `is_eof` is true with an empty chunk.
    pub is_eof: bool,
}

impl fmt::Debug for ConsoleReadOutputResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleReadOutputResult")
            .field("session", &"<redacted>")
            .field("stream", &self.stream)
            .field("offset", &self.offset)
            .field("chunk_base64_len", &self.chunk_base64.len())
            .field("ring_buffer_start_offset", &self.ring_buffer_start_offset)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("is_eof", &self.is_eof)
            .finish()
    }
}

/// Generic control acknowledgement for console write and resize operations.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleControlResult {
    /// Opaque session handle.
    pub session: String,
    /// Whether the control operation was applied.
    pub ok: bool,
}

impl fmt::Debug for ConsoleControlResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleControlResult")
            .field("session", &"<redacted>")
            .field("ok", &self.ok)
            .finish()
    }
}

/// Result of waiting for the console session to end.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleWaitResult {
    /// Opaque session handle.
    pub session: String,
    /// True if the session ended; false if the timeout elapsed before exit.
    pub exited: bool,
}

impl fmt::Debug for ConsoleWaitResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleWaitResult")
            .field("session", &"<redacted>")
            .field("exited", &self.exited)
            .finish()
    }
}

/// Result of closing a console session.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleCloseResult {
    /// Opaque session handle.
    pub session: String,
    /// True if the session was found and closed; false if it was already gone.
    pub closed: bool,
}

impl fmt::Debug for ConsoleCloseResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleCloseResult")
            .field("session", &"<redacted>")
            .field("closed", &self.closed)
            .finish()
    }
}

/// Console operation response dispatched inside [`PublicResponse::Console`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "result", rename_all = "camelCase")]
pub enum ConsoleOpResponse {
    Attach(ConsoleAttachResult),
    WriteStdin(ConsoleControlResult),
    ReadOutput(ConsoleReadOutputResult),
    Resize(ConsoleControlResult),
    Wait(ConsoleWaitResult),
    Close(ConsoleCloseResult),
}

// ---- Audio operation (ADR 0041) ---------------------------------------------

/// An audio channel controlled by a single operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioChannel {
    /// Speaker / playback output volume.
    Speaker,
    /// Microphone / capture gain.
    Microphone,
}

/// How audio enforcement is applied for a target VM (ADR 0041).
///
/// This enum describes only *successful* enforcement outcomes. Provider
/// misconfiguration is not a successful outcome and is reported exclusively
/// through [`AudioVmError`] with `kind = AudioErrorKind::ProviderMisconfigured`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioEnforcementPosture {
    /// Host-side PipeWire policy and guest-side guestd policy both applied.
    HostAndGuest,
    /// Host-side enforcement only; guestd is absent or the provider does not
    /// support guest enforcement.
    HostOnly,
    /// Guest-side guestd enforcement only; no local PipeWire node (e.g.
    /// ACA sandbox with a guestd-compatible agent).
    GuestOnly,
    /// Neither host nor guest enforcement is supported for this target.
    Unsupported,
}

/// Low-cardinality error kind for a per-VM audio failure.
///
/// Used in [`AudioVmError`] and [`VmAudioErrorOutputV1`](crate::VmAudioErrorOutputV1)
/// so that both the wire protocol and the generated CLI schema carry enum
/// constraints rather than a free-form string.
///
/// Serde names match the canonical wire strings exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioErrorKind {
    /// The provider is expected to expose a guestd-compatible agent but none
    /// was found; operator remediation is required.
    ProviderMisconfigured,
    /// The requested VM was not found in the bundle.
    VmNotFound,
    /// Audio enforcement is not available for this VM (e.g. the runtime does
    /// not support it and no degraded path exists).
    EnforcementUnavailable,
    /// The VM exists but audio is not enabled in its manifest entry.
    AudioNotEnabled,
    /// An unexpected I/O or deserialization error occurred while reading or
    /// writing audio state. Operator should check daemon logs.
    InternalError,
}

/// Which runtime provider handles audio for a given target VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioProviderKind {
    /// Cloud Hypervisor NixOS VM with vhost-user-sound + PipeWire.
    LocalHypervisor,
    /// qemu-media VM with a declared qemu audio backend.
    QemuMedia,
    /// ACA sandbox with a guestd-compatible agent for audio policy.
    AcaSandbox,
}

/// Request audio status for one or more target VMs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioStatusArgs {
    /// VMs to query. An empty list queries all accessible VMs.
    #[serde(default)]
    pub vms: Vec<String>,
}

/// Set the volume or gain level for one channel of one VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioSetVolumeArgs {
    /// Target VM.
    pub vm: String,
    /// Which audio channel to adjust.
    pub channel: AudioChannel,
    /// New level (0–100 inclusive). Validated at the wire boundary.
    pub level: LevelPercent,
}

/// Mute or unmute one channel of one VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioMuteArgs {
    /// Target VM.
    pub vm: String,
    /// Which audio channel to mute/unmute.
    pub channel: AudioChannel,
    /// True to mute, false to unmute.
    pub mute: bool,
}

/// Audio operation sub-request dispatched inside [`PublicRequest::Audio`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "args", rename_all = "camelCase")]
pub enum AudioOp {
    Status(AudioStatusArgs),
    SetVolume(AudioSetVolumeArgs),
    Mute(AudioMuteArgs),
}

/// Per-channel audio state for one VM target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioChannelState {
    /// Current volume/gain level in percent. `None` when the level is unknown
    /// (e.g. the provider has not yet synced state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<LevelPercent>,
    /// Whether the channel is currently muted.
    pub muted: bool,
}

/// Full audio status for one VM target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioVmState {
    /// Target VM name.
    pub vm: String,
    /// Speaker / playback channel state.
    pub speaker: AudioChannelState,
    /// Microphone / capture channel state.
    pub microphone: AudioChannelState,
    /// Which runtime provider handles audio for this VM.
    pub provider_kind: AudioProviderKind,
    /// Current enforcement posture.
    pub enforcement: AudioEnforcementPosture,
}

/// A per-VM error in a multi-target audio status response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioVmError {
    /// VM that failed.
    pub vm: String,
    /// Low-cardinality error kind. Never contains provider-internal details
    /// or credential fragments.
    pub kind: AudioErrorKind,
    /// Optional operator-facing remediation hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

/// Multi-target audio status result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatusResult {
    /// Per-VM state for targets that resolved successfully.
    pub entries: Vec<AudioVmState>,
    /// Per-VM errors for targets that could not be resolved. One
    /// misconfigured target does not fail the entire multi-target query.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<AudioVmError>,
}

/// Whether a set-volume or mute operation was applied and through which path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioSetApplied {
    /// Applied to both host (PipeWire/qemu) and guest (guestd).
    HostAndGuest,
    /// Applied to host only; guestd enforcement was unavailable or degraded.
    HostOnly,
    /// Applied to guest only (ACA sandbox; no local host audio state written).
    GuestOnly,
    /// No enforcement path was available; the operation was not applied.
    Unsupported,
}

/// Result of a set-volume or mute operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioSetResult {
    /// Target VM.
    pub vm: String,
    /// Channel that was changed.
    pub channel: AudioChannel,
    /// Whether and how the change was applied.
    pub applied: AudioSetApplied,
    /// Channel state after the operation.
    pub state: AudioChannelState,
}

/// Audio operation response dispatched inside [`PublicResponse::Audio`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "result", rename_all = "camelCase")]
pub enum AudioOpResponse {
    Status(AudioStatusResult),
    SetVolume(AudioSetResult),
    Mute(AudioSetResult),
}

// ---- Remaining request structs -----------------------------------------------

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
    /// Re-run the per-env nftables / route / sysctl reconcile.
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
/// the historical `D2B_LEGACY_BASH_OPT_IN` escape hatch and the
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_model: Option<PublicReadModelMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusResponse {
    pub entries: Vec<VmStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_model: Option<PublicReadModelMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicReadModelMetadata {
    pub schema_version: u32,
    pub kind: String,
    pub generation: u64,
    pub source_fingerprint: String,
    pub updated_at_unix_ms: u128,
    pub freshness: String,
    pub deep_refresh: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProbeStatus {
    Bound,
    #[default]
    Unbound,
    Degraded,
    Enrollable,
    Enrolled,
    Stale,
    DirectConfig,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
/// Wire-compatible state name for the USBIP host-session claim. The backing
/// lock lives under `/run/d2b/locks/usbip`, so the claim survives VM
/// stop/restart and daemon restart during one host boot, but not host reboot.
pub enum UsbipDurableClaimState {
    #[default]
    Missing,
    HeldByDesiredOwner,
    HeldByOtherOwner,
    StaleOwner,
    Corrupt,
    NotApplicable,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Wire-compatible status object for the USBIP host-session claim. The JSON
/// field is still named `durableClaim` for compatibility; treat it as durable
/// only within the current host boot/session.
pub struct UsbipDurableClaimStatus {
    pub state: UsbipDurableClaimState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_vm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipHostBindState {
    Unbound,
    BoundToUsbipHost,
    BoundToUnexpectedDriver,
    DeviceMissing,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipHostCarrierState {
    Absent,
    Unavailable,
    WithheldForOwner,
    Ready,
    DepartedDuringProbe,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProxyState {
    NotDeclared,
    Stopped,
    Starting,
    Listening,
    Stale,
    Failed,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipHostProbeStatus {
    pub bind: UsbipHostBindState,
    pub carrier: UsbipHostCarrierState,
    pub proxy: UsbipProxyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipGuestImportState {
    Detached,
    Imported,
    Unavailable,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipGuestProbeStatus {
    pub import: UsbipGuestImportState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipTopologyState {
    Match,
    Mismatch,
    Incomplete,
    NotObserved,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipPolicyState {
    Allowed,
    Denied,
    Missing,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipTopologyPolicyStatus {
    pub topology: UsbipTopologyState,
    pub policy: UsbipPolicyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProbeDegradedReasonCode {
    PolicyFailed,
    DeviceDepartedBeforeClaim,
    DeviceDepartedAfterLock,
    DeviceDepartedDuringMutation,
    DeviceReappearedWithDifferentTopology,
    LockHeldByOtherOwner,
    InvalidPersistedLockClaim,
    CarrierUnavailable,
    HostBindUnavailable,
    ProxyUnavailable,
    GuestImportUnavailable,
    StaleHostState,
    StaleGuestState,
    ProbeIncomplete,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipProbeDegradedReason {
    pub code: UsbipProbeDegradedReasonCode,
    pub summary: String,
    pub remediation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipVmStatus {
    pub degraded: bool,
    pub entries: Vec<UsbipProbeEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UsbProbeEntryKind {
    #[default]
    Usbip,
    QemuMediaSlot,
}

fn is_default_usb_probe_entry_kind(kind: &UsbProbeEntryKind) -> bool {
    matches!(kind, UsbProbeEntryKind::Usbip)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipProbeEntry {
    #[serde(default, skip_serializing_if = "is_default_usb_probe_entry_kind")]
    pub kind: UsbProbeEntryKind,
    pub vm: String,
    pub env: String,
    pub bus_id: String,
    pub lock_path: String,
    pub status: UsbipProbeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_vm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_ref: Option<MediaRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_bus_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up_command: Option<String>,
    #[serde(default)]
    /// USBIP host-session claim status. Serialized as `durableClaim` for
    /// compatibility with existing clients.
    pub durable_claim: UsbipDurableClaimStatus,
    #[serde(default)]
    pub host: UsbipHostProbeStatus,
    #[serde(default)]
    pub guest: UsbipGuestProbeStatus,
    #[serde(default)]
    pub topology_policy: UsbipTopologyPolicyStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_reasons: Vec<UsbipProbeDegradedReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediation_commands: Vec<String>,
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
    pub graphics: bool,
    pub is_net_vm: bool,
    pub lifecycle: VmLifecycle,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_closure_out_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autostart: Option<VmAutostartPosture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<QemuMediaStatus>,
    pub runtime: RuntimeSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    pub services: PublicVmServices,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_capabilities: Vec<String>,
    pub ssh_user: Option<String>,
    pub static_ip: Option<String>,
    pub tpm: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_capabilities: Vec<String>,
    pub usbip: bool,
    pub vm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmStatus {
    pub bridge_checks: Vec<BridgeCheck>,
    pub env: Option<String>,
    pub graphics: bool,
    pub is_net_vm: bool,
    pub lifecycle: VmLifecycle,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autostart: Option<VmAutostartPosture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<QemuMediaStatus>,
    pub runtime: RuntimeSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    pub services: PublicVmServices,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_capabilities: Vec<String>,
    pub ssh_user: Option<String>,
    pub static_ip: Option<String>,
    pub tpm: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_capabilities: Vec<String>,
    pub usbip: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb: Option<UsbipVmStatus>,
    pub vm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicVmServices {
    pub gpu: Option<String>,
    pub microvm: String,
    pub d2b: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<String>,
    pub snd: Option<String>,
    pub swtpm: Option<String>,
    pub video: Option<String>,
    pub virtiofsd: String,
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
    #[serde(default)]
    pub degraded: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_reasons: Vec<VmLifecycleDegradedReason>,
    pub pending_restart: bool,
    pub state: VmLifecycleState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmLifecycleDegradedReason {
    pub reason: String,
    pub remediation: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "RuntimeOperationCapabilities::is_empty"
    )]
    pub operation_capabilities: RuntimeOperationCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<RuntimeServiceSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmAutostartPosture {
    pub mode: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaStatus {
    pub firmware_mode: String,
    pub media: Vec<QemuMediaSourceStatus>,
    pub runner: QemuMediaRunnerStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaRunnerStatus {
    pub pre_cont_progress: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qmp_readiness: Option<String>,
    pub role: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaSourceStatus {
    pub format: String,
    pub media_ref: String,
    pub read_only: bool,
    pub registry: QemuMediaRegistryStatus,
    pub slot: String,
    pub source_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaRegistryStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    pub state: String,
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
    use super::{
        LevelPercent, MutationFlags, PublicRequest, RuntimeSummary, VmLifecycleRequest,
        VmLifecycleState,
    };
    use crate::{decode_frame, encode_frame};
    use d2b_core::{
        processes::ProcessRole,
        runtime::{RuntimeOperationCapabilities, RuntimeServiceRole, RuntimeServiceSummary},
    };

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

    #[test]
    fn vm_lifecycle_force_defaults_false_for_compatibility() {
        let decoded: PublicRequest = serde_json::from_value(serde_json::json!({
            "kind": "vm stop",
            "payload": {
                "vm": "corp-vm",
                "apply": true
            }
        }))
        .expect("old vm stop payload decodes");

        assert!(matches!(
            decoded,
            PublicRequest::VmStop(VmLifecycleRequest {
                vm,
                flags: MutationFlags { apply: true, .. },
                force: false,
                no_wait_api: false,
            }) if vm == "corp-vm"
        ));
    }

    #[test]
    fn vm_lifecycle_omits_false_force_but_serializes_true() {
        let without_force = serde_json::to_value(PublicRequest::VmStop(VmLifecycleRequest {
            vm: "corp-vm".to_owned(),
            flags: MutationFlags::default(),
            force: false,
            no_wait_api: false,
        }))
        .expect("vm stop serializes");
        assert!(without_force["payload"].get("force").is_none());

        let with_force = serde_json::to_value(PublicRequest::VmRestart(VmLifecycleRequest {
            vm: "corp-vm".to_owned(),
            flags: MutationFlags::default(),
            force: true,
            no_wait_api: false,
        }))
        .expect("vm restart serializes");
        assert_eq!(with_force["payload"]["force"], true);
    }

    #[test]
    fn runtime_summary_omits_default_runtime_seam_fields() {
        let summary = RuntimeSummary {
            detail: "running".to_owned(),
            kind: Some("nixos".to_owned()),
            operation_capabilities: RuntimeOperationCapabilities::default(),
            services: Vec::new(),
        };

        let value = serde_json::to_value(summary).expect("serializes");
        assert_eq!(
            value.get("detail").and_then(|v| v.as_str()),
            Some("running")
        );
        assert!(value.get("operationCapabilities").is_none());
        assert!(value.get("services").is_none());
    }

    #[test]
    fn runtime_summary_serializes_positive_capabilities_and_services() {
        let summary = RuntimeSummary {
            detail: "qemu media runner active".to_owned(),
            kind: Some("qemu-media".to_owned()),
            operation_capabilities: RuntimeOperationCapabilities::local_qemu_media(),
            services: vec![RuntimeServiceSummary::from_process_role(
                "qemu-media",
                ProcessRole::QemuMediaRunner,
                false,
            )],
        };

        let value = serde_json::to_value(summary).expect("serializes");
        assert_eq!(
            value.pointer("/operationCapabilities/media/qemuMedia"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            value.pointer("/services/0/role"),
            Some(&serde_json::json!("hypervisor"))
        );
        assert!(value.pointer("/services/0/processRole").is_none());
        let service: RuntimeServiceSummary =
            serde_json::from_value(value["services"][0].clone()).expect("service deserializes");
        assert_eq!(service.role, RuntimeServiceRole::Hypervisor);
    }

    #[test]
    fn usb_enroll_is_not_public_wire() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "usb enroll",
            "payload": {
                "vm": "corp-vm",
                "mediaRef": "installer-usb",
                "busId": "1-2.3",
                "apply": true
            }
        }))
        .expect("encodes");
        decode_frame::<PublicRequest>("PublicRequest", &frame)
            .expect_err("removed enroll verb fails");
    }

    // A stray `{:?}` on any exec DTO must never leak argv, env keys or
    // values, cwd, raw stdio bytes, or the opaque session handle. Each sentinel
    // below is a unique marker that, if it appeared in the formatted output,
    // would prove a redaction regression.
    #[test]
    fn exec_dto_debug_redacts_secrets() {
        use super::{
            ExecCloseArgs, ExecDetachedCreateResult, ExecDetachedKillArgs, ExecDetachedKillOutcome,
            ExecDetachedKillResult, ExecDetachedListArgs, ExecDetachedListEntry,
            ExecDetachedListResult, ExecDetachedLogsArgs, ExecDetachedLogsResult,
            ExecDetachedStatusArgs, ExecDetachedStatusResult, ExecEnvVar, ExecOp, ExecOpResponse,
            ExecReadOutputArgs, ExecReadOutputResult, ExecResizeArgs, ExecSignalArgs,
            ExecStartArgs, ExecStartResult, ExecState, ExecStream, ExecWaitArgs,
            ExecWriteStdinArgs,
        };

        const SECRET_ENV_KEY: &str = "SENTINEL_ENV_KEY_b6f1";
        const SECRET_ENV_VAL: &str = "SENTINEL_ENV_VALUE_a90c";
        const SECRET_ARGV: &str = "SENTINEL_ARGV_3d2e";
        const SECRET_CWD: &str = "SENTINEL_CWD_77ab";
        const SECRET_HANDLE: &str = "SENTINEL_HANDLE_c41f";
        const SECRET_CHUNK: &str = "U0VOVElORUxfQ0hVTktfZGVhZA==";
        const SECRET_DATA: &str = "U0VOVElORUxfREFUQV9iZWVm";
        const SECRET_STDOUT: &str = "U0VOVElORUxfU1RET1VUXzM2YWE=";
        const SECRET_STDERR: &str = "U0VOVElORUxfU1RERVJSXzU0YmM=";
        const SECRET_REASON: &str = "SENTINEL_REASON_c35d";

        let secrets = [
            SECRET_ENV_KEY,
            SECRET_ENV_VAL,
            SECRET_ARGV,
            SECRET_CWD,
            SECRET_HANDLE,
            SECRET_CHUNK,
            SECRET_DATA,
            SECRET_STDOUT,
            SECRET_STDERR,
            SECRET_REASON,
        ];

        let assert_clean = |rendered: &str, label: &str| {
            for secret in secrets {
                assert!(
                    !rendered.contains(secret),
                    "{label} Debug leaked sentinel {secret}: {rendered}"
                );
            }
        };

        let env_var = ExecEnvVar {
            key: SECRET_ENV_KEY.to_owned(),
            value: SECRET_ENV_VAL.to_owned(),
        };
        assert_clean(&format!("{env_var:?}"), "ExecEnvVar");

        let start = ExecStartArgs {
            vm: "corp-vm".to_owned(),
            argv: vec!["sh".to_owned(), SECRET_ARGV.to_owned()],
            tty: true,
            detached: false,
            env: Some(vec![env_var.clone()]),
            cwd: Some(SECRET_CWD.to_owned()),
            term_size: None,
        };
        let rendered = format!("{start:?}");
        assert_clean(&rendered, "ExecStartArgs");
        assert!(rendered.contains("argv_len"), "argv length is observable");
        assert!(rendered.contains("env_len"), "env length is observable");
        assert!(rendered.contains("has_cwd"), "cwd presence is observable");
        assert!(
            !rendered.contains("corp-vm"),
            "create Debug omits VM name and exposes only command-shape counts"
        );
        assert_clean(
            &format!("{:?}", ExecOp::Start(start.clone())),
            "ExecOp::Start",
        );

        let write = ExecWriteStdinArgs {
            session: SECRET_HANDLE.to_owned(),
            offset: 17,
            chunk_base64: SECRET_CHUNK.to_owned(),
            eof: false,
        };
        assert_clean(&format!("{write:?}"), "ExecWriteStdinArgs");

        let read = ExecReadOutputArgs {
            session: SECRET_HANDLE.to_owned(),
            stream: ExecStream::Stdout,
            offset: 0,
            max_len: 4096,
            wait: true,
            timeout_ms: 250,
        };
        assert_clean(&format!("{read:?}"), "ExecReadOutputArgs");

        let signal = ExecSignalArgs {
            session: SECRET_HANDLE.to_owned(),
            signo: 15,
            op_id: 3,
        };
        assert_clean(&format!("{signal:?}"), "ExecSignalArgs");

        let resize = ExecResizeArgs {
            session: SECRET_HANDLE.to_owned(),
            rows: 24,
            cols: 80,
            op_id: 4,
        };
        assert_clean(&format!("{resize:?}"), "ExecResizeArgs");

        let wait = ExecWaitArgs {
            session: SECRET_HANDLE.to_owned(),
            timeout_ms: 1000,
        };
        assert_clean(&format!("{wait:?}"), "ExecWaitArgs");

        let close = ExecCloseArgs {
            session: SECRET_HANDLE.to_owned(),
        };
        assert_clean(&format!("{close:?}"), "ExecCloseArgs");

        let start_result = ExecStartResult {
            session: SECRET_HANDLE.to_owned(),
            tty: true,
            stdout_offset: 0,
            stderr_offset: 0,
        };
        assert_clean(&format!("{start_result:?}"), "ExecStartResult");

        let read_result = ExecReadOutputResult {
            data_base64: SECRET_DATA.to_owned(),
            next_offset: 64,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        };
        assert_clean(&format!("{read_result:?}"), "ExecReadOutputResult");

        let detached_create = ExecDetachedCreateResult {
            exec_id: "exec-0001".to_owned(),
            state: ExecState::Running,
        };
        assert_clean(&format!("{detached_create:?}"), "ExecDetachedCreateResult");
        assert_clean(
            &format!("{:?}", ExecOpResponse::DetachedCreate(detached_create)),
            "ExecOpResponse::DetachedCreate",
        );

        let detached_list = ExecDetachedListArgs {
            vm: "corp-vm".to_owned(),
        };
        assert_clean(&format!("{detached_list:?}"), "ExecDetachedListArgs");
        assert_clean(
            &format!("{:?}", ExecOp::List(detached_list)),
            "ExecOp::List",
        );

        let detached_status = ExecDetachedStatusArgs {
            vm: "corp-vm".to_owned(),
            exec_id: "exec-0001".to_owned(),
        };
        assert_clean(&format!("{detached_status:?}"), "ExecDetachedStatusArgs");
        assert_clean(
            &format!("{:?}", ExecOp::Status(detached_status)),
            "ExecOp::Status",
        );

        let detached_logs = ExecDetachedLogsArgs {
            vm: "corp-vm".to_owned(),
            exec_id: "exec-0001".to_owned(),
            stdout_offset: Some(64),
            stderr_offset: Some(96),
            max_len: Some(4096),
        };
        assert_clean(&format!("{detached_logs:?}"), "ExecDetachedLogsArgs");
        assert_clean(
            &format!("{:?}", ExecOp::Logs(detached_logs.clone())),
            "ExecOp::Logs",
        );

        let detached_kill = ExecDetachedKillArgs {
            vm: "corp-vm".to_owned(),
            exec_id: "exec-0001".to_owned(),
        };
        assert_clean(&format!("{detached_kill:?}"), "ExecDetachedKillArgs");
        assert_clean(
            &format!("{:?}", ExecOp::Kill(detached_kill)),
            "ExecOp::Kill",
        );

        let detached_entry = ExecDetachedListEntry {
            exec_id: "exec-0001".to_owned(),
            state: ExecState::Exited,
            exit_code: Some(0),
            signal: None,
            started_at: "2026-06-15T18:00:00Z".to_owned(),
            start_offset: 0,
            end_offset: 128,
            stdout_start_offset: 0,
            stdout_end_offset: 64,
            stderr_start_offset: 8,
            stderr_end_offset: 128,
            dropped_bytes: 0,
            stdout_dropped_bytes: 0,
            stderr_dropped_bytes: 0,
            truncated: false,
            stdout_truncated: false,
            stderr_truncated: false,
        };
        assert_clean(&format!("{detached_entry:?}"), "ExecDetachedListEntry");

        let detached_list_result = ExecDetachedListResult {
            execs: vec![detached_entry],
        };
        assert_clean(
            &format!("{detached_list_result:?}"),
            "ExecDetachedListResult",
        );
        assert_clean(
            &format!("{:?}", ExecOpResponse::List(detached_list_result)),
            "ExecOpResponse::List",
        );

        let detached_status_result = ExecDetachedStatusResult {
            exec_id: "exec-0001".to_owned(),
            state: ExecState::ProtocolError,
            reason: Some(SECRET_REASON.to_owned()),
            exit_code: None,
            signal: None,
            start_offset: 0,
            end_offset: 128,
            dropped_bytes: 0,
            truncated: false,
        };
        assert_clean(
            &format!("{detached_status_result:?}"),
            "ExecDetachedStatusResult",
        );
        assert_clean(
            &format!(
                "{:?}",
                ExecOpResponse::Status(detached_status_result.clone())
            ),
            "ExecOpResponse::Status",
        );

        let detached_logs_result = ExecDetachedLogsResult {
            exec_id: "exec-0001".to_owned(),
            stdout_base64: SECRET_STDOUT.to_owned(),
            stderr_base64: SECRET_STDERR.to_owned(),
            start_offset: 0,
            end_offset: 128,
            dropped_bytes: 64,
            truncated: true,
            stdout_start_offset: 0,
            stdout_end_offset: 64,
            stdout_next_offset: 64,
            stdout_eof: true,
            stdout_dropped_bytes: 16,
            stdout_truncated: true,
            stderr_start_offset: 8,
            stderr_end_offset: 128,
            stderr_next_offset: 96,
            stderr_eof: false,
            stderr_dropped_bytes: 48,
            stderr_truncated: false,
        };
        let rendered_logs = format!("{detached_logs_result:?}");
        assert_clean(&rendered_logs, "ExecDetachedLogsResult");
        assert!(
            rendered_logs.contains("data_len")
                && rendered_logs.contains("dropped")
                && rendered_logs.contains("truncated"),
            "logs Debug exposes only length/accounting metadata: {rendered_logs}"
        );
        assert!(
            !rendered_logs.contains("exec-0001")
                && !rendered_logs.contains("start_offset")
                && !rendered_logs.contains("end_offset"),
            "logs Debug must not expose IDs, offsets, or payloads: {rendered_logs}"
        );
        assert_clean(
            &format!("{:?}", ExecOpResponse::Logs(detached_logs_result)),
            "ExecOpResponse::Logs",
        );

        let detached_kill_result = ExecDetachedKillResult {
            exec_id: "exec-0001".to_owned(),
            result: ExecDetachedKillOutcome::Cancelling,
            state: ExecState::Running,
        };
        assert_clean(
            &format!("{detached_kill_result:?}"),
            "ExecDetachedKillResult",
        );
        assert_clean(
            &format!("{:?}", ExecOpResponse::Kill(detached_kill_result)),
            "ExecOpResponse::Kill",
        );
    }

    #[test]
    fn shell_name_enforces_adr_shape() {
        use super::ShellName;

        for name in ["default", "dev_1", "A.B-c_9", "_scratch"] {
            assert!(ShellName::new(name).is_ok(), "{name} should be valid");
            let json = serde_json::json!(name);
            let decoded: ShellName = serde_json::from_value(json).expect("valid name decodes");
            assert_eq!(decoded.as_str(), name);
        }

        let too_long = "a".repeat(65);
        for name in [
            "",
            ".",
            "..",
            "-default",
            "bad/name",
            "bad name",
            "bad{name}",
            "bad\nname",
            too_long.as_str(),
        ] {
            assert!(ShellName::new(name).is_err(), "{name:?} should be invalid");
            let json = serde_json::json!(name);
            serde_json::from_value::<ShellName>(json).expect_err("invalid name rejects");
        }
    }

    #[test]
    fn shell_dto_debug_redacts_names_handles_and_output() {
        use super::{
            ShellAttachArgs, ShellAttachResult, ShellCloseAttachArgs, ShellDetachArgs,
            ShellDetachResult, ShellKillArgs, ShellKillResult, ShellListEntry, ShellListResult,
            ShellName, ShellSessionState,
        };

        const SECRET_NAME: &str = "SENTINEL_NAME_cafe";
        const SECRET_HANDLE: &str = "SENTINEL_HANDLE_feed";

        let name = ShellName::new(SECRET_NAME).expect("valid sentinel name");
        let assert_clean = |rendered: &str, label: &str| {
            for secret in [SECRET_NAME, SECRET_HANDLE] {
                assert!(
                    !rendered.contains(secret),
                    "{label} Debug leaked sentinel {secret}: {rendered}"
                );
            }
        };

        let attach = ShellAttachArgs {
            vm: "corp-vm".to_owned(),
            name: Some(name.clone()),
            force: true,
            initial_terminal_size: crate::terminal_wire::TerminalSize { rows: 24, cols: 80 },
        };
        assert_clean(&format!("{attach:?}"), "ShellAttachArgs");

        let attach_result = ShellAttachResult {
            session: SECRET_HANDLE.to_owned(),
            resolved_name: name.clone(),
            state: ShellSessionState::Attached,
            force_evicted: true,
        };
        assert_clean(&format!("{attach_result:?}"), "ShellAttachResult");

        let detach = ShellDetachArgs {
            vm: "corp-vm".to_owned(),
            name: Some(name.clone()),
        };
        assert_clean(&format!("{detach:?}"), "ShellDetachArgs");

        let kill = ShellKillArgs {
            vm: "corp-vm".to_owned(),
            name: name.clone(),
        };
        assert_clean(&format!("{kill:?}"), "ShellKillArgs");

        let close = ShellCloseAttachArgs {
            session: SECRET_HANDLE.to_owned(),
        };
        assert_clean(&format!("{close:?}"), "ShellCloseAttachArgs");

        let entry = ShellListEntry {
            name: name.clone(),
            state: ShellSessionState::Detached,
            attached: false,
            is_default: true,
        };
        assert_clean(&format!("{entry:?}"), "ShellListEntry");

        let list = ShellListResult {
            default_name: name.clone(),
            sessions: vec![entry],
        };
        assert_clean(&format!("{list:?}"), "ShellListResult");

        let detached = ShellDetachResult {
            resolved_name: name.clone(),
            detached: true,
            cause: None,
        };
        assert_clean(&format!("{detached:?}"), "ShellDetachResult");

        let killed = ShellKillResult {
            name,
            killed: true,
            state: ShellSessionState::Killed,
        };
        assert_clean(&format!("{killed:?}"), "ShellKillResult");
    }

    #[test]
    fn shell_public_wire_json_shape_is_stable() {
        use super::{
            PublicResponse, ShellAttachArgs, ShellAttachResult, ShellListEntry, ShellListResult,
            ShellName, ShellOp, ShellOpResponse, ShellSessionState,
        };

        let attach = PublicRequest::Shell(ShellOp::Attach(ShellAttachArgs {
            vm: "corp-vm".to_owned(),
            name: Some(ShellName::new("ops_1").expect("valid name")),
            force: true,
            initial_terminal_size: crate::terminal_wire::TerminalSize { rows: 24, cols: 80 },
        }));
        let value = serde_json::to_value(&attach).expect("shell attach serializes");
        assert_eq!(value["kind"], "shell");
        assert_eq!(value["payload"]["op"], "attach");
        assert_eq!(value["payload"]["args"]["vm"], "corp-vm");
        assert_eq!(value["payload"]["args"]["name"], "ops_1");
        assert_eq!(value["payload"]["args"]["force"], true);
        assert_eq!(value["payload"]["args"]["initialTerminalSize"]["rows"], 24);

        let decoded: PublicRequest =
            serde_json::from_value(value.clone()).expect("shell attach decodes");
        assert_eq!(decoded, attach);

        let kill_without_name = serde_json::json!({
            "kind": "shell",
            "payload": {
                "op": "kill",
                "args": { "vm": "corp-vm" }
            }
        });
        serde_json::from_value::<PublicRequest>(kill_without_name)
            .expect_err("kill requires an explicit shell name");

        let list = PublicResponse::Shell(ShellOpResponse::List(ShellListResult {
            default_name: ShellName::new("default").expect("valid default"),
            sessions: vec![ShellListEntry {
                name: ShellName::new("default").expect("valid entry"),
                state: ShellSessionState::Detached,
                attached: false,
                is_default: true,
            }],
        }));
        let value = serde_json::to_value(&list).expect("shell list response serializes");
        assert_eq!(value["kind"], "shell");
        assert_eq!(value["payload"]["op"], "list");
        assert_eq!(value["payload"]["result"]["defaultName"], "default");
        assert_eq!(value["payload"]["result"]["sessions"][0]["name"], "default");
        assert_eq!(value["payload"]["result"]["sessions"][0]["isDefault"], true);

        let attach_response = PublicResponse::Shell(ShellOpResponse::Attach(ShellAttachResult {
            session: "opaque-session".to_owned(),
            resolved_name: ShellName::new("default").expect("valid result"),
            state: ShellSessionState::Attached,
            force_evicted: false,
        }));
        let value =
            serde_json::to_value(&attach_response).expect("shell attach response serializes");
        assert_eq!(value["payload"]["result"]["resolvedName"], "default");
        assert_eq!(value["payload"]["result"]["state"], "attached");
    }

    #[test]
    fn console_public_wire_json_shape_is_stable() {
        use super::{
            ConsoleAttachArgs, ConsoleAttachResult, ConsoleOp, ConsoleOpResponse,
            ConsoleProviderKind, PublicResponse,
        };

        let attach = PublicRequest::Console(ConsoleOp::Attach(ConsoleAttachArgs {
            vm: "corp-vm".to_owned(),
            initial_terminal_size: crate::terminal_wire::TerminalSize { rows: 24, cols: 80 },
        }));
        let value = serde_json::to_value(&attach).expect("console attach serializes");
        assert_eq!(value["kind"], "console");
        assert_eq!(value["payload"]["op"], "attach");
        assert_eq!(value["payload"]["args"]["vm"], "corp-vm");
        assert_eq!(value["payload"]["args"]["initialTerminalSize"]["rows"], 24);

        let decoded: PublicRequest =
            serde_json::from_value(value.clone()).expect("console attach decodes");
        assert_eq!(decoded, attach);

        let attach_response =
            PublicResponse::Console(ConsoleOpResponse::Attach(ConsoleAttachResult {
                session: "opaque-console".to_owned(),
                provider_kind: ConsoleProviderKind::LocalHypervisor,
                ring_buffer_start_offset: 0,
            }));
        let value =
            serde_json::to_value(&attach_response).expect("console attach response serializes");
        assert_eq!(value["kind"], "console");
        assert_eq!(value["payload"]["op"], "attach");
        assert_eq!(
            value["payload"]["result"]["providerKind"],
            "local-hypervisor"
        );
        assert_eq!(value["payload"]["result"]["ringBufferStartOffset"], 0);
        // session must not be present in the serialized output (it is, but must
        // be redacted in Debug output — verify Debug does not leak it).
        let debug_str = format!("{attach_response:?}");
        assert!(
            !debug_str.contains("opaque-console"),
            "ConsoleAttachResult Debug must not leak session handle"
        );
    }

    #[test]
    fn console_session_handle_is_redacted_in_debug() {
        use super::{
            ConsoleCloseArgs, ConsoleReadOutputArgs, ConsoleResizeArgs, ConsoleWaitArgs,
            ConsoleWriteStdinArgs,
        };
        const SECRET: &str = "secret-session-handle";

        let write = ConsoleWriteStdinArgs {
            session: SECRET.to_owned(),
            offset: 0,
            chunk_base64: "aGVsbG8=".to_owned(),
            eof: false,
        };
        assert!(!format!("{write:?}").contains(SECRET));

        let read = ConsoleReadOutputArgs {
            session: SECRET.to_owned(),
            stream: crate::terminal_wire::TerminalStream::Stdout,
            offset: 0,
            max_len: 4096,
            wait: false,
            timeout_ms: 0,
        };
        assert!(!format!("{read:?}").contains(SECRET));

        let resize = ConsoleResizeArgs {
            session: SECRET.to_owned(),
            size: crate::terminal_wire::TerminalSize { rows: 24, cols: 80 },
        };
        assert!(!format!("{resize:?}").contains(SECRET));

        let wait = ConsoleWaitArgs {
            session: SECRET.to_owned(),
            timeout_ms: 5000,
        };
        assert!(!format!("{wait:?}").contains(SECRET));

        let close = ConsoleCloseArgs {
            session: SECRET.to_owned(),
        };
        assert!(!format!("{close:?}").contains(SECRET));
    }

    #[test]
    fn audio_public_wire_json_shape_is_stable() {
        use super::{
            AudioChannel, AudioChannelState, AudioEnforcementPosture, AudioErrorKind,
            AudioMuteArgs, AudioOp, AudioOpResponse, AudioProviderKind, AudioSetApplied,
            AudioSetResult, AudioStatusArgs, AudioStatusResult, AudioVmError, AudioVmState,
            PublicResponse,
        };

        // Status request with explicit VM list.
        let status = PublicRequest::Audio(AudioOp::Status(AudioStatusArgs {
            vms: vec!["corp-vm".to_owned(), "work-vm".to_owned()],
        }));
        let value = serde_json::to_value(&status).expect("audio status serializes");
        assert_eq!(value["kind"], "audio");
        assert_eq!(value["payload"]["op"], "status");
        assert_eq!(value["payload"]["args"]["vms"][0], "corp-vm");

        let decoded: PublicRequest = serde_json::from_value(value).expect("audio status decodes");
        assert_eq!(decoded, status);

        // Empty-vms status request (query-all).
        let all = PublicRequest::Audio(AudioOp::Status(AudioStatusArgs { vms: vec![] }));
        let value = serde_json::to_value(&all).expect("empty status serializes");
        assert_eq!(value["payload"]["args"]["vms"].as_array().unwrap().len(), 0);

        // Mute request.
        let mute = PublicRequest::Audio(AudioOp::Mute(AudioMuteArgs {
            vm: "corp-vm".to_owned(),
            channel: AudioChannel::Speaker,
            mute: true,
        }));
        let value = serde_json::to_value(&mute).expect("mute serializes");
        assert_eq!(value["payload"]["op"], "mute");
        assert_eq!(value["payload"]["args"]["channel"], "speaker");
        assert_eq!(value["payload"]["args"]["mute"], true);

        // Status response with an entry and an error.
        let status_response = PublicResponse::Audio(AudioOpResponse::Status(AudioStatusResult {
            entries: vec![AudioVmState {
                vm: "corp-vm".to_owned(),
                speaker: AudioChannelState {
                    level: Some(LevelPercent::new(80).expect("valid level")),
                    muted: false,
                },
                microphone: AudioChannelState {
                    level: Some(LevelPercent::new(60).expect("valid level")),
                    muted: true,
                },
                provider_kind: AudioProviderKind::LocalHypervisor,
                enforcement: AudioEnforcementPosture::HostAndGuest,
            }],
            errors: vec![AudioVmError {
                vm: "missing-vm".to_owned(),
                kind: AudioErrorKind::VmNotFound,
                remediation: None,
            }],
        }));
        let value =
            serde_json::to_value(&status_response).expect("audio status response serializes");
        assert_eq!(value["kind"], "audio");
        assert_eq!(value["payload"]["op"], "status");
        assert_eq!(value["payload"]["result"]["entries"][0]["vm"], "corp-vm");
        assert_eq!(
            value["payload"]["result"]["entries"][0]["speaker"]["level"],
            80
        );
        assert_eq!(
            value["payload"]["result"]["entries"][0]["speaker"]["muted"],
            false
        );
        assert_eq!(
            value["payload"]["result"]["entries"][0]["microphone"]["muted"],
            true
        );
        assert_eq!(
            value["payload"]["result"]["entries"][0]["enforcement"],
            "host-and-guest"
        );
        assert_eq!(value["payload"]["result"]["errors"][0]["vm"], "missing-vm");
        assert_eq!(
            value["payload"]["result"]["errors"][0]["kind"],
            "vm-not-found"
        );

        // provider-misconfigured error serializes to the canonical wire string.
        let pm_error = AudioVmError {
            vm: "aca-vm".to_owned(),
            kind: AudioErrorKind::ProviderMisconfigured,
            remediation: Some("Check the provider agent is running.".to_owned()),
        };
        let pm_value = serde_json::to_value(&pm_error).expect("provider-misconfigured serializes");
        assert_eq!(pm_value["kind"], "provider-misconfigured");
        let roundtrip: AudioVmError =
            serde_json::from_value(pm_value).expect("provider-misconfigured roundtrips");
        assert_eq!(roundtrip.kind, AudioErrorKind::ProviderMisconfigured);

        // Set-volume response.
        let set_response = PublicResponse::Audio(AudioOpResponse::SetVolume(AudioSetResult {
            vm: "corp-vm".to_owned(),
            channel: AudioChannel::Microphone,
            applied: AudioSetApplied::HostAndGuest,
            state: AudioChannelState {
                level: Some(LevelPercent::new(50).expect("valid level")),
                muted: false,
            },
        }));
        let value = serde_json::to_value(&set_response).expect("set-volume response serializes");
        assert_eq!(value["payload"]["op"], "setVolume");
        assert_eq!(value["payload"]["result"]["channel"], "microphone");
        assert_eq!(value["payload"]["result"]["applied"], "host-and-guest");
        assert_eq!(value["payload"]["result"]["state"]["level"], 50);
    }

    #[test]
    fn level_percent_validates_range_at_wire_boundary() {
        // Values in range round-trip cleanly.
        for v in [0u8, 1, 50, 99, 100] {
            let lp = LevelPercent::new(v).expect("valid level");
            assert_eq!(lp.get(), v);
            let json = serde_json::to_value(lp).expect("serializes");
            let decoded: LevelPercent = serde_json::from_value(json).expect("deserializes");
            assert_eq!(decoded.get(), v);
        }

        // Out-of-range construction fails.
        assert!(LevelPercent::new(101).is_err());

        // Out-of-range wire value is rejected at deserialize time.
        let bad: Result<LevelPercent, _> = serde_json::from_str("101");
        assert!(bad.is_err(), "level 101 must be rejected at wire boundary");

        // 100 is the exact cap and must be accepted.
        let at_cap: LevelPercent = serde_json::from_str("100").expect("100 is valid");
        assert_eq!(at_cap.get(), 100);
    }

    #[test]
    fn audio_set_volume_rejects_out_of_range_level() {
        use super::{AudioChannel, AudioSetVolumeArgs};

        let bad_json = serde_json::json!({
            "vm": "corp-vm",
            "channel": "speaker",
            "level": 101
        });
        serde_json::from_value::<AudioSetVolumeArgs>(bad_json)
            .expect_err("level 101 must be rejected");

        let good_json = serde_json::json!({
            "vm": "corp-vm",
            "channel": "speaker",
            "level": 75
        });
        let args =
            serde_json::from_value::<AudioSetVolumeArgs>(good_json).expect("level 75 is valid");
        assert_eq!(args.level.get(), 75);
        assert_eq!(args.channel, AudioChannel::Speaker);
    }

    #[test]
    fn audio_status_unknown_fields_fail_closed() {
        use super::AudioStatusArgs;
        let bad = serde_json::json!({ "vms": [], "extraField": true });
        serde_json::from_value::<AudioStatusArgs>(bad).expect_err("unknown field must be rejected");
    }
}
