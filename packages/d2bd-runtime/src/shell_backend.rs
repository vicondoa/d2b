use crate::{
    daemon_audit,
    exec_session::is_unspecified,
    exec_session::{ComponentSessionExecClient, ExecOpError, GuestOpError},
    guest_control_health::GuestControlHealthError,
    terminal_session::{OutputStreamSel, TerminalBackend},
    typed_error::{GuestControlShellErrorKind, TypedError, UnsafeLocalShellErrorKind},
    unsafe_local_terminal::{UnsafeLocalTerminalClient, UnsafeLocalTerminalError},
};
use d2b_contracts_control::{guest_proto as pb, public_wire, terminal_wire as tw};
use protobuf::EnumOrUnknown;
use std::{fmt, sync::Arc, time::Duration};

pub const SHELL_MANAGEMENT_TIMEOUT: Duration = Duration::from_secs(12);
pub const SHELL_POLL_CAP: Duration = Duration::from_secs(30);
pub const SHELL_POLL_SLACK: Duration = Duration::from_secs(2);

pub enum ShellTerminalOp {
    WriteStdin(tw::TerminalWriteStdin),
    ReadOutput(tw::TerminalReadOutput),
    Resize(tw::TerminalResize),
}

#[derive(Debug)]
pub enum ShellTerminalResponse {
    WriteStdin(tw::TerminalWriteStdinResult),
    ReadOutput(tw::TerminalReadOutputChunk),
    Delivered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellProvider {
    /// ComponentSession guest provider or its legacy characterization.
    GuestControl,
    /// Unsafe-local host provider.
    UnsafeLocal,
}

impl ShellProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::GuestControl => "guest-control",
            Self::UnsafeLocal => "unsafe-local",
        }
    }
}

pub trait ShellBackend: Send + Sync {
    fn handle_op(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
        op: ShellTerminalOp,
    ) -> Result<Option<ShellTerminalResponse>, TypedError>;

    fn close_attachment(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError>;

    /// Reset the named stream when its owning public connection disappears.
    /// Backends without a distinct reset operation may close normally.
    fn cancel_attachment(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError> {
        self.close_attachment(runtime, control_sequence)
    }
}

pub struct EstablishedShell {
    pub backend: Arc<dyn ShellBackend>,
    pub attach: public_wire::ShellAttachResult,
    pub target: String,
    pub provider: ShellProvider,
    pub operation_digest: Option<String>,
    pub initial_control_sequence: u64,
}

/// Persistent-shell backend over a ComponentSession named stream.
///
/// Shell lifecycle is still authorized by the ShellSession resource and its
/// Provider controller. This adapter only translates terminal operations to
/// the already-admitted stream; it has no process-spawn or broker authority.
pub struct ComponentSessionShellBackend<D> {
    public_session: String,
    resolved_name: public_wire::ShellName,
    client: ComponentSessionExecClient<D>,
}

impl<D> fmt::Debug for ComponentSessionShellBackend<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComponentSessionShellBackend")
            .field("public_session", &"<redacted>")
            .field("resolved_name", &"<redacted>")
            .field("client", &self.client)
            .finish()
    }
}

impl<D> ComponentSessionShellBackend<D>
where
    D: d2b_session::ComponentSessionDriver + 'static,
{
    /// Open the fixed terminal named stream after ShellSession admission.
    pub async fn open(
        driver: D,
        stream_number: u16,
        public_session: String,
        resolved_name: public_wire::ShellName,
    ) -> Result<Self, TypedError> {
        let client = ComponentSessionExecClient::open(
            driver,
            stream_number,
            d2b_contracts_zone_session::v3::component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
            d2b_contracts_zone_session::v3::component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
        )
        .await
        .map_err(map_component_session_shell_error)?;
        Ok(Self {
            public_session,
            resolved_name,
            client,
        })
    }

    /// Reset the stream after owner cancellation or a disconnected peer.
    pub fn cancel(&self, runtime: &tokio::runtime::Handle) -> Result<(), TypedError> {
        runtime
            .block_on(self.client.cancel())
            .map_err(map_component_session_shell_error)
    }

    fn ensure_session(&self, session: &str) -> Result<(), TypedError> {
        if session == self.public_session {
            Ok(())
        } else {
            Err(shell_failed(
                crate::typed_error::GuestControlShellErrorKind::StaleSession,
            ))
        }
    }
}

