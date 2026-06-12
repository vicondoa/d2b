//! Guest-control ttRPC/protobuf schema DTOs.
//!
//! These Rust DTOs are the schema oracle for the guest-control protobuf
//! surface selected by ADR 0028. They intentionally model the message
//! contract, not a JSON transport: implementations generate protobuf/ttRPC
//! bindings from the matching `.proto` surface and keep these DTOs aligned
//! through the schema drift gate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GUEST_CONTROL_SCHEMA_VERSION: &str = "v2";
pub const GUEST_CONTROL_PROTOCOL_VERSION: u32 = 2;
pub const GUEST_CONTROL_VSOCK_PORT: u32 = 14_318;
pub const TTRPC_FRAME_CAP_BYTES: u64 = 4 * 1024 * 1024;
pub const DEFAULT_MAX_CHUNK_BYTES: u64 = 64 * 1024;
pub const HARD_MAX_CHUNK_BYTES: u64 = 1024 * 1024;

macro_rules! bounded_string {
    ($(#[$meta:meta])* $name:ident, $max:literal) => {
        $(#[$meta])*
        #[derive(
            Clone,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(
                _gen: &mut schemars::gen::SchemaGenerator,
            ) -> schemars::schema::Schema {
                schemars::schema::Schema::Object(schemars::schema::SchemaObject {
                    instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                        schemars::schema::InstanceType::String,
                    ))),
                    string: Some(Box::new(schemars::schema::StringValidation {
                        min_length: Some(1),
                        max_length: Some($max),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

macro_rules! bounded_bytes {
    ($(#[$meta:meta])* $name:ident, $min:literal, $max:literal) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Vec<u8>);

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(
                _gen: &mut schemars::gen::SchemaGenerator,
            ) -> schemars::schema::Schema {
                let item = schemars::schema::Schema::Object(schemars::schema::SchemaObject {
                    instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                        schemars::schema::InstanceType::Integer,
                    ))),
                    format: Some("uint8".to_owned()),
                    number: Some(Box::new(schemars::schema::NumberValidation {
                        minimum: Some(0.0),
                        maximum: Some(255.0),
                        ..Default::default()
                    })),
                    ..Default::default()
                });

                schemars::schema::Schema::Object(schemars::schema::SchemaObject {
                    instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                        schemars::schema::InstanceType::Array,
                    ))),
                    array: Some(Box::new(schemars::schema::ArrayValidation {
                        items: Some(schemars::schema::SingleOrVec::Single(Box::new(item))),
                        min_items: Some($min),
                        max_items: Some($max),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

bounded_bytes! {
    /// Stdin payload bytes.
    GuestStdinBytes, 1, 1048576
}

bounded_bytes! {
    /// Stdout/stderr payload bytes.
    GuestOutputBytes, 0, 1048576
}

bounded_string! {
    /// Guest-control schema version token.
    GuestSchemaVersion, 32
}

bounded_string! {
    /// Cloud Hypervisor CONNECT request line shape.
    GuestConnectRequestLine, 64
}

bounded_string! {
    /// Cloud Hypervisor OK acknowledgement line shape.
    GuestConnectAckLine, 64
}

bounded_string! {
    /// VM identity bound into guest-control auth transcripts.
    GuestVmId, 128
}

bounded_string! {
    /// Guest-control request idempotency key.
    RequestId, 128
}

bounded_string! {
    /// Guest-control exec session id.
    ExecId, 128
}

bounded_string! {
    /// Hex-encoded SHA-256 of a detached exec's argv (never the raw argv).
    ArgvSha256, 64
}

bounded_bytes! {
    /// Fixed-size raw challenge nonce bytes.
    GuestNonce, 32, 32
}

bounded_bytes! {
    /// Fixed-size HMAC-SHA256 proof tag bytes.
    GuestAuthTag, 32, 32
}

bounded_string! {
    /// Guest boot identity for stale-session detection.
    GuestBootId, 128
}

bounded_string! {
    /// Hash of the negotiated bounded capability set.
    CapabilitiesHash, 128
}

bounded_string! {
    /// One command argument.
    GuestArg, 4096
}

bounded_string! {
    /// Guest user selector.
    GuestUser, 128
}

bounded_string! {
    /// Guest working directory.
    GuestCwd, 4096
}

bounded_string! {
    /// Guest environment variable name.
    EnvKey, 128
}

bounded_string! {
    /// Guest environment variable value.
    EnvValue, 8192
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestCapability {
    Health,
    Capabilities,
    ExecAttached,
    ExecDetached,
    ExecTty,
    ExecLogs,
    TtyResize,
    Signals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestSubsystem {
    Guestd,
    Userd,
    Exec,
    LogStorage,
    Token,
    Vsock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HealthOrigin {
    GuestReported,
    HostSynthesized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestVsockDirection {
    HostToGuest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestIdentityBinding {
    VmIdCidPortAndTokenTranscript,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestControlSchema {
    pub schema_version: GuestSchemaVersion,
    pub protocol_version: u32,
    pub transport: GuestTransportSchema,
    pub limits: GuestEffectiveLimits,
    pub hello: HelloRequest,
    pub hello_ok: HelloResponse,
    pub authenticate: AuthenticateRequest,
    pub authenticated: AuthenticateResponse,
    pub health_request: HealthRequest,
    pub health: HealthResponse,
    pub capabilities_request: CapabilitiesRequest,
    pub capabilities: CapabilitiesResponse,
    pub exec_create: ExecCreateRequest,
    pub exec_created: ExecCreateResponse,
    pub exec_inspect: ExecInspectRequest,
    pub exec_inspected: ExecInspectResponse,
    pub exec_wait: ExecWaitRequest,
    pub exec_waited: ExecWaitResponse,
    pub exec_logs: ExecLogsRequest,
    pub exec_log_chunk: ExecLogsResponse,
    pub exec_list: ExecListRequest,
    pub exec_listed: ExecListResponse,
    pub write_stdin: WriteStdinRequest,
    pub write_stdin_result: WriteStdinResponse,
    pub read_output: ReadOutputRequest,
    pub output_chunk: ReadOutputResponse,
    pub close_stdin: CloseStdinRequest,
    pub close_stdin_result: CloseStdinResponse,
    pub tty_win_resize: TtyWinResizeRequest,
    pub exec_signal: ExecSignalRequest,
    pub exec_cancel: ExecCancelRequest,
    pub control_ack: ControlAck,
    pub error: GuestControlError,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestTransportSchema {
    pub transport: GuestTransportKind,
    pub direction: GuestVsockDirection,
    pub guest_control_vsock_port: u32,
    pub guest_to_host_observability_port: u32,
    pub reserved_side_channel_port: u32,
    pub identity_binding: GuestIdentityBinding,
    pub ttrpc_frame_cap_bytes: u64,
    pub host_connect: GuestHostConnectShape,
    pub readiness: GuestReadinessContract,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestTransportKind {
    VirtioVsockTtrpc,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestHostConnectShape {
    pub request_line: GuestConnectRequestLine,
    pub ok_ack: GuestConnectAckLine,
    pub ack_value: GuestConnectAckValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestConnectAckValue {
    OpaqueLocalPort,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestReadinessContract {
    pub socket_existence_is_readiness: bool,
    pub requires_connect_hello_auth_and_health: bool,
    pub pre_ttrpc_failures_are_host_synthesized: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestRequestMetadata {
    pub vm_id: GuestVmId,
    pub request_id: RequestId,
    pub protocol_version: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestExecRequestMetadata {
    pub common: GuestRequestMetadata,
    pub exec_id: ExecId,
    pub guest_boot_id: GuestBootId,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloRequest {
    pub metadata: GuestRequestMetadata,
    pub host_nonce: GuestNonce,
    pub transcript_version: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloResponse {
    pub guest_nonce: GuestNonce,
    pub guest_boot_id: GuestBootId,
    pub protocol_version: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthenticateRequest {
    pub metadata: GuestRequestMetadata,
    pub host_nonce: GuestNonce,
    pub guest_nonce: GuestNonce,
    pub guest_boot_id: GuestBootId,
    pub transcript_version: u32,
    pub host_auth_tag: GuestAuthTag,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthenticateResponse {
    pub guest_auth_tag: Option<GuestAuthTag>,
    pub capabilities_hash: Option<CapabilitiesHash>,
    pub health: Option<HealthResponse>,
    pub capabilities: Option<CapabilitiesResponse>,
    pub error: Option<GuestControlError>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitiesRequest {
    pub metadata: GuestRequestMetadata,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitiesResponse {
    pub protocol_version: u32,
    #[schemars(length(max = 32))]
    pub capabilities: Vec<GuestCapability>,
    pub limits: GuestEffectiveLimits,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HealthRequest {
    pub metadata: GuestRequestMetadata,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HealthResponse {
    pub origin: HealthOrigin,
    pub state: HealthState,
    pub reason: HealthReason,
    pub remediation: HealthRemediation,
    pub protocol_version: u32,
    #[schemars(length(max = 32))]
    pub capabilities: Vec<GuestCapability>,
    #[schemars(length(max = 16))]
    pub degraded_subsystems: Vec<GuestSubsystem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HealthState {
    Healthy,
    Degraded,
    UnavailableOldGeneration,
    ListenerAbsent,
    TransportUnreachable,
    AuthFailed,
    ProtocolMismatch,
    StaleSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HealthReason {
    None,
    OldGeneration,
    ListenerAbsent,
    ConnectRefused,
    ConnectTimeout,
    EofBeforeAck,
    MalformedAck,
    AckTooLong,
    TransportIo,
    AuthTokenRejected,
    ProtocolVersionUnsupported,
    SessionGenerationMismatch,
    ExecSubsystemUnavailable,
    LogStorageUnavailable,
    QuotaExceeded,
    RateLimited,
    InternalHealthCheckFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HealthRemediation {
    None,
    Retry,
    RestartVm,
    UpgradeGuest,
    CheckAuthToken,
    CheckGuestdService,
    ReduceLoad,
    InspectGuestLogs,
}

impl HealthResponse {
    pub fn is_valid_mapping(&self) -> bool {
        use HealthOrigin as Origin;
        use HealthReason as Reason;
        use HealthRemediation as Remediation;
        use HealthState as State;

        matches!(
            (self.origin, self.state, self.reason, self.remediation),
            (
                Origin::GuestReported,
                State::Healthy,
                Reason::None,
                Remediation::None
            ) | (
                Origin::GuestReported,
                State::Degraded,
                Reason::ExecSubsystemUnavailable
                    | Reason::LogStorageUnavailable
                    | Reason::QuotaExceeded
                    | Reason::RateLimited
                    | Reason::InternalHealthCheckFailed,
                Remediation::Retry
                    | Remediation::ReduceLoad
                    | Remediation::InspectGuestLogs
                    | Remediation::RestartVm,
            ) | (
                Origin::HostSynthesized,
                State::UnavailableOldGeneration,
                Reason::OldGeneration,
                Remediation::UpgradeGuest | Remediation::RestartVm,
            ) | (
                Origin::HostSynthesized,
                State::ListenerAbsent,
                Reason::ListenerAbsent,
                Remediation::CheckGuestdService | Remediation::RestartVm,
            ) | (
                Origin::HostSynthesized,
                State::TransportUnreachable,
                Reason::ConnectRefused
                    | Reason::ConnectTimeout
                    | Reason::EofBeforeAck
                    | Reason::MalformedAck
                    | Reason::AckTooLong
                    | Reason::TransportIo,
                Remediation::Retry | Remediation::RestartVm | Remediation::CheckGuestdService,
            ) | (
                Origin::HostSynthesized,
                State::AuthFailed,
                Reason::AuthTokenRejected,
                Remediation::CheckAuthToken
            ) | (
                Origin::HostSynthesized,
                State::ProtocolMismatch,
                Reason::ProtocolVersionUnsupported,
                Remediation::UpgradeGuest,
            ) | (
                Origin::HostSynthesized,
                State::StaleSession,
                Reason::SessionGenerationMismatch,
                Remediation::Retry | Remediation::RestartVm,
            )
        )
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestEffectiveLimits {
    #[schemars(range(min = 1, max = 1048576))]
    pub max_chunk_bytes: u64,
    #[schemars(range(min = 1, max = 4194304))]
    pub max_recv_message_bytes: u64,
    #[schemars(range(max = 16777216))]
    pub decoded_write_stdin_bytes_per_connection: u64,
    #[schemars(range(max = 4))]
    pub write_stdin_handlers_per_connection: u32,
    #[schemars(range(max = 1))]
    pub stdin_queue_chunks_per_exec: u32,
    #[schemars(range(max = 8388608))]
    pub stdout_live_buffer_bytes: u64,
    #[schemars(range(max = 8388608))]
    pub stderr_live_buffer_bytes: u64,
    #[schemars(range(max = 4194304))]
    pub detached_stdout_log_bytes: u64,
    #[schemars(range(max = 4194304))]
    pub detached_stderr_log_bytes: u64,
    #[schemars(range(max = 1000))]
    pub long_poll_timeout_ms: u64,
    #[schemars(range(max = 300000))]
    pub slow_consumer_grace_ms: u64,
    #[schemars(range(max = 256))]
    pub exec_sessions_per_vm: u32,
    #[schemars(range(max = 64))]
    pub attached_sessions_per_vm: u32,
    #[schemars(range(max = 512))]
    pub pending_read_output_waits_per_stream: u32,
    #[schemars(range(max = 512))]
    pub pending_exec_waits_per_vm: u32,
    #[schemars(range(max = 200))]
    pub rpc_rate_per_connection_per_second: u32,
    #[schemars(range(max = 1000))]
    pub rpc_rate_per_vm_burst: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCreateRequest {
    pub metadata: GuestRequestMetadata,
    #[schemars(length(min = 1, max = 128))]
    pub argv: Vec<GuestArg>,
    pub user: Option<GuestUser>,
    pub cwd: Option<GuestCwd>,
    #[schemars(length(max = 256))]
    pub env: Vec<EnvVar>,
    pub tty: bool,
    pub stdin_open: bool,
    pub detached: bool,
    pub initial_terminal_size: Option<TerminalSize>,
    pub output_policy: OutputPolicy,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvVar {
    pub key: EnvKey,
    pub value: EnvValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalSize {
    #[schemars(range(min = 1, max = 65535))]
    pub rows: u32,
    #[schemars(range(min = 1, max = 65535))]
    pub cols: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputPolicy {
    pub max_chunk_bytes: u64,
    pub max_stdout_log_bytes: u64,
    pub max_stderr_log_bytes: u64,
    pub slow_consumer_timeout_ms: u64,
    pub wait_timeout_ms: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCreateResponse {
    pub exec_id: Option<ExecId>,
    pub created_at_monotonic_ns: u64,
    pub control_seq: u64,
    pub stdout_cursor: u64,
    pub stderr_cursor: u64,
    pub effective_limits: GuestEffectiveLimits,
    pub state: ExecState,
    pub error: Option<GuestControlError>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecInspectRequest {
    pub metadata: GuestExecRequestMetadata,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecInspectResponse {
    pub state: ExecState,
    pub visible_terminal_status: Option<TerminalStatus>,
    pub stdin_state: StdinState,
    pub stdout_start_offset: u64,
    pub stdout_end_offset: u64,
    pub stderr_start_offset: u64,
    pub stderr_end_offset: u64,
    pub stdout_dropped_bytes: u64,
    pub stderr_dropped_bytes: u64,
    pub stdout_truncated_for_retention: bool,
    pub stderr_truncated_for_retention: bool,
    pub last_control_seq: u64,
    pub state_generation: u64,
    pub error: Option<GuestControlError>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWaitRequest {
    pub metadata: GuestExecRequestMetadata,
    pub timeout_ms: u64,
    pub known_state_generation: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWaitResponse {
    pub state: ExecState,
    pub visible_terminal_status: Option<TerminalStatus>,
    pub state_generation: u64,
    pub stdout_start_offset: u64,
    pub stdout_end_offset: u64,
    pub stderr_start_offset: u64,
    pub stderr_end_offset: u64,
    pub timed_out: bool,
    pub error: Option<GuestControlError>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecLogsRequest {
    pub metadata: GuestExecRequestMetadata,
    pub stream: OutputStream,
    pub offset: u64,
    #[schemars(range(min = 1, max = 1048576))]
    pub max_len: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecLogsResponse {
    pub stream: OutputStream,
    pub offset: u64,
    pub end_offset: u64,
    pub data: GuestOutputBytes,
    pub next_offset: u64,
    pub eof: bool,
    pub start_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub error: Option<GuestControlError>,
}

/// Read-only listing of detached execs visible to the same VM token on the
/// same guest boot. Carries no exec id, argv, or output; the boot id binds
/// the request to the current guest generation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecListRequest {
    pub metadata: GuestRequestMetadata,
    pub guest_boot_id: GuestBootId,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecListResponse {
    #[schemars(length(max = 32))]
    pub entries: Vec<ExecListEntry>,
    pub error: Option<GuestControlError>,
}

/// One detached exec record summary. Discloses only bounded, non-sensitive
/// metadata: the opaque id, slot, lifecycle state, create time, an argv
/// hash (never the raw argv), and retained-log accounting.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecListEntry {
    pub exec_id: ExecId,
    #[schemars(range(max = 31))]
    pub slot: u32,
    pub state: ExecState,
    pub create_time_unix: u64,
    pub argv_sha256: ArgvSha256,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub dropped_bytes: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteStdinRequest {
    pub metadata: GuestExecRequestMetadata,
    pub offset: u64,
    pub data: GuestStdinBytes,
    pub close_after: bool,
    pub client_deadline_ms: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteStdinResponse {
    pub accepted_offset: u64,
    pub accepted_len: u64,
    pub next_offset: u64,
    pub stdin_state: StdinState,
    pub blocked_ms: u64,
    pub disposition: WriteDisposition,
    pub error: Option<GuestControlError>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadOutputRequest {
    pub metadata: GuestExecRequestMetadata,
    pub stream: OutputStream,
    pub offset: u64,
    #[schemars(range(min = 1, max = 1048576))]
    pub max_len: u64,
    pub wait: bool,
    pub timeout_ms: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadOutputResponse {
    pub stream: OutputStream,
    pub offset: u64,
    pub end_offset: u64,
    pub data: GuestOutputBytes,
    pub next_offset: u64,
    pub eof: bool,
    pub start_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
    pub error: Option<GuestControlError>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseStdinRequest {
    pub metadata: GuestExecRequestMetadata,
    pub offset: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseStdinResponse {
    pub stdin_state: StdinState,
    pub final_offset: u64,
    pub disposition: WriteDisposition,
    pub error: Option<GuestControlError>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtyWinResizeRequest {
    pub metadata: GuestExecRequestMetadata,
    pub control_seq: u64,
    #[schemars(range(min = 1, max = 65535))]
    pub rows: u32,
    #[schemars(range(min = 1, max = 65535))]
    pub cols: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecSignalRequest {
    pub metadata: GuestExecRequestMetadata,
    pub control_seq: u64,
    pub signal: u32,
    pub target: SignalTarget,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCancelRequest {
    pub metadata: GuestExecRequestMetadata,
    pub control_seq: u64,
    pub reason: ExecCancelReason,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlAck {
    pub control_seq: u64,
    pub duplicate: bool,
    pub error: Option<GuestControlError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WriteDisposition {
    Accepted,
    Duplicate,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecState {
    Created,
    Running,
    Exited,
    Signaled,
    Cancelled,
    SlowConsumerCancelled,
    ProtocolError,
    LostGuestd,
    Reaped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StdinState {
    Open,
    Closing,
    Closed,
    ClosedByProcess,
    RejectedNotInteractive,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    tag = "outcome",
    deny_unknown_fields
)]
pub enum TerminalStatus {
    ExitCode {
        #[serde(rename = "exitCode")]
        exit_code: i32,
    },
    Signal {
        signal: u32,
    },
    StatusCode {
        #[serde(rename = "statusCode")]
        status_code: i32,
    },
    Error {
        error: GuestControlErrorKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SignalTarget {
    ForegroundProcessGroup,
    ProcessTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecCancelReason {
    ClientDisconnect,
    UserRequested,
    SlowConsumer,
    ProtocolError,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestControlError {
    pub kind: GuestControlErrorKind,
    pub remediation: HealthRemediation,
    pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestControlErrorKind {
    ProtocolError,
    MaxChunkExceeded,
    StdinBackpressure,
    StdinClosed,
    StdinNotOpen,
    StdinClosedByProcess,
    StdinOffsetMismatch,
    StdinByteBudgetExhausted,
    OffsetExpired,
    OffsetInFuture,
    OffsetExhausted,
    OutputLost,
    TtyStderrUnavailable,
    TtyRequired,
    ExecCapacityExceeded,
    ExecAttachCapacityExceeded,
    ExecNotFound,
    ExecAlreadyExited,
    GuestExecDisabled,
    GuestExecRootDenied,
    GuestExecUserDenied,
    CwdInvalid,
    CwdDenied,
    RetainedLogPathUnsafe,
    RetainedLogQuotaExceeded,
    ReadWaitCapacityExceeded,
    WaitCapacityExceeded,
    SupersededReadWait,
    RateLimited,
    RequestIdConflict,
    ControlSeqMismatch,
    SlowConsumerCancelled,
    StaleSession,
    GuestControlUnavailableOldGeneration,
    AuthFailed,
    TransportUnreachable,
    ExecExpired,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_mappings_are_closed() {
        let valid = HealthResponse {
            origin: HealthOrigin::HostSynthesized,
            state: HealthState::TransportUnreachable,
            reason: HealthReason::MalformedAck,
            remediation: HealthRemediation::Retry,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities: Vec::new(),
            degraded_subsystems: Vec::new(),
        };
        assert!(valid.is_valid_mapping());

        let invalid = HealthResponse {
            origin: HealthOrigin::GuestReported,
            state: HealthState::Healthy,
            reason: HealthReason::AuthTokenRejected,
            remediation: HealthRemediation::CheckAuthToken,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities: Vec::new(),
            degraded_subsystems: Vec::new(),
        };
        assert!(!invalid.is_valid_mapping());
    }

    #[test]
    fn enums_serialize_kebab_case() {
        assert_eq!(
            serde_json::to_string(&HealthState::UnavailableOldGeneration).unwrap(),
            "\"unavailable-old-generation\""
        );
        assert_eq!(
            serde_json::to_string(&GuestControlErrorKind::TtyStderrUnavailable).unwrap(),
            "\"tty-stderr-unavailable\""
        );
        assert_eq!(
            serde_json::to_string(&GuestControlErrorKind::ExecExpired).unwrap(),
            "\"exec-expired\""
        );
        assert_eq!(
            serde_json::to_string(&TerminalStatus::Signal { signal: 2 }).unwrap(),
            "{\"outcome\":\"signal\",\"signal\":2}"
        );
    }
}
