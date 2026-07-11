//! Private unsafe-local helper protocol.
//!
//! The authenticated Unix peer credential is the execution identity. No frame
//! carries a uid, environment, cwd, compositor path, or arbitrary public argv.

use crate::{
    public_wire::{
        EXEC_MAX_CHUNK_BYTES, ShellCloseCause, ShellDetachResult, ShellKillResult, ShellListResult,
        ShellName, ShellSessionState,
    },
    terminal_wire::{
        TerminalCloseResult, TerminalControlResult, TerminalReadOutputChunk, TerminalSize,
        TerminalStream, TerminalWaitResult, TerminalWriteStdinResult,
    },
};
use d2b_core::{configured_argv::ConfiguredArgv, workload_identity::WorkloadIdentity};
use d2b_realm_core::{ids::OperationId, token::ProtocolToken};
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fmt;

pub const UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION: u32 = 2;
pub const UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION: u32 = 1;
/// Every terminal-ready frame carries exactly one connected Unix stream fd.
pub const UNSAFE_LOCAL_TERMINAL_FD_COUNT: usize = 1;
pub const MAX_HELPER_FRAME_SIZE: usize = 256 * 1024;
/// Maximum length-prefixed JSON frame on one attached terminal stream.
pub const MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE: usize = 128 * 1024;
pub const UNSAFE_LOCAL_TERMINAL_LENGTH_PREFIX_BYTES: usize = 4;
/// Maximum decoded terminal input or output chunk.
pub const MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES: u64 = EXEC_MAX_CHUNK_BYTES;
/// Maximum standard padded base64 envelope for one terminal chunk.
pub const MAX_UNSAFE_LOCAL_TERMINAL_BASE64_BYTES: usize =
    (MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES as usize).div_ceil(3) * 4;
/// Maximum retained output per persistent shell stream.
pub const MAX_UNSAFE_LOCAL_TERMINAL_OUTPUT_RING_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum duration of one terminal long-poll.
pub const MAX_UNSAFE_LOCAL_TERMINAL_WAIT_TIMEOUT_MS: u64 = 1_000;
pub const MAX_UNSAFE_LOCAL_TERMINAL_ROWS: u32 = 65_535;
pub const MAX_UNSAFE_LOCAL_TERMINAL_COLS: u32 = 65_535;
pub const MAX_HELPER_SUPERVISOR_ID_BYTES: usize = 128;
const _: () =
    assert!(MAX_UNSAFE_LOCAL_TERMINAL_BASE64_BYTES + 1024 < MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE);
/// Value requested through `SO_SNDBUF` and `SO_RCVBUF` on both control peers.
pub const HELPER_SOCKET_BUFFER_REQUEST_BYTES: usize = MAX_HELPER_FRAME_SIZE;
/// Minimum value that `getsockopt` must report after Linux doubles the request.
pub const MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES: usize = MAX_HELPER_FRAME_SIZE * 2;
pub const MAX_HELPER_QUEUE_DEPTH: usize = 128;
pub const MAX_HELPER_SNAPSHOT_SCOPES: usize = 1024;
pub const MAX_COMPLETED_OPERATIONS_PER_UID: usize = 1024;
pub const MAX_COMPLETED_OPERATION_AGE_SECS: u64 = 24 * 60 * 60;

pub const fn unsafe_local_helper_protocol_supported(version: u32) -> bool {
    version == UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION
}

