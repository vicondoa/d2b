use crate::{FeatureFlag, Version};
use nixling_core::{error::Error, host::IfName};
use schemars::JsonSchema;
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
    /// Dedicated reconcile verb that re-runs the daemon-side net-route
    /// preflight + the broker-side per-env nftables / route / sysctl
    /// reconcile
    /// without starting any VM. On success it resets the
    /// operator-only-mode counter so future daemon startups are
    /// no longer locked out of autostart. The CLI exposes this as
    /// `nixling host reconcile --network --apply`.
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
    /// connect, auth, or `ExecCreate`. `nixling vm exec` (and the
    /// `vm konsole` wrapper) drive this verb; it never crosses SSH.
    #[serde(rename = "exec")]
    Exec(ExecOp),
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
/// `{:?}` can never leak a key or secret value (WR12).
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
/// the command, and the session shape. `detached` is carried so the daemon
/// teardown semantics are unambiguous (a detached session survives owner
/// disconnect); the W16 CLI ships non-detached + interactive `-it` only.
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
        // Redaction (WR12): show the VM name + shape + counts, never the raw
        // argv / env keys+values / cwd.
        f.debug_struct("ExecStartArgs")
            .field("vm", &self.vm)
            .field("tty", &self.tty)
            .field("detached", &self.detached)
            .field("argv_len", &self.argv.len())
            .field("env_len", &self.env.as_ref().map_or(0, Vec::len))
            .field("has_cwd", &self.cwd.is_some())
            .field("term_size", &self.term_size)
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
        // Redaction (WR12): the session handle + the raw stdin chunk (keystroke
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
        // Redaction (WR12): the session handle never appears.
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
/// of the exec (W14 semantics); the CLI maps host SIGINT/SIGTSTP/SIGTERM to
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

/// `Close` op args. Idempotent: closing an already-closed/torn-down session
/// returns success.
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
        // Redaction (WR12): the issued session handle never appears.
        f.debug_struct("ExecStartResult")
            .field("session", &"<redacted>")
            .field("tty", &self.tty)
            .field("stdout_offset", &self.stdout_offset)
            .field("stderr_offset", &self.stderr_offset)
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
        // Redaction (WR12): the raw guest stdout/stderr bytes never appear; show
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

/// `Close` op result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCloseResult {
    #[serde(default)]
    pub stdin_closed: bool,
}

/// Multiplexed exec operation result. Closed adjacently-tagged enum mirroring
/// [`ExecOp`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "result", rename_all = "camelCase")]
pub enum ExecOpResponse {
    Start(ExecStartResult),
    WriteStdin(ExecWriteStdinResult),
    ReadOutput(ExecReadOutputResult),
    Signal(ExecControlResult),
    Resize(ExecControlResult),
    Wait(ExecWaitResult),
    Close(ExecCloseResult),
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

    // WR12: a stray `{:?}` on any exec DTO must never leak argv, env keys or
    // values, cwd, raw stdio bytes, or the opaque session handle. Each sentinel
    // below is a unique marker that, if it appeared in the formatted output,
    // would prove a redaction regression.
    #[test]
    fn exec_dto_debug_redacts_secrets() {
        use super::{
            ExecCloseArgs, ExecEnvVar, ExecReadOutputArgs, ExecReadOutputResult, ExecResizeArgs,
            ExecSignalArgs, ExecStartArgs, ExecStartResult, ExecStream, ExecWaitArgs,
            ExecWriteStdinArgs,
        };

        const SECRET_ENV_KEY: &str = "SENTINEL_ENV_KEY_b6f1";
        const SECRET_ENV_VAL: &str = "SENTINEL_ENV_VALUE_a90c";
        const SECRET_ARGV: &str = "SENTINEL_ARGV_3d2e";
        const SECRET_CWD: &str = "SENTINEL_CWD_77ab";
        const SECRET_HANDLE: &str = "SENTINEL_HANDLE_c41f";
        const SECRET_CHUNK: &str = "U0VOVElORUxfQ0hVTktfZGVhZA==";
        const SECRET_DATA: &str = "U0VOVElORUxfREFUQV9iZWVm";

        let secrets = [
            SECRET_ENV_KEY,
            SECRET_ENV_VAL,
            SECRET_ARGV,
            SECRET_CWD,
            SECRET_HANDLE,
            SECRET_CHUNK,
            SECRET_DATA,
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
        assert!(rendered.contains("corp-vm"), "vm name is observable");
        assert!(rendered.contains("argv_len"), "argv length is observable");

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
    }
}