impl<D> ShellBackend for ComponentSessionShellBackend<D>
where
    D: d2b_session::ComponentSessionDriver + 'static,
{
    fn handle_op(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
        op: ShellTerminalOp,
    ) -> Result<Option<ShellTerminalResponse>, TypedError> {
        match op {
            ShellTerminalOp::WriteStdin(args) => {
                self.ensure_session(&args.session)?;
                let data = d2b_core::base64_codec::decode(&args.chunk_base64)
                    .map_err(|_| shell_protocol_failed())?;
                let result = if data.is_empty() && args.eof {
                    runtime
                        .block_on(
                            self.client
                                .close_stdin(args.offset, SHELL_MANAGEMENT_TIMEOUT),
                        )
                        .map(|()| crate::terminal_session::WriteStdinOutcome {
                            accepted_len: 0,
                            next_offset: args.offset,
                            backpressured: false,
                            stdin_closed: true,
                        })
                } else {
                    runtime.block_on(self.client.write_stdin(
                        args.offset,
                        data,
                        args.eof,
                        SHELL_MANAGEMENT_TIMEOUT,
                    ))
                }
                .map_err(map_component_session_shell_error)?;
                Ok(Some(ShellTerminalResponse::WriteStdin(
                    tw::TerminalWriteStdinResult {
                        accepted_len: result.accepted_len,
                        next_offset: result.next_offset,
                        backpressured: result.backpressured,
                        stdin_closed: result.stdin_closed,
                    },
                )))
            }
            ShellTerminalOp::ReadOutput(args) => {
                self.ensure_session(&args.session)?;
                if args.stream != tw::TerminalStream::Stdout {
                    return Err(shell_protocol_failed());
                }
                let (timeout_ms, deadline) = backend_shell_poll_timeout(args.timeout_ms, args.wait);
                let result = runtime
                    .block_on(self.client.read_output(
                        OutputStreamSel::Stdout,
                        args.offset,
                        args.max_len,
                        args.wait,
                        timeout_ms,
                        deadline,
                    ))
                    .map_err(map_component_session_shell_error)?;
                Ok(Some(ShellTerminalResponse::ReadOutput(
                    tw::TerminalReadOutputChunk {
                        data_base64: d2b_core::base64_codec::encode(&result.data),
                        next_offset: result.next_offset,
                        eof: result.eof,
                        dropped_bytes: result.dropped_bytes,
                        truncated: result.truncated,
                        timed_out: result.timed_out,
                    },
                )))
            }
            ShellTerminalOp::Resize(args) => {
                self.ensure_session(&args.session)?;
                *control_sequence = control_sequence.saturating_add(1);
                runtime
                    .block_on(self.client.resize(
                        *control_sequence,
                        args.rows,
                        args.cols,
                        SHELL_MANAGEMENT_TIMEOUT,
                    ))
                    .map_err(map_component_session_shell_error)?;
                Ok(Some(ShellTerminalResponse::Delivered))
            }
        }
    }

    fn close_attachment(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError> {
        *control_sequence = control_sequence.saturating_add(1);
        runtime
            .block_on(self.client.close_stream())
            .map_err(map_component_session_shell_error)?;
        Ok(public_wire::ShellDetachResult {
            resolved_name: self.resolved_name.clone(),
            detached: true,
            cause: Some(public_wire::ShellCloseCause::ClientDetach),
        })
    }

    fn cancel_attachment(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError> {
        *control_sequence = control_sequence.saturating_add(1);
        runtime
            .block_on(self.client.cancel())
            .map_err(map_component_session_shell_error)?;
        Ok(public_wire::ShellDetachResult {
            resolved_name: self.resolved_name.clone(),
            detached: true,
            cause: Some(public_wire::ShellCloseCause::ClientDetach),
        })
    }
}

fn map_component_session_shell_error(error: ExecOpError) -> TypedError {
    use crate::typed_error::GuestControlShellErrorKind as Kind;
    let kind = match error {
        ExecOpError::Transport => Kind::Transport,
        ExecOpError::Auth => Kind::Auth,
        ExecOpError::StaleSession => Kind::StaleSession,
        ExecOpError::Protocol => Kind::Protocol,
        ExecOpError::Timeout => Kind::Timeout,
        ExecOpError::OldGeneration | ExecOpError::Capability => Kind::Capability,
        ExecOpError::DetachedUnavailable => Kind::Capability,
        ExecOpError::Guest(GuestOpError::ExecNotFound | GuestOpError::ExecExpired) => {
            Kind::NotFound
        }
        ExecOpError::Guest(GuestOpError::StdinBackpressure) => Kind::Capacity,
        ExecOpError::Guest(GuestOpError::OffsetMismatch) => Kind::Protocol,
        ExecOpError::Guest(GuestOpError::StdinClosed | GuestOpError::StdinNotOpen) => {
            Kind::StaleSession
        }
        ExecOpError::Guest(GuestOpError::ControlSeqMismatch) => Kind::StaleSession,
        ExecOpError::Guest(GuestOpError::RateLimited) => Kind::Capacity,
        ExecOpError::Guest(GuestOpError::MaxChunkExceeded | GuestOpError::InvalidProgram) => {
            Kind::Protocol
        }
        ExecOpError::Guest(GuestOpError::ExecAlreadyExited) => Kind::NotFound,
        ExecOpError::Guest(GuestOpError::Protocol | GuestOpError::Other) => Kind::GuestError,
    };
    shell_failed(kind)
}

impl fmt::Debug for EstablishedShell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EstablishedShell")
            .field("target", &self.target)
            .field("provider", &self.provider)
            .field("operation_digest", &self.operation_digest)
            .field("initial_control_sequence", &self.initial_control_sequence)
            .field("attach", &self.attach)
            .finish_non_exhaustive()
    }
}

