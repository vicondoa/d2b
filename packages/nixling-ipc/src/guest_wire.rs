//! Guest-control ttRPC/protobuf schema DTOs.
//!
//! These Rust DTOs are the schema oracle for the guest-control protobuf
//! surface selected by ADR 0026. They intentionally model the message
//! contract, not a JSON transport: implementations generate protobuf/ttRPC
//! bindings from the matching `.proto` surface and keep these DTOs aligned
//! through the schema drift gate.

use crate::types::VmId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GUEST_CONTROL_SCHEMA_VERSION: &str = "v2";
pub const GUEST_CONTROL_PROTOCOL_VERSION: u32 = 1;
pub const GUEST_CONTROL_VSOCK_PORT: u32 = 14_318;
pub const TTRPC_FRAME_CAP_BYTES: u64 = 4 * 1024 * 1024;
pub const DEFAULT_MAX_CHUNK_BYTES: u64 = 64 * 1024;
pub const HARD_MAX_CHUNK_BYTES: u64 = 1024 * 1024;

macro_rules! guest_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            JsonSchema,
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
    };
}

guest_id! {
    /// Guest-control request idempotency key.
    RequestId
}

guest_id! {
    /// Guest-control exec session id.
    ExecId
}

guest_id! {
    /// Bounded guest capability name.
    GuestCapabilityName
}

