use crate::{
    daemon_audit,
    exec_session::{ComponentSessionExecClient, ExecOpError, ProcessOpError},
    terminal_session::{OutputStreamSel, TerminalBackend},
    typed_error::{ComponentSessionShellErrorKind, TypedError},
};
use d2b_contracts_control::{public_wire, terminal_wire as tw};
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
    backend: Arc<dyn ShellBackend>,
    pub attach: public_wire::ShellAttachResult,
    pub target: String,
    pub operation_digest: Option<String>,
    pub initial_control_sequence: u64,
}

impl EstablishedShell {
    pub fn new(
        backend: Arc<dyn ShellBackend>,
        attach: public_wire::ShellAttachResult,
        target: String,
        operation_digest: Option<String>,
        initial_control_sequence: u64,
    ) -> Self {
        Self {
            backend,
            attach,
            target,
            operation_digest,
            initial_control_sequence,
        }
    }

    /// Delegate a terminal operation to the attached backend.
    pub fn handle_op(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
        op: ShellTerminalOp,
    ) -> Result<Option<ShellTerminalResponse>, TypedError> {
        self.backend.handle_op(runtime, control_sequence, op)
    }

    /// Close the attached terminal stream.
    pub fn close_attachment(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError> {
        self.backend.close_attachment(runtime, control_sequence)
    }

    /// Cancel the attached terminal stream when its owner disappears.
    pub fn cancel_attachment(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError> {
        self.backend.cancel_attachment(runtime, control_sequence)
    }
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
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
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
                crate::typed_error::ComponentSessionShellErrorKind::StaleSession,
            ))
        }
    }
}

impl<D> ShellBackend for ComponentSessionShellBackend<D>
where
    D: d2b_session::ComponentSessionDriver + 'static,
{
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
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

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
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

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
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
    use crate::typed_error::ComponentSessionShellErrorKind as Kind;
    let kind = match error {
        ExecOpError::Transport => Kind::Transport,
        ExecOpError::Auth => Kind::Auth,
        ExecOpError::StaleSession => Kind::StaleSession,
        ExecOpError::Protocol => Kind::Protocol,
        ExecOpError::Timeout => Kind::Timeout,
        ExecOpError::OldGeneration | ExecOpError::Capability => Kind::Capability,
        ExecOpError::DetachedUnavailable => Kind::Capability,
        ExecOpError::Guest(ProcessOpError::ExecNotFound | ProcessOpError::ExecExpired) => {
            Kind::NotFound
        }
        ExecOpError::Guest(ProcessOpError::StdinBackpressure) => Kind::Capacity,
        ExecOpError::Guest(ProcessOpError::OffsetMismatch) => Kind::Protocol,
        ExecOpError::Guest(ProcessOpError::StdinClosed | ProcessOpError::StdinNotOpen) => {
            Kind::StaleSession
        }
        ExecOpError::Guest(ProcessOpError::ControlSeqMismatch) => Kind::StaleSession,
        ExecOpError::Guest(ProcessOpError::RateLimited) => Kind::Capacity,
        ExecOpError::Guest(ProcessOpError::MaxChunkExceeded | ProcessOpError::InvalidProgram) => {
            Kind::Protocol
        }
        ExecOpError::Guest(ProcessOpError::ExecAlreadyExited) => Kind::NotFound,
        ExecOpError::Guest(ProcessOpError::Protocol | ProcessOpError::Other) => Kind::GuestError,
    };
    shell_failed(kind)
}

impl fmt::Debug for EstablishedShell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EstablishedShell")
            .field("target", &self.target)
            .field("operation_digest", &self.operation_digest)
            .field("initial_control_sequence", &self.initial_control_sequence)
            .field("attach", &self.attach)
            .finish_non_exhaustive()
    }
}

pub fn best_effort_close(
    shell: &EstablishedShell,
    runtime: &tokio::runtime::Handle,
    control_sequence: &mut u64,
) -> daemon_audit::ShellAuditResult {
    match shell.close_attachment(runtime, control_sequence) {
        Ok(_) => daemon_audit::ShellAuditResult::Closed,
        Err(TypedError::ComponentSessionShellFailed {
            kind: crate::typed_error::ComponentSessionShellErrorKind::Timeout,
        }) => daemon_audit::ShellAuditResult::Timeout,
        Err(_) => daemon_audit::ShellAuditResult::Error,
    }
}

pub fn best_effort_cancel(
    shell: &EstablishedShell,
    runtime: &tokio::runtime::Handle,
    control_sequence: &mut u64,
) -> daemon_audit::ShellAuditResult {
    match shell.cancel_attachment(runtime, control_sequence) {
        Ok(_) => daemon_audit::ShellAuditResult::Closed,
        Err(TypedError::ComponentSessionShellFailed {
            kind: crate::typed_error::ComponentSessionShellErrorKind::Timeout,
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

pub fn shell_failed(kind: ComponentSessionShellErrorKind) -> TypedError {
    TypedError::ComponentSessionShellFailed { kind }
}

pub fn shell_transport_failed() -> TypedError {
    shell_failed(ComponentSessionShellErrorKind::Transport)
}

pub fn shell_capability_failed() -> TypedError {
    shell_failed(ComponentSessionShellErrorKind::Capability)
}

pub fn shell_protocol_failed() -> TypedError {
    shell_failed(ComponentSessionShellErrorKind::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_session_shell_errors_stay_in_the_closed_shell_vocabulary() {
        assert!(matches!(
            map_component_session_shell_error(ExecOpError::Auth),
            TypedError::ComponentSessionShellFailed {
                kind: ComponentSessionShellErrorKind::Auth
            }
        ));
        assert!(matches!(
            map_component_session_shell_error(ExecOpError::Guest(
                ProcessOpError::StdinBackpressure
            )),
            TypedError::ComponentSessionShellFailed {
                kind: ComponentSessionShellErrorKind::Capacity
            }
        ));
        assert!(matches!(
            map_component_session_shell_error(ExecOpError::Transport),
            TypedError::ComponentSessionShellFailed {
                kind: ComponentSessionShellErrorKind::Transport
            }
        ));
    }
}