pub struct UnsafeLocalShellBackend {
    public_session: String,
    resolved_name: public_wire::ShellName,
    terminal: UnsafeLocalTerminalClient,
}

impl fmt::Debug for UnsafeLocalShellBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnsafeLocalShellBackend")
            .field("public_session", &"<redacted>")
            .field("resolved_name", &"<redacted>")
            .field("terminal", &self.terminal)
            .finish()
    }
}

impl UnsafeLocalShellBackend {
    pub fn new(
        public_session: String,
        resolved_name: public_wire::ShellName,
        terminal: UnsafeLocalTerminalClient,
    ) -> Self {
        Self {
            public_session,
            resolved_name,
            terminal,
        }
    }

    fn ensure_session(&self, session: &str) -> Result<(), TypedError> {
        if session == self.public_session {
            Ok(())
        } else {
            Err(unsafe_shell_failed(UnsafeLocalShellErrorKind::StaleSession))
        }
    }
}

impl ShellBackend for UnsafeLocalShellBackend {
    fn handle_op(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
        op: ShellTerminalOp,
    ) -> Result<Option<ShellTerminalResponse>, TypedError> {
        match op {
            ShellTerminalOp::WriteStdin(args) => {
                self.ensure_session(&args.session)?;
                let data = d2b_core::base64_codec::decode(&args.chunk_base64)
                    .map_err(|_| unsafe_shell_failed(UnsafeLocalShellErrorKind::Protocol))?;
                let result = runtime
                    .block_on(self.terminal.write_stdin(
                        args.offset,
                        data,
                        args.eof,
                        shell_operation_timeout(),
                    ))
                    .map_err(map_terminal_error)?;
                Ok(Some(ShellTerminalResponse::WriteStdin(
                    tw::TerminalWriteStdinResult {
                        accepted_len: result.accepted_len,
                        next_offset: result.next_offset,
                        backpressured: result.backpressured,
                        stdin_closed: result.stdin_closed,
                    },
                )))
            }
            ShellTerminalOp::ReadOutput(args) => {
                self.ensure_session(&args.session)?;
                let (timeout_ms, deadline) = backend_shell_poll_timeout(args.timeout_ms, args.wait);
                let result = runtime
                    .block_on(self.terminal.read_output(
                        match args.stream {
                            tw::TerminalStream::Stdout => OutputStreamSel::Stdout,
                            tw::TerminalStream::Stderr => OutputStreamSel::Stderr,
                        },
                        args.offset,
                        args.max_len,
                        args.wait,
                        timeout_ms,
                        deadline,
                    ))
                    .map_err(map_terminal_error)?;
                Ok(Some(ShellTerminalResponse::ReadOutput(
                    tw::TerminalReadOutputChunk {
                        data_base64: d2b_core::base64_codec::encode(&result.data),
                        next_offset: result.next_offset,
                        eof: result.eof,
                        dropped_bytes: result.dropped_bytes,
                        truncated: result.truncated,
                        timed_out: result.timed_out,
                    },
                )))
            }
            ShellTerminalOp::Resize(args) => {
                self.ensure_session(&args.session)?;
                *control_sequence = control_sequence.saturating_add(1);
                runtime
                    .block_on(self.terminal.resize(
                        *control_sequence,
                        args.rows,
                        args.cols,
                        shell_operation_timeout(),
                    ))
                    .map_err(map_terminal_error)?;
                Ok(Some(ShellTerminalResponse::Delivered))
            }
        }
    }