guest_id! {
    /// Bounded guest health subsystem name.
    GuestSubsystemName
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestControlSchema {
    pub schema_version: String,
    pub protocol_version: u32,
    pub transport: GuestTransportSchema,
    pub limits: GuestEffectiveLimits,
    pub hello: HelloRequest,
    pub hello_ok: HelloResponse,
    pub health: HealthResponse,
    pub capabilities: CapabilitiesResponse,
    pub exec_create: ExecCreateRequest,
    pub exec_created: ExecCreateResponse,
    pub exec_inspect: ExecInspectRequest,
    pub exec_inspected: ExecInspectResponse,
    pub exec_wait: ExecWaitRequest,
    pub exec_waited: ExecWaitResponse,
    pub exec_logs: ExecLogsRequest,
    pub exec_log_chunk: ExecLogsResponse,
    pub write_stdin: WriteStdinRequest,
    pub write_stdin_result: WriteStdinResponse,
    pub read_output: ReadOutputRequest,
    pub output_chunk: ReadOutputResponse,
    pub close_stdin: CloseStdinRequest,
    pub close_stdin_result: CloseStdinResponse,
    pub tty_win_resize: TtyWinResizeRequest,
    pub exec_signal: ExecSignalRequest,
    pub exec_cancel: ExecCancelRequest,
    pub error: GuestControlError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestTransportSchema {
    pub transport: GuestTransportKind,
    pub vsock_port: u32,
    pub ttrpc_frame_cap_bytes: u64,
    pub host_connect: GuestHostConnectShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestTransportKind {
    VirtioVsockTtrpc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestHostConnectShape {
    pub request_line: String,
    pub ok_ack: String,
    pub ack_value: GuestConnectAckValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestConnectAckValue {
    OpaqueLocalPort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestRequestMetadata {
    pub vm_id: VmId,
    pub request_id: RequestId,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestExecRequestMetadata {
    pub common: GuestRequestMetadata,
    pub exec_id: ExecId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloRequest {
    pub metadata: GuestRequestMetadata,
    pub host_nonce: String,
    pub transcript_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloResponse {
    pub guest_nonce: String,
    pub guest_boot_id: String,
    pub protocol_version: u32,
    pub capabilities_hash: String,
    pub health: HealthResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitiesResponse {
    pub protocol_version: u32,
    pub capabilities: Vec<GuestCapabilityName>,
    pub limits: GuestEffectiveLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HealthResponse {
    pub state: HealthState,
    pub reason: HealthReason,
    pub remediation: HealthRemediation,
    pub protocol_version: u32,
    pub capabilities: Vec<GuestCapabilityName>,
    pub degraded_subsystems: Vec<GuestSubsystemName>,
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
        use HealthReason as Reason;
        use HealthRemediation as Remediation;
        use HealthState as State;

        matches!(
            (self.state, self.reason, self.remediation),
            (State::Healthy, Reason::None, Remediation::None)
                | (
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
                )
                | (
                    State::UnavailableOldGeneration,
                    Reason::OldGeneration,
                    Remediation::UpgradeGuest | Remediation::RestartVm,
                )
                | (
                    State::ListenerAbsent,
                    Reason::ListenerAbsent,
                    Remediation::CheckGuestdService | Remediation::RestartVm,
                )
                | (
                    State::TransportUnreachable,
                    Reason::ConnectRefused
                        | Reason::ConnectTimeout
                        | Reason::EofBeforeAck
                        | Reason::MalformedAck
                        | Reason::AckTooLong
                        | Reason::TransportIo,
                    Remediation::Retry | Remediation::RestartVm | Remediation::CheckGuestdService,
                )
                | (
                    State::AuthFailed,
                    Reason::AuthTokenRejected,
                    Remediation::CheckAuthToken
                )
                | (
                    State::ProtocolMismatch,
                    Reason::ProtocolVersionUnsupported,
                    Remediation::UpgradeGuest,
                )
                | (
                    State::StaleSession,
                    Reason::SessionGenerationMismatch,
                    Remediation::Retry | Remediation::RestartVm,
                )
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestEffectiveLimits {
    pub max_chunk_bytes: u64,
    pub max_recv_message_bytes: u64,
    pub decoded_write_stdin_bytes_per_connection: u64,
    pub write_stdin_handlers_per_connection: u32,
    pub stdin_queue_chunks_per_exec: u32,
    pub stdout_live_buffer_bytes: u64,
    pub stderr_live_buffer_bytes: u64,
    pub detached_stdout_log_bytes: u64,
    pub detached_stderr_log_bytes: u64,
    pub long_poll_timeout_ms: u64,
    pub slow_consumer_grace_ms: u64,
    pub exec_sessions_per_vm: u32,
    pub attached_sessions_per_vm: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCreateRequest {
    pub metadata: GuestRequestMetadata,
    pub argv: Vec<String>,
    pub user: Option<String>,
    pub cwd: Option<String>,
    pub env: Vec<EnvVar>,
    pub tty: bool,
    pub stdin_open: bool,
    pub detached: bool,
    pub initial_terminal_size: Option<TerminalSize>,
    pub output_policy: OutputPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalSize {
    pub rows: u32,
    pub cols: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputPolicy {
    pub max_chunk_bytes: u64,
    pub max_stdout_log_bytes: u64,
    pub max_stderr_log_bytes: u64,
    pub slow_consumer_timeout_ms: u64,
    pub wait_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecInspectRequest {
    pub metadata: GuestExecRequestMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    pub truncated_for_retention: bool,
    pub last_control_seq: u64,
    pub state_generation: u64,
    pub error: Option<GuestControlError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWaitRequest {
    pub metadata: GuestExecRequestMetadata,
    pub timeout_ms: u64,
    pub known_state_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecLogsRequest {
    pub metadata: GuestExecRequestMetadata,
    pub stream: OutputStream,
    pub offset: u64,
    pub max_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecLogsResponse {
    pub stream: OutputStream,
    pub offset: u64,
    pub data: Vec<u8>,
    pub next_offset: u64,
    pub eof: bool,
    pub start_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteStdinRequest {
    pub metadata: GuestExecRequestMetadata,
    pub offset: u64,
    pub data: Vec<u8>,
    pub close_after: bool,
    pub client_deadline_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteStdinResponse {
    pub accepted_offset: u64,
    pub accepted_len: u64,
    pub next_offset: u64,
    pub stdin_state: StdinState,
    pub blocked_ms: u64,
    pub disposition: WriteDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadOutputRequest {
    pub metadata: GuestExecRequestMetadata,
    pub stream: OutputStream,
    pub offset: u64,
    pub max_len: u64,
    pub wait: bool,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadOutputResponse {
    pub stream: OutputStream,
    pub offset: u64,
    pub data: Vec<u8>,
    pub next_offset: u64,
    pub eof: bool,
    pub start_offset: u64,
    pub dropped_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseStdinRequest {
    pub metadata: GuestExecRequestMetadata,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseStdinResponse {
    pub stdin_state: StdinState,
    pub final_offset: u64,
    pub disposition: WriteDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TtyWinResizeRequest {
    pub metadata: GuestExecRequestMetadata,
    pub control_seq: u64,
    pub rows: u32,
    pub cols: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecSignalRequest {
    pub metadata: GuestExecRequestMetadata,
    pub control_seq: u64,
    pub signal: u32,
    pub target: SignalTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecCancelRequest {
    pub metadata: GuestExecRequestMetadata,
    pub control_seq: u64,
    pub reason: ExecCancelReason,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalStatus {
    pub exit_code: Option<i32>,
    pub signal: Option<u32>,
    pub status_code: Option<i32>,
    pub error: Option<GuestControlErrorKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SignalTarget {
    ForegroundProcessGroup,
    ProcessTree,
    RootProcess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecCancelReason {
    ClientDisconnect,
    UserRequested,
    SlowConsumer,
    ProtocolError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    OffsetExpired,
    OutputLost,
    TtyStderrUnavailable,
    ExecCapacityExceeded,
    ExecAttachCapacityExceeded,
    ReadWaitCapacityExceeded,
    WaitCapacityExceeded,
    RateLimited,
    StaleSession,
    GuestControlUnavailableOldGeneration,
    AuthFailed,
    TransportUnreachable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_mappings_are_closed() {
        let valid = HealthResponse {
            state: HealthState::TransportUnreachable,
            reason: HealthReason::MalformedAck,
            remediation: HealthRemediation::Retry,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities: Vec::new(),
            degraded_subsystems: Vec::new(),
        };
        assert!(valid.is_valid_mapping());

        let invalid = HealthResponse {
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
    }
}