pub fn encode_unsafe_local_terminal_frame<T>(message: &T) -> Result<Vec<u8>, HelperFailureCode>
where
    T: Serialize,
{
    let body = serde_json::to_vec(message).map_err(|_| HelperFailureCode::InvalidRequest)?;
    if body.len() > MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE {
        return Err(HelperFailureCode::InvalidRequest);
    }
    let mut frame = Vec::with_capacity(UNSAFE_LOCAL_TERMINAL_LENGTH_PREFIX_BYTES + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode_unsafe_local_terminal_frame<T>(frame: &[u8]) -> Result<T, HelperFailureCode>
where
    T: DeserializeOwned,
{
    if frame.len() < UNSAFE_LOCAL_TERMINAL_LENGTH_PREFIX_BYTES {
        return Err(HelperFailureCode::InvalidRequest);
    }
    let declared_length = u32::from_le_bytes(
        frame[..UNSAFE_LOCAL_TERMINAL_LENGTH_PREFIX_BYTES]
            .try_into()
            .map_err(|_| HelperFailureCode::InvalidRequest)?,
    ) as usize;
    if declared_length > MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE
        || frame.len() != UNSAFE_LOCAL_TERMINAL_LENGTH_PREFIX_BYTES + declared_length
    {
        return Err(HelperFailureCode::InvalidRequest);
    }
    serde_json::from_slice(&frame[UNSAFE_LOCAL_TERMINAL_LENGTH_PREFIX_BYTES..])
        .map_err(|_| HelperFailureCode::InvalidRequest)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHello {
    pub protocol_version: u32,
    pub generation: u64,
    #[serde(default)]
    pub features: Vec<ProtocolToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHelloAccepted {
    pub protocol_version: u32,
    pub generation: u64,
    pub heartbeat_interval_secs: u32,
    pub operation_timeout_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHeartbeat {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperScopeKind {
    LauncherApp,
    WaylandProxy,
    PersistentShell,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeIdentity {
    pub invocation_id: String,
    pub kind: HelperScopeKind,
}

impl fmt::Debug for ScopeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeIdentity")
            .field("invocation_id", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperScopeState {
    Starting,
    Active,
    Stopping,
    Exited,
    Degraded,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct HelperSupervisorId(String);

impl HelperSupervisorId {
    pub fn new(value: impl Into<String>) -> Result<Self, HelperFailureCode> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_HELPER_SUPERVISOR_ID_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            });
        valid
            .then_some(Self(value))
            .ok_or(HelperFailureCode::InvalidRequest)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for HelperSupervisorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HelperSupervisorId(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for HelperSupervisorId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| {
            serde::de::Error::custom(
                "helper supervisor id must be 1..=128 bytes of [A-Za-z0-9._:-]",
            )
        })
    }
}

impl JsonSchema for HelperSupervisorId {
    fn schema_name() -> String {
        "HelperSupervisorId".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        bounded_string_schema(
            1,
            MAX_HELPER_SUPERVISOR_ID_BYTES as u32,
            "^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$",
            "Opaque persistent-shell supervisor identity.",
        )
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperPersistentShellSnapshot {
    pub name: ShellName,
    pub state: ShellSessionState,
    pub attached: bool,
    pub supervisor_id: HelperSupervisorId,
}

impl fmt::Debug for HelperPersistentShellSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperPersistentShellSnapshot")
            .field("name", &"<redacted>")
            .field("state", &self.state)
            .field("attached", &self.attached)
            .field("supervisor_id", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperScopeSnapshot {
    pub operation_id: OperationId,
    pub workload: WorkloadIdentity,
    pub scope: ScopeIdentity,
    pub state: HelperScopeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent_shell: Option<HelperPersistentShellSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperSnapshot {
    pub generation: u64,
    pub scopes: Vec<HelperScopeSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperLaunchRequest {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub workload: WorkloadIdentity,
    pub item_id: ProtocolToken,
    pub argv: ConfiguredArgv,
    pub graphical: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "op",
    content = "args",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HelperShellRequest {
    List {
        #[schemars(rename = "requestId")]
        request_id: u64,
        #[schemars(rename = "operationId")]
        operation_id: OperationId,
        workload: WorkloadIdentity,
    },
    Attach {
        #[schemars(rename = "requestId")]
        request_id: u64,
        #[schemars(rename = "operationId")]
        operation_id: OperationId,
        workload: WorkloadIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<ShellName>,
        #[serde(default)]
        force: bool,
        #[schemars(
            rename = "initialTerminalSize",
            schema_with = "bounded_terminal_size_schema"
        )]
        initial_terminal_size: TerminalSize,
    },
    Detach {
        #[schemars(rename = "requestId")]
        request_id: u64,
        #[schemars(rename = "operationId")]
        operation_id: OperationId,
        workload: WorkloadIdentity,
        name: ShellName,
    },
    Kill {
        #[schemars(rename = "requestId")]
        request_id: u64,
        #[schemars(rename = "operationId")]
        operation_id: OperationId,
        workload: WorkloadIdentity,
        name: ShellName,
    },
}

impl fmt::Debug for HelperShellRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List {
                request_id,
                operation_id,
                workload,
            } => f
                .debug_struct("HelperShellRequest::List")
                .field("request_id", request_id)
                .field("operation_id", operation_id)
                .field("workload", workload)
                .finish(),
            Self::Attach {
                request_id,
                operation_id,
                workload,
                name,
                force,
                initial_terminal_size,
            } => f
                .debug_struct("HelperShellRequest::Attach")
                .field("request_id", request_id)
                .field("operation_id", operation_id)
                .field("workload", workload)
                .field("name", &name.as_ref().map(|_| "<redacted>"))
                .field("force", force)
                .field("initial_terminal_size", initial_terminal_size)
                .finish(),
            Self::Detach {
                request_id,
                operation_id,
                workload,
                ..
            } => f
                .debug_struct("HelperShellRequest::Detach")
                .field("request_id", request_id)
                .field("operation_id", operation_id)
                .field("workload", workload)
                .field("name", &"<redacted>")
                .finish(),
            Self::Kill {
                request_id,
                operation_id,
                workload,
                ..
            } => f
                .debug_struct("HelperShellRequest::Kill")
                .field("request_id", request_id)
                .field("operation_id", operation_id)
                .field("workload", workload)
                .field("name", &"<redacted>")
                .finish(),
        }
    }
}

impl HelperShellRequest {
    pub fn request_id(&self) -> u64 {
        match self {
            Self::List { request_id, .. }
            | Self::Attach { request_id, .. }
            | Self::Detach { request_id, .. }
            | Self::Kill { request_id, .. } => *request_id,
        }
    }

    pub fn operation_id(&self) -> &OperationId {
        match self {
            Self::List { operation_id, .. }
            | Self::Attach { operation_id, .. }
            | Self::Detach { operation_id, .. }
            | Self::Kill { operation_id, .. } => operation_id,
        }
    }

    pub fn validate_bounds(&self) -> Result<(), HelperFailureCode> {
        match self {
            Self::Attach {
                initial_terminal_size,
                ..
            } if !terminal_size_valid(*initial_terminal_size) => {
                Err(HelperFailureCode::InvalidTerminalSize)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperFailureCode {
    InvalidRequest,
    OperationIdConflict,
    QueueFull,
    Timeout,
    UserManagerUnavailable,
    EnvironmentInvalid,
    ExecutableUnavailable,
    ScopeCreateFailed,
    ScopeIdentityMismatch,
    GraphicalSessionInactive,
    WaylandUnavailable,
    ProxyUnavailable,
    FirstClientTimeout,
    ShellUnavailable,
    ShellNotFound,
    ShellAlreadyAttached,
    TerminalOutputGap,
    TerminalOffsetMismatch,
    TerminalClosed,
    InvalidTerminalSize,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperOperationDisposition {
    Committed,
    AlreadyCommitted,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperOperationResult {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub disposition: HelperOperationDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeIdentity>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalReady {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub terminal_protocol_version: u32,
    pub transport: HelperTerminalTransport,
    pub scope: ScopeIdentity,
    pub result: HelperShellAttachResult,
}

impl fmt::Debug for HelperTerminalReady {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperTerminalReady")
            .field("request_id", &self.request_id)
            .field("operation_id", &self.operation_id)
            .field("terminal_protocol_version", &self.terminal_protocol_version)
            .field("transport", &self.transport)
            .field("scope", &self.scope)
            .field("result", &self.result)
            .finish()
    }
}

/// Transport represented by the single fd attached to a terminal-ready frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperTerminalTransport {
    /// A connected `AF_UNIX` `SOCK_STREAM`.
    ///
    /// Receivers must require `SO_TYPE == SOCK_STREAM`, `SO_ACCEPTCONN == 0`,
    /// and a successful `getpeername`; listeners, datagrams, and unconnected
    /// sockets are invalid.
    ConnectedUnixStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperOperationRejected {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub code: HelperFailureCode,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperShellAttachResult {
    pub resolved_name: ShellName,
    pub state: ShellSessionState,
    #[serde(default)]
    pub force_evicted: bool,
}

impl fmt::Debug for HelperShellAttachResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperShellAttachResult")
            .field("resolved_name", &"<redacted>")
            .field("state", &self.state)
            .field("force_evicted", &self.force_evicted)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperShellListResponse {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub result: ShellListResult,
}

impl fmt::Debug for HelperShellListResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperShellListResponse")
            .field("request_id", &self.request_id)
            .field("operation_id", &self.operation_id)
            .field("result", &self.result)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperShellDetachResponse {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub result: ShellDetachResult,
}

impl fmt::Debug for HelperShellDetachResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperShellDetachResponse")
            .field("request_id", &self.request_id)
            .field("operation_id", &self.operation_id)
            .field("result", &self.result)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperShellKillResponse {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub result: ShellKillResult,
}

impl fmt::Debug for HelperShellKillResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperShellKillResponse")
            .field("request_id", &self.request_id)
            .field("operation_id", &self.operation_id)
            .field("result", &self.result)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "op",
    content = "result",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum HelperShellResponse {
    List(HelperShellListResponse),
    Detach(HelperShellDetachResponse),
    Kill(HelperShellKillResponse),
}

impl HelperShellResponse {
    pub fn request_id(&self) -> u64 {
        match self {
            Self::List(response) => response.request_id,
            Self::Detach(response) => response.request_id,
            Self::Kill(response) => response.request_id,
        }
    }

    pub fn operation_id(&self) -> &OperationId {
        match self {
            Self::List(response) => &response.operation_id,
            Self::Detach(response) => &response.operation_id,
            Self::Kill(response) => &response.operation_id,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HelperTerminalChunkBase64(String);

impl HelperTerminalChunkBase64 {
    pub fn new(value: impl Into<String>) -> Result<Self, HelperFailureCode> {
        let value = value.into();
        valid_base64_chunk(&value)
            .then_some(Self(value))
            .ok_or(HelperFailureCode::InvalidRequest)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn decoded_len(&self) -> usize {
        decoded_base64_len(&self.0).unwrap_or(0)
    }
}

impl fmt::Debug for HelperTerminalChunkBase64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperTerminalChunkBase64")
            .field("encoded_len", &self.0.len())
            .field("decoded_len", &self.decoded_len())
            .finish()
    }
}

impl<'de> Deserialize<'de> for HelperTerminalChunkBase64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| {
            serde::de::Error::custom("terminal chunk must be bounded standard padded base64")
        })
    }
}

impl JsonSchema for HelperTerminalChunkBase64 {
    fn schema_name() -> String {
        "HelperTerminalChunkBase64".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        bounded_string_schema(
            0,
            MAX_UNSAFE_LOCAL_TERMINAL_BASE64_BYTES as u32,
            "^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$",
            "Bounded standard padded base64 terminal bytes.",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalWriteStdin {
    pub request_id: u64,
    pub offset: u64,
    pub chunk_base64: HelperTerminalChunkBase64,
    #[serde(default)]
    pub eof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalReadOutput {
    pub request_id: u64,
    pub stream: TerminalStream,
    pub cursor: u64,
    #[schemars(range(min = 1, max = 65536))]
    pub max_len: u64,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    #[schemars(range(max = 1000))]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalResize {
    pub request_id: u64,
    pub control_sequence: u64,
    #[schemars(range(min = 1, max = 65535))]
    pub rows: u32,
    #[schemars(range(min = 1, max = 65535))]
    pub cols: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalWait {
    pub request_id: u64,
    #[serde(default)]
    #[schemars(range(max = 1000))]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalControl {
    pub request_id: u64,
    pub control_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "op",
    content = "args",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum HelperTerminalRequest {
    WriteStdin(HelperTerminalWriteStdin),
    ReadOutput(HelperTerminalReadOutput),
    Resize(HelperTerminalResize),
    Wait(HelperTerminalWait),
    CloseStdin(HelperTerminalControl),
    CloseAttachment(HelperTerminalControl),
}

impl HelperTerminalRequest {
    pub fn request_id(&self) -> u64 {
        match self {
            Self::WriteStdin(request) => request.request_id,
            Self::ReadOutput(request) => request.request_id,
            Self::Resize(request) => request.request_id,
            Self::Wait(request) => request.request_id,
            Self::CloseStdin(request) | Self::CloseAttachment(request) => request.request_id,
        }
    }

    pub fn validate_bounds(&self) -> Result<(), HelperFailureCode> {
        match self {
            Self::ReadOutput(request)
                if request.max_len > MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES
                    || request.timeout_ms > MAX_UNSAFE_LOCAL_TERMINAL_WAIT_TIMEOUT_MS =>
            {
                Err(HelperFailureCode::InvalidRequest)
            }
            Self::Resize(request)
                if !terminal_size_valid(TerminalSize {
                    rows: request.rows,
                    cols: request.cols,
                }) =>
            {
                Err(HelperFailureCode::InvalidTerminalSize)
            }
            Self::Wait(request)
                if request.timeout_ms > MAX_UNSAFE_LOCAL_TERMINAL_WAIT_TIMEOUT_MS =>
            {
                Err(HelperFailureCode::InvalidRequest)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalReadOutputResult {
    pub data_base64: HelperTerminalChunkBase64,
    pub next_cursor: u64,
    #[serde(default)]
    pub eof: bool,
    #[serde(default)]
    pub dropped_bytes: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub timed_out: bool,
}

impl fmt::Debug for HelperTerminalReadOutputResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperTerminalReadOutputResult")
            .field("data_base64_len", &self.data_base64.as_str().len())
            .field("next_cursor", &self.next_cursor)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
}

impl TryFrom<TerminalReadOutputChunk> for HelperTerminalReadOutputResult {
    type Error = HelperFailureCode;

    fn try_from(value: TerminalReadOutputChunk) -> Result<Self, Self::Error> {
        Ok(Self {
            data_base64: HelperTerminalChunkBase64::new(value.data_base64)?,
            next_cursor: value.next_offset,
            eof: value.eof,
            dropped_bytes: value.dropped_bytes,
            truncated: value.truncated,
            timed_out: value.timed_out,
        })
    }
}

impl From<HelperTerminalReadOutputResult> for TerminalReadOutputChunk {
    fn from(value: HelperTerminalReadOutputResult) -> Self {
        Self {
            data_base64: value.data_base64.0,
            next_offset: value.next_cursor,
            eof: value.eof,
            dropped_bytes: value.dropped_bytes,
            truncated: value.truncated,
            timed_out: value.timed_out,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalOperationResult<T> {
    pub request_id: u64,
    pub result: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalControlResponse<T> {
    pub request_id: u64,
    pub control_sequence: u64,
    pub result: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalAttachmentClosed {
    pub detached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<ShellCloseCause>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalRejected {
    pub request_id: u64,
    pub code: HelperFailureCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "op",
    content = "result",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum HelperTerminalResponse {
    WriteStdin(HelperTerminalOperationResult<TerminalWriteStdinResult>),
    ReadOutput(HelperTerminalOperationResult<HelperTerminalReadOutputResult>),
    Resize(HelperTerminalControlResponse<TerminalControlResult>),
    Wait(HelperTerminalOperationResult<TerminalWaitResult>),
    CloseStdin(HelperTerminalControlResponse<TerminalCloseResult>),
    CloseAttachment(HelperTerminalControlResponse<HelperTerminalAttachmentClosed>),
    Rejected(HelperTerminalRejected),
}

impl HelperTerminalResponse {
    pub fn request_id(&self) -> u64 {
        match self {
            Self::WriteStdin(response) => response.request_id,
            Self::ReadOutput(response) => response.request_id,
            Self::Resize(response) => response.request_id,
            Self::Wait(response) => response.request_id,
            Self::CloseStdin(response) => response.request_id,
            Self::CloseAttachment(response) => response.request_id,
            Self::Rejected(response) => response.request_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum DaemonToUnsafeLocalHelper {
    HelloAccepted(HelperHelloAccepted),
    Heartbeat(HelperHeartbeat),
    Launch(HelperLaunchRequest),
    Shell(HelperShellRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum UnsafeLocalHelperToDaemon {
    Hello(HelperHello),
    Snapshot(HelperSnapshot),
    Heartbeat(HelperHeartbeat),
    Operation(HelperOperationResult),
    TerminalReady(HelperTerminalReady),
    Shell(HelperShellResponse),
    Rejected(HelperOperationRejected),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalHelperWireSchema {
    pub protocol_version: u32,
    pub terminal_protocol_version: u32,
    pub daemon_to_helper: DaemonToUnsafeLocalHelper,
    pub helper_to_daemon: UnsafeLocalHelperToDaemon,
    pub terminal_request: HelperTerminalRequest,
    pub terminal_response: HelperTerminalResponse,
}

fn terminal_size_valid(size: TerminalSize) -> bool {
    (1..=MAX_UNSAFE_LOCAL_TERMINAL_ROWS).contains(&size.rows)
        && (1..=MAX_UNSAFE_LOCAL_TERMINAL_COLS).contains(&size.cols)
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct BoundedTerminalSizeSchema {
    #[schemars(range(min = 1, max = 65535))]
    rows: u32,
    #[schemars(range(min = 1, max = 65535))]
    cols: u32,
}

fn bounded_terminal_size_schema(r#gen: &mut SchemaGenerator) -> Schema {
    BoundedTerminalSizeSchema::json_schema(r#gen)
}

fn decoded_base64_len(value: &str) -> Option<usize> {
    if !value.len().is_multiple_of(4) {
        return None;
    }
    let padding = value.bytes().rev().take_while(|byte| *byte == b'=').count();
    if padding > 2 {
        return None;
    }
    value
        .len()
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)
}

fn valid_base64_chunk(value: &str) -> bool {
    if value.len() > MAX_UNSAFE_LOCAL_TERMINAL_BASE64_BYTES {
        return false;
    }
    let padding_start = value.find('=').unwrap_or(value.len());
    if value[..padding_start]
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/')))
        || value[padding_start..].bytes().any(|byte| byte != b'=')
    {
        return false;
    }
    decoded_base64_len(value)
        .is_some_and(|len| len <= MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES as usize)
}

fn bounded_string_schema(
    min_length: u32,
    max_length: u32,
    pattern: &str,
    description: &str,
) -> Schema {
    let mut object = SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        string: Some(Box::new(StringValidation {
            min_length: Some(min_length),
            max_length: Some(max_length),
            pattern: Some(pattern.to_owned()),
        })),
        ..Default::default()
    };
    object.metadata = Some(Box::new(Metadata {
        description: Some(description.to_owned()),
        ..Default::default()
    }));
    Schema::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_wire::{ShellKillResult, ShellListEntry, ShellSessionState};
    use serde::de::DeserializeOwned;

    fn workload() -> WorkloadIdentity {
        serde_json::from_value(serde_json::json!({
            "workloadId": "tools",
            "realmId": "host",
            "realmPath": ["host"],
            "canonicalTarget": "tools.host.d2b"
        }))
        .unwrap()
    }

    fn operation(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    fn shell_name(value: &str) -> ShellName {
        ShellName::new(value).unwrap()
    }

    fn round_trip<T>(value: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + fmt::Debug,
    {
        let encoded = serde_json::to_vec(value).unwrap();
        let decoded: T = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(&decoded, value);
    }

    fn terminal_round_trip<T>(value: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + fmt::Debug,
    {
        let frame = encode_unsafe_local_terminal_frame(value).unwrap();
        let decoded: T = decode_unsafe_local_terminal_frame(&frame).unwrap();
        assert_eq!(&decoded, value);
    }

    #[test]
    fn management_request_variants_round_trip_and_correlate() {
        let requests = [
            HelperShellRequest::List {
                request_id: 1,
                operation_id: operation("op-list"),
                workload: workload(),
            },
            HelperShellRequest::Attach {
                request_id: 2,
                operation_id: operation("op-attach"),
                workload: workload(),
                name: Some(shell_name("primary")),
                force: true,
                initial_terminal_size: TerminalSize { rows: 24, cols: 80 },
            },
            HelperShellRequest::Detach {
                request_id: 3,
                operation_id: operation("op-detach"),
                workload: workload(),
                name: shell_name("primary"),
            },
            HelperShellRequest::Kill {
                request_id: 4,
                operation_id: operation("op-kill"),
                workload: workload(),
                name: shell_name("primary"),
            },
        ];

        for (index, request) in requests.iter().enumerate() {
            round_trip(request);
            let encoded = serde_json::to_string(request).unwrap();
            assert!(!encoded.contains("request_id"));
            assert!(!encoded.contains("operation_id"));
            assert!(!encoded.contains("initial_terminal_size"));
            assert_eq!(request.request_id(), index as u64 + 1);
            assert_eq!(
                request.operation_id().as_str(),
                ["op-list", "op-attach", "op-detach", "op-kill"][index]
            );
            assert_eq!(request.validate_bounds(), Ok(()));
        }
    }

    #[test]
    fn management_response_variants_round_trip_and_correlate() {
        let responses = [
            HelperShellResponse::List(HelperShellListResponse {
                request_id: 1,
                operation_id: operation("op-list"),
                result: ShellListResult {
                    default_name: shell_name("primary"),
                    sessions: vec![ShellListEntry {
                        name: shell_name("primary"),
                        state: ShellSessionState::Detached,
                        attached: false,
                        is_default: true,
                    }],
                },
            }),
            HelperShellResponse::Detach(HelperShellDetachResponse {
                request_id: 2,
                operation_id: operation("op-detach"),
                result: ShellDetachResult {
                    resolved_name: shell_name("primary"),
                    detached: true,
                    cause: Some(ShellCloseCause::EvictedByAdminDetach),
                },
            }),
            HelperShellResponse::Kill(HelperShellKillResponse {
                request_id: 3,
                operation_id: operation("op-kill"),
                result: ShellKillResult {
                    name: shell_name("primary"),
                    killed: true,
                    state: ShellSessionState::Killed,
                },
            }),
        ];

        for (index, response) in responses.iter().enumerate() {
            round_trip(response);
            assert_eq!(response.request_id(), index as u64 + 1);
            assert_eq!(
                response.operation_id().as_str(),
                ["op-list", "op-detach", "op-kill"][index]
            );
        }
    }

    #[test]
    fn terminal_request_variants_round_trip_and_correlate() {
        let requests = [
            HelperTerminalRequest::WriteStdin(HelperTerminalWriteStdin {
                request_id: 1,
                offset: 0,
                chunk_base64: HelperTerminalChunkBase64::new("aGVsbG8=").unwrap(),
                eof: false,
            }),
            HelperTerminalRequest::ReadOutput(HelperTerminalReadOutput {
                request_id: 2,
                stream: TerminalStream::Stdout,
                cursor: 0,
                max_len: 4096,
                wait: true,
                timeout_ms: 250,
            }),
            HelperTerminalRequest::Resize(HelperTerminalResize {
                request_id: 3,
                control_sequence: 7,
                rows: 40,
                cols: 120,
            }),
            HelperTerminalRequest::Wait(HelperTerminalWait {
                request_id: 4,
                timeout_ms: 500,
            }),
            HelperTerminalRequest::CloseStdin(HelperTerminalControl {
                request_id: 5,
                control_sequence: 8,
            }),
            HelperTerminalRequest::CloseAttachment(HelperTerminalControl {
                request_id: 6,
                control_sequence: 9,
            }),
        ];

        for (index, request) in requests.iter().enumerate() {
            round_trip(request);
            terminal_round_trip(request);
            assert_eq!(request.request_id(), index as u64 + 1);
            assert_eq!(request.validate_bounds(), Ok(()));
            let encoded = serde_json::to_vec(request).unwrap();
            assert!(encoded.len() <= MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE);
            assert!(!encoded.windows(11).any(|window| window == b"\"session\":"));
        }
    }

    #[test]
    fn terminal_response_variants_round_trip_and_correlate() {
        let responses = [
            HelperTerminalResponse::WriteStdin(HelperTerminalOperationResult {
                request_id: 1,
                result: TerminalWriteStdinResult {
                    accepted_len: 5,
                    next_offset: 5,
                    backpressured: false,
                    stdin_closed: false,
                },
            }),
            HelperTerminalResponse::ReadOutput(HelperTerminalOperationResult {
                request_id: 2,
                result: HelperTerminalReadOutputResult {
                    data_base64: HelperTerminalChunkBase64::new("aGVsbG8=").unwrap(),
                    next_cursor: 5,
                    eof: false,
                    dropped_bytes: 0,
                    truncated: false,
                    timed_out: false,
                },
            }),
            HelperTerminalResponse::Resize(HelperTerminalControlResponse {
                request_id: 3,
                control_sequence: 7,
                result: TerminalControlResult { delivered: true },
            }),
            HelperTerminalResponse::Wait(HelperTerminalOperationResult {
                request_id: 4,
                result: TerminalWaitResult {
                    running: true,
                    terminal_status: None,
                },
            }),
            HelperTerminalResponse::CloseStdin(HelperTerminalControlResponse {
                request_id: 5,
                control_sequence: 8,
                result: TerminalCloseResult { stdin_closed: true },
            }),
            HelperTerminalResponse::CloseAttachment(HelperTerminalControlResponse {
                request_id: 6,
                control_sequence: 9,
                result: HelperTerminalAttachmentClosed {
                    detached: true,
                    cause: Some(ShellCloseCause::ClientDetach),
                },
            }),
            HelperTerminalResponse::Rejected(HelperTerminalRejected {
                request_id: 7,
                code: HelperFailureCode::TerminalOffsetMismatch,
            }),
        ];

        for (index, response) in responses.iter().enumerate() {
            round_trip(response);
            terminal_round_trip(response);
            assert_eq!(response.request_id(), index as u64 + 1);
        }

        let shared = TerminalReadOutputChunk {
            data_base64: "aGVsbG8=".to_owned(),
            next_offset: 5,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        };
        let helper = HelperTerminalReadOutputResult::try_from(shared.clone()).unwrap();
        assert_eq!(TerminalReadOutputChunk::from(helper), shared);
    }

    #[test]
    fn helper_frames_reject_unknown_and_forbidden_fields() {
        let hello = r#"{
          "type":"hello",
          "payload":{"protocolVersion":2,"generation":1,"features":[],"uid":1000}
        }"#;
        assert!(serde_json::from_str::<UnsafeLocalHelperToDaemon>(hello).is_err());

        let shell = serde_json::json!({
            "op": "list",
            "args": {
                "requestId": 1,
                "operationId": "op-list",
                "workload": serde_json::to_value(workload()).unwrap(),
                "cwd": "/forbidden"
            }
        });
        assert!(serde_json::from_value::<HelperShellRequest>(shell).is_err());

        let terminal = serde_json::json!({
            "op": "wait",
            "args": {"requestId": 1, "timeoutMs": 1, "session": "forbidden"}
        });
        assert!(serde_json::from_value::<HelperTerminalRequest>(terminal).is_err());

        let response = serde_json::json!({
            "op": "rejected",
            "result": {
                "requestId": 1,
                "code": "terminal-closed",
                "message": "forbidden"
            }
        });
        assert!(serde_json::from_value::<HelperTerminalResponse>(response).is_err());
    }

    #[test]
    fn terminal_and_geometry_bounds_are_closed() {
        assert_eq!(MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES, EXEC_MAX_CHUNK_BYTES);
        assert_eq!(MAX_UNSAFE_LOCAL_TERMINAL_OUTPUT_RING_BYTES, 8 * 1024 * 1024);
        assert_eq!(UNSAFE_LOCAL_TERMINAL_LENGTH_PREFIX_BYTES, 4);

        let maximum = format!(
            "{}==",
            "A".repeat(MAX_UNSAFE_LOCAL_TERMINAL_BASE64_BYTES - 2)
        );
        assert!(HelperTerminalChunkBase64::new(maximum).is_ok());
        let oversized = "A".repeat(MAX_UNSAFE_LOCAL_TERMINAL_BASE64_BYTES + 4);
        assert!(HelperTerminalChunkBase64::new(oversized).is_err());
        assert!(HelperTerminalChunkBase64::new("not padded").is_err());

        let oversized_read = HelperTerminalRequest::ReadOutput(HelperTerminalReadOutput {
            request_id: 1,
            stream: TerminalStream::Stdout,
            cursor: 0,
            max_len: MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES + 1,
            wait: true,
            timeout_ms: MAX_UNSAFE_LOCAL_TERMINAL_WAIT_TIMEOUT_MS + 1,
        });
        assert_eq!(
            oversized_read.validate_bounds(),
            Err(HelperFailureCode::InvalidRequest)
        );

        let invalid_resize = HelperTerminalRequest::Resize(HelperTerminalResize {
            request_id: 1,
            control_sequence: 1,
            rows: 0,
            cols: 80,
        });
        assert_eq!(
            invalid_resize.validate_bounds(),
            Err(HelperFailureCode::InvalidTerminalSize)
        );

        assert_eq!(
            decode_unsafe_local_terminal_frame::<HelperTerminalRequest>(&[0, 0, 0]),
            Err(HelperFailureCode::InvalidRequest)
        );
        let oversized_prefix = ((MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE + 1) as u32).to_le_bytes();
        assert_eq!(
            decode_unsafe_local_terminal_frame::<HelperTerminalRequest>(&oversized_prefix),
            Err(HelperFailureCode::InvalidRequest)
        );
    }

    #[test]
    fn debug_redacts_names_terminal_bytes_and_supervisor_identity() {
        let shell_canary = "private-shell-name-canary";
        let request = HelperShellRequest::Attach {
            request_id: 1,
            operation_id: operation("op-1"),
            workload: workload(),
            name: Some(shell_name(shell_canary)),
            force: true,
            initial_terminal_size: TerminalSize { rows: 24, cols: 80 },
        };
        assert!(!format!("{request:?}").contains(shell_canary));

        let bytes_canary = "cHJpdmF0ZS10ZXJtaW5hbC1jYW5hcnk=";
        let output = HelperTerminalReadOutputResult {
            data_base64: HelperTerminalChunkBase64::new(bytes_canary).unwrap(),
            next_cursor: 24,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        };
        assert!(!format!("{output:?}").contains(bytes_canary));

        let supervisor_canary = "private-supervisor-canary";
        let snapshot = HelperPersistentShellSnapshot {
            name: shell_name(shell_canary),
            state: ShellSessionState::Detached,
            attached: false,
            supervisor_id: HelperSupervisorId::new(supervisor_canary).unwrap(),
        };
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains(shell_canary));
        assert!(!debug.contains(supervisor_canary));
        assert!(HelperSupervisorId::new("x".repeat(MAX_HELPER_SUPERVISOR_ID_BYTES + 1)).is_err());
    }

    #[test]
    fn generated_shell_and_terminal_schema_excludes_authority_fields() {
        let schema = serde_json::to_value(schemars::schema_for!(UnsafeLocalHelperWireSchema))
            .expect("schema serializes");
        let definitions = schema["definitions"].as_object().unwrap();
        let selected = definitions
            .iter()
            .filter(|(name, _)| {
                name.starts_with("HelperShell")
                    || name.starts_with("HelperTerminal")
                    || *name == "HelperPersistentShellSnapshot"
            })
            .map(|(_, definition)| definition)
            .collect::<Vec<_>>();
        let selected = serde_json::to_string(&selected).unwrap();
        for forbidden in [
            "\"uid\"",
            "\"argv\"",
            "\"environment\"",
            "\"cwd\"",
            "\"path\"",
            "\"transcript\"",
            "\"pid\"",
            "\"unitName\"",
            "\"compositor\"",
            "\"session\"",
        ] {
            assert!(
                !selected.contains(forbidden),
                "private shell/terminal schema contains forbidden field {forbidden}"
            );
        }
    }

    #[test]
    fn helper_v1_is_rejected_while_terminal_v1_is_preserved() {
        assert_eq!(UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION, 2);
        assert!(!unsafe_local_helper_protocol_supported(1));
        assert!(unsafe_local_helper_protocol_supported(2));
        assert_eq!(UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION, 1);
    }

    #[test]
    fn terminal_ready_freezes_single_connected_stream_transport() {
        assert_eq!(UNSAFE_LOCAL_TERMINAL_FD_COUNT, 1);
        assert_eq!(HELPER_SOCKET_BUFFER_REQUEST_BYTES, MAX_HELPER_FRAME_SIZE);
        assert_eq!(
            MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES,
            MAX_HELPER_FRAME_SIZE * 2
        );
        let ready = HelperTerminalReady {
            request_id: 1,
            operation_id: operation("op-1"),
            terminal_protocol_version: UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION,
            transport: HelperTerminalTransport::ConnectedUnixStream,
            scope: ScopeIdentity {
                invocation_id: "opaque".to_owned(),
                kind: HelperScopeKind::PersistentShell,
            },
            result: HelperShellAttachResult {
                resolved_name: shell_name("primary"),
                state: ShellSessionState::Attached,
                force_evicted: false,
            },
        };
        round_trip(&ready);
        let json = serde_json::to_string(&ready).unwrap();
        assert!(json.contains("\"transport\":\"connected-unix-stream\""));
        let invalid = json.replace("connected-unix-stream", "unix-datagram");
        assert!(serde_json::from_str::<HelperTerminalReady>(&invalid).is_err());
    }
}