    fn close_attachment(
        &self,
        _runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError> {
        *control_sequence = control_sequence.saturating_add(1);
        let result = self
            .terminal
            .close_attachment(*control_sequence, shell_operation_timeout())
            .map_err(map_terminal_error)?;
        Ok(public_wire::ShellDetachResult {
            resolved_name: self.resolved_name.clone(),
            detached: result.detached,
            cause: result.cause,
        })
    }
}

pub fn best_effort_close(
    backend: &dyn ShellBackend,
    runtime: &tokio::runtime::Handle,
    control_sequence: &mut u64,
) -> daemon_audit::ShellAuditResult {
    match backend.close_attachment(runtime, control_sequence) {
        Ok(_) => daemon_audit::ShellAuditResult::Closed,
        Err(TypedError::UnsafeLocalShellFailed {
            kind: UnsafeLocalShellErrorKind::Timeout,
        })
        | Err(TypedError::GuestControlShellFailed {
            kind: crate::typed_error::GuestControlShellErrorKind::Timeout,
        }) => daemon_audit::ShellAuditResult::Timeout,
        Err(_) => daemon_audit::ShellAuditResult::Error,
    }
}

pub fn best_effort_cancel(
    backend: &dyn ShellBackend,
    runtime: &tokio::runtime::Handle,
    control_sequence: &mut u64,
) -> daemon_audit::ShellAuditResult {
    match backend.cancel_attachment(runtime, control_sequence) {
        Ok(_) => daemon_audit::ShellAuditResult::Closed,
        Err(TypedError::UnsafeLocalShellFailed {
            kind: UnsafeLocalShellErrorKind::Timeout,
        })
        | Err(TypedError::GuestControlShellFailed {
            kind: crate::typed_error::GuestControlShellErrorKind::Timeout,
        }) => daemon_audit::ShellAuditResult::Timeout,
        Err(_) => daemon_audit::ShellAuditResult::Error,
    }
}

fn shell_operation_timeout() -> Duration {
    Duration::from_secs(3)
}

fn backend_shell_poll_timeout(requested_ms: u64, wait: bool) -> (u64, Duration) {
    if !wait {
        return (0, shell_operation_timeout());
    }
    let timeout_ms = requested_ms.min(1_000);
    (timeout_ms, Duration::from_millis(timeout_ms + 1_000))
}

pub fn shell_poll_timeout(args_timeout_ms: u64, wait: bool) -> (u64, Duration) {
    if !wait {
        return (0, SHELL_MANAGEMENT_TIMEOUT);
    }
    let cap_ms = SHELL_POLL_CAP.as_millis().min(u64::MAX as u128) as u64;
    let timeout_ms = args_timeout_ms.min(cap_ms);
    (
        timeout_ms,
        Duration::from_millis(timeout_ms) + SHELL_POLL_SLACK,
    )
}

pub fn shell_failed(kind: GuestControlShellErrorKind) -> TypedError {
    TypedError::GuestControlShellFailed { kind }
}

pub fn map_shell_health_error(error: GuestControlHealthError) -> TypedError {
    use GuestControlHealthError as E;
    use GuestControlShellErrorKind as K;
    match error {
        E::TransportIo | E::Ttrpc | E::Signer => shell_failed(K::Transport),
        E::Timeout => shell_failed(K::Timeout),
        E::AuthFailed => shell_failed(K::Auth),
        E::StaleSession => shell_failed(K::StaleSession),
        E::Protocol => shell_failed(K::Protocol),
    }
}

pub fn shell_transport_failed() -> TypedError {
    shell_failed(GuestControlShellErrorKind::Transport)
}

pub fn shell_capability_failed() -> TypedError {
    shell_failed(GuestControlShellErrorKind::Capability)
}

pub fn shell_protocol_failed() -> TypedError {
    shell_failed(GuestControlShellErrorKind::Protocol)
}

pub fn map_shell_guest_error(error: &pb::GuestControlError) -> TypedError {
    use GuestControlShellErrorKind as K;
    use pb::GuestControlErrorKind as G;
    let kind = match error.kind.enum_value() {
        Ok(G::GUEST_CONTROL_ERROR_KIND_AUTH_FAILED) => K::Auth,
        Ok(G::GUEST_CONTROL_ERROR_KIND_STALE_SESSION) => K::StaleSession,
        Ok(G::GUEST_CONTROL_ERROR_KIND_TRANSPORT_UNREACHABLE) => K::Transport,
        Ok(G::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR)
        | Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_INVALID_NAME) => K::Protocol,
        Ok(G::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED)
        | Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE)
        | Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_DAEMON_EPOCH_MISMATCH) => K::Capability,
        Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_CAPACITY_EXCEEDED)
        | Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_ATTACH_CAPACITY_EXCEEDED) => K::Capacity,
        Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_ALREADY_ATTACHED) => K::AlreadyAttached,
        Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_NOT_FOUND) => K::NotFound,
        Ok(G::GUEST_CONTROL_ERROR_KIND_SHELL_OUTPUT_GAP) => K::OutputGap,
        _ => K::GuestError,
    };
    shell_failed(kind)
}

pub fn guest_advertises_capability(
    capabilities: &[EnumOrUnknown<pb::GuestCapability>],
    cap: pb::GuestCapability,
) -> bool {
    capabilities
        .iter()
        .filter_map(|value| value.enum_value().ok())
        .any(|value| value == cap)
}

pub fn shell_error_to_typed(error: Option<&pb::GuestControlError>) -> Result<(), TypedError> {
    if let Some(error) = error
        && !is_unspecified(error.kind)
    {
        return Err(map_shell_guest_error(error));
    }
    Ok(())
}

pub fn shell_name_from_guest(value: String) -> Result<public_wire::ShellName, TypedError> {
    public_wire::ShellName::new(value).map_err(|_| shell_protocol_failed())
}

pub fn map_shell_state(state: EnumOrUnknown<pb::ShellState>) -> public_wire::ShellSessionState {
    match state
        .enum_value()
        .unwrap_or(pb::ShellState::SHELL_STATE_UNSPECIFIED)
    {
        pb::ShellState::SHELL_STATE_ATTACHED => public_wire::ShellSessionState::Attached,
        pb::ShellState::SHELL_STATE_DETACHED => public_wire::ShellSessionState::Detached,
        pb::ShellState::SHELL_STATE_KILLED => public_wire::ShellSessionState::Killed,
        pb::ShellState::SHELL_STATE_POOL_UNAVAILABLE => {
            public_wire::ShellSessionState::PoolUnavailable
        }
        pb::ShellState::SHELL_STATE_FEATURE_DISABLED => {
            public_wire::ShellSessionState::FeatureDisabled
        }
        pb::ShellState::SHELL_STATE_OUTPUT_GAP => public_wire::ShellSessionState::OutputGap,
        pb::ShellState::SHELL_STATE_UNSPECIFIED => public_wire::ShellSessionState::Detached,
    }
}

pub fn map_shell_close_cause(
    cause: EnumOrUnknown<pb::ShellCloseCause>,
) -> Option<public_wire::ShellCloseCause> {
    match cause
        .enum_value()
        .unwrap_or(pb::ShellCloseCause::SHELL_CLOSE_CAUSE_UNSPECIFIED)
    {
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_CLIENT_DETACH => {
            Some(public_wire::ShellCloseCause::ClientDetach)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_EVICTED_BY_FORCE => {
            Some(public_wire::ShellCloseCause::EvictedByForce)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_EVICTED_BY_ADMIN_DETACH => {
            Some(public_wire::ShellCloseCause::EvictedByAdminDetach)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_KILLED_BY_ADMIN => {
            Some(public_wire::ShellCloseCause::KilledByAdmin)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_POOL_UNAVAILABLE => {
            Some(public_wire::ShellCloseCause::PoolUnavailable)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_OUTPUT_GAP => {
            Some(public_wire::ShellCloseCause::OutputGap)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_UNSPECIFIED => None,
    }
}

pub fn map_shell_list_response(
    response: pb::ShellListResponse,
) -> Result<public_wire::ShellListResult, TypedError> {
    Ok(public_wire::ShellListResult {
        default_name: shell_name_from_guest(response.default_name)?,
        sessions: response
            .sessions
            .into_iter()
            .map(|entry| {
                Ok(public_wire::ShellListEntry {
                    name: shell_name_from_guest(entry.name)?,
                    state: map_shell_state(entry.state),
                    attached: entry.attached,
                    is_default: entry.is_default,
                })
            })
            .collect::<Result<Vec<_>, TypedError>>()?,
    })
}

pub fn map_shell_attach_response(
    response: pb::ShellAttachResponse,
) -> Result<public_wire::ShellAttachResult, TypedError> {
    shell_error_to_typed(response.error.as_ref())?;
    Ok(public_wire::ShellAttachResult {
        session: response.session_id.ok_or_else(shell_protocol_failed)?,
        resolved_name: shell_name_from_guest(response.resolved_name)?,
        state: map_shell_state(response.state),
        force_evicted: response.force_evicted,
    })
}

pub fn map_shell_detach_response(
    response: pb::ShellDetachResponse,
) -> Result<public_wire::ShellDetachResult, TypedError> {
    shell_error_to_typed(response.error.as_ref())?;
    Ok(public_wire::ShellDetachResult {
        resolved_name: shell_name_from_guest(response.resolved_name)?,
        detached: response.detached,
        cause: map_shell_close_cause(response.cause),
    })
}

pub fn map_shell_kill_response(
    response: pb::ShellKillResponse,
) -> Result<public_wire::ShellKillResult, TypedError> {
    shell_error_to_typed(response.error.as_ref())?;
    Ok(public_wire::ShellKillResult {
        name: shell_name_from_guest(response.name)?,
        killed: response.killed,
        state: map_shell_state(response.state),
    })
}

fn map_terminal_error(error: UnsafeLocalTerminalError) -> TypedError {
    use UnsafeLocalShellErrorKind as UnsafeKind;
    let kind = match error {
        UnsafeLocalTerminalError::Bounds
        | UnsafeLocalTerminalError::Protocol
        | UnsafeLocalTerminalError::ResponseMismatch => UnsafeKind::Protocol,
        UnsafeLocalTerminalError::Capacity => UnsafeKind::QueueFull,
        UnsafeLocalTerminalError::Timeout => UnsafeKind::Timeout,
        UnsafeLocalTerminalError::Closed => UnsafeKind::TerminalClosed,
        UnsafeLocalTerminalError::OutputGap => UnsafeKind::OutputGap,
        UnsafeLocalTerminalError::OffsetMismatch => UnsafeKind::OffsetMismatch,
        UnsafeLocalTerminalError::InvalidSize => UnsafeKind::InvalidSize,
        UnsafeLocalTerminalError::Unsupported => UnsafeKind::Protocol,
        UnsafeLocalTerminalError::Rejected(code) => return map_helper_failure(code),
    };
    unsafe_shell_failed(kind)
}

pub fn map_helper_failure(
    code: d2b_contracts_control::unsafe_local_wire::HelperFailureCode,
) -> TypedError {
    use UnsafeLocalShellErrorKind as UnsafeKind;
    use d2b_contracts_control::unsafe_local_wire::HelperFailureCode as H;
    let kind = match code {
        H::InvalidRequest => UnsafeKind::Protocol,
        H::OperationIdConflict => UnsafeKind::OperationConflict,
        H::QueueFull => UnsafeKind::QueueFull,
        H::Timeout => UnsafeKind::Timeout,
        H::UserManagerUnavailable => UnsafeKind::UserManagerUnavailable,
        H::EnvironmentInvalid => UnsafeKind::EnvironmentInvalid,
        H::ExecutableUnavailable => UnsafeKind::ExecutableUnavailable,
        H::ScopeCreateFailed => UnsafeKind::ScopeCreateFailed,
        H::ScopeIdentityMismatch => UnsafeKind::ScopeIdentityMismatch,
        H::GraphicalSessionInactive => UnsafeKind::GraphicalSessionInactive,
        H::WaylandUnavailable => UnsafeKind::WaylandUnavailable,
        H::ProxyUnavailable => UnsafeKind::ProxyUnavailable,
        H::FirstClientTimeout => UnsafeKind::FirstClientTimeout,
        H::ShellUnavailable => UnsafeKind::ShellUnavailable,
        H::ShellNotFound => UnsafeKind::NotFound,
        H::ShellAlreadyAttached => UnsafeKind::AlreadyAttached,
        H::TerminalOutputGap => UnsafeKind::OutputGap,
        H::TerminalOffsetMismatch => UnsafeKind::OffsetMismatch,
        H::TerminalClosed => UnsafeKind::TerminalClosed,
        H::InvalidTerminalSize => UnsafeKind::InvalidSize,
        H::Internal => UnsafeKind::Internal,
    };
    unsafe_shell_failed(kind)
}

pub fn unsafe_shell_failed(kind: UnsafeLocalShellErrorKind) -> TypedError {
    TypedError::UnsafeLocalShellFailed { kind }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_control::unsafe_local_wire::{
        HelperTerminalAttachmentClosed, HelperTerminalControlResponse, HelperTerminalRequest,
        HelperTerminalResponse,
    };
    use std::{
        io::{Read, Write},
        os::unix::net::UnixStream,
    };

    fn backend_pair() -> (UnsafeLocalShellBackend, UnixStream) {
        let (client, peer) = UnixStream::pair().unwrap();
        (
            UnsafeLocalShellBackend::new(
                "shell-public-handle".to_owned(),
                public_wire::ShellName::new("primary").unwrap(),
                UnsafeLocalTerminalClient::new(client.into()).unwrap(),
            ),
            peer,
        )
    }

    fn read_request(peer: &mut UnixStream) -> HelperTerminalRequest {
        let mut prefix = [0u8; 4];
        peer.read_exact(&mut prefix).unwrap();
        let length = u32::from_le_bytes(prefix) as usize;
        let mut frame = Vec::from(prefix);
        frame.resize(length + 4, 0);
        peer.read_exact(&mut frame[4..]).unwrap();
        d2b_contracts_control::unsafe_local_wire::decode_unsafe_local_terminal_frame(&frame)
            .unwrap()
    }

    fn send_response(peer: &mut UnixStream, response: HelperTerminalResponse) {
        peer.write_all(
            &d2b_contracts_control::unsafe_local_wire::encode_unsafe_local_terminal_frame(
                &response,
            )
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn unsafe_backend_rejects_stale_public_handle_before_terminal_io() {
        let (backend, _peer) = backend_pair();
        let debug = format!("{backend:?}");
        assert!(!debug.contains("shell-public-handle"));
        assert!(!debug.contains("primary"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = backend
            .handle_op(
                runtime.handle(),
                &mut 0,
                ShellTerminalOp::WriteStdin(tw::TerminalWriteStdin {
                    session: "wrong-handle".to_owned(),
                    offset: 0,
                    chunk_base64: String::new(),
                    eof: false,
                }),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            TypedError::UnsafeLocalShellFailed {
                kind: UnsafeLocalShellErrorKind::StaleSession
            }
        ));
    }

    #[test]
    fn unsafe_backend_closes_attachment_without_kill() {
        let (backend, mut peer) = backend_pair();
        let backend = Arc::new(backend);
        let close_backend = Arc::clone(&backend);
        let close = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            close_backend.close_attachment(runtime.handle(), &mut 0)
        });
        let close_request = read_request(&mut peer);
        let HelperTerminalRequest::CloseAttachment(control) = close_request else {
            panic!("expected close attachment");
        };
        send_response(
            &mut peer,
            HelperTerminalResponse::CloseAttachment(HelperTerminalControlResponse {
                request_id: control.request_id,
                control_sequence: control.control_sequence,
                result: HelperTerminalAttachmentClosed {
                    detached: true,
                    cause: Some(public_wire::ShellCloseCause::ClientDetach),
                },
            }),
        );
        assert!(matches!(
            close.join().unwrap().unwrap(),
            public_wire::ShellDetachResult {
                detached: true,
                cause: Some(public_wire::ShellCloseCause::ClientDetach),
                ..
            }
        ));
    }

    #[test]
    fn component_session_shell_errors_stay_in_the_closed_shell_vocabulary() {
        assert!(matches!(
            map_component_session_shell_error(ExecOpError::Auth),
            TypedError::GuestControlShellFailed {
                kind: GuestControlShellErrorKind::Auth
            }
        ));
        assert!(matches!(
            map_component_session_shell_error(ExecOpError::Guest(GuestOpError::StdinBackpressure)),
            TypedError::GuestControlShellFailed {
                kind: GuestControlShellErrorKind::Capacity
            }
        ));
        assert!(matches!(
            map_component_session_shell_error(ExecOpError::Transport),
            TypedError::GuestControlShellFailed {
                kind: GuestControlShellErrorKind::Transport
            }
        ));
    }
}
