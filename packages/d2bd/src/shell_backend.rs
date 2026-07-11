use crate::{
    daemon_audit,
    terminal_session::{OutputStreamSel, TerminalBackend, TerminalKind},
    typed_error::{TypedError, UnsafeLocalShellErrorKind},
    unsafe_local_terminal::{UnsafeLocalTerminalClient, UnsafeLocalTerminalError},
};
use d2b_contracts::{
    public_wire::{self, ShellOp, ShellOpResponse},
    terminal_wire as tw,
};
use std::{fmt, sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellProvider {
    GuestControl,
    UnsafeLocal,
}

impl ShellProvider {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::GuestControl => "guest-control",
            Self::UnsafeLocal => "unsafe-local",
        }
    }
}

pub(crate) trait ShellBackend: Send + Sync {
    fn handle_op(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
        op: ShellOp,
    ) -> Result<Option<ShellOpResponse>, TypedError>;

    fn close_attachment(
        &self,
        runtime: &tokio::runtime::Handle,
        control_sequence: &mut u64,
    ) -> Result<public_wire::ShellDetachResult, TypedError>;
}

pub(crate) struct EstablishedShell {
    pub(crate) backend: Arc<dyn ShellBackend>,
    pub(crate) attach: public_wire::ShellAttachResult,
    pub(crate) target: String,
    pub(crate) provider: ShellProvider,
    pub(crate) operation_digest: Option<String>,
    pub(crate) initial_control_sequence: u64,
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

pub(crate) struct UnsafeLocalShellBackend {
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
    pub(crate) fn new(
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
        op: ShellOp,
    ) -> Result<Option<ShellOpResponse>, TypedError> {
        match op {
            ShellOp::WriteStdin(args) => {
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
                Ok(Some(ShellOpResponse::WriteStdin(
                    tw::TerminalWriteStdinResult {
                        accepted_len: result.accepted_len,
                        next_offset: result.next_offset,
                        backpressured: result.backpressured,
                        stdin_closed: result.stdin_closed,
                    },
                )))
            }
            ShellOp::ReadOutput(args) => {
                self.ensure_session(&args.session)?;
                let (timeout_ms, deadline) = shell_poll_timeout(args.timeout_ms, args.wait);
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
                Ok(Some(ShellOpResponse::ReadOutput(
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
            ShellOp::Resize(args) => {
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
                Ok(Some(ShellOpResponse::Resize(tw::TerminalControlResult {
                    delivered: true,
                })))
            }
            ShellOp::Wait(args) => {
                self.ensure_session(&args.session)?;
                let (timeout_ms, deadline) = shell_poll_timeout(args.timeout_ms, true);
                let result = runtime
                    .block_on(self.terminal.wait(timeout_ms, deadline))
                    .map_err(map_terminal_error)?;
                Ok(Some(ShellOpResponse::Wait(tw::TerminalWaitResult {
                    running: result.running,
                    terminal_status: result.terminal.map(|terminal| match terminal {
                        TerminalKind::Exited(code) => tw::TerminalStatus::Exited { code },
                        TerminalKind::Signaled(signal) => tw::TerminalStatus::Signaled { signal },
                        TerminalKind::Error(slug) => tw::TerminalStatus::Error {
                            slug: slug.to_owned(),
                        },
                    }),
                })))
            }
            ShellOp::CloseStdin(args) => {
                self.ensure_session(&args.session)?;
                *control_sequence = control_sequence.saturating_add(1);
                runtime
                    .block_on(
                        self.terminal
                            .close_stdin(*control_sequence, shell_operation_timeout()),
                    )
                    .map_err(map_terminal_error)?;
                Ok(Some(ShellOpResponse::CloseStdin(tw::TerminalCloseResult {
                    stdin_closed: true,
                })))
            }
            ShellOp::CloseAttach(args) => {
                self.ensure_session(&args.session)?;
                self.close_attachment(runtime, control_sequence)
                    .map(ShellOpResponse::CloseAttach)
                    .map(Some)
            }
            ShellOp::Attach(_) | ShellOp::List(_) | ShellOp::Detach(_) | ShellOp::Kill(_) => {
                Err(unsafe_shell_failed(UnsafeLocalShellErrorKind::Protocol))
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

pub(crate) fn best_effort_close(
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

fn shell_operation_timeout() -> Duration {
    Duration::from_secs(3)
}

fn shell_poll_timeout(requested_ms: u64, wait: bool) -> (u64, Duration) {
    if !wait {
        return (0, shell_operation_timeout());
    }
    let timeout_ms = requested_ms.min(1_000);
    (timeout_ms, Duration::from_millis(timeout_ms + 1_000))
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

pub(crate) fn map_helper_failure(
    code: d2b_contracts::unsafe_local_wire::HelperFailureCode,
) -> TypedError {
    use UnsafeLocalShellErrorKind as UnsafeKind;
    use d2b_contracts::unsafe_local_wire::HelperFailureCode as H;
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

pub(crate) fn unsafe_shell_failed(kind: UnsafeLocalShellErrorKind) -> TypedError {
    TypedError::UnsafeLocalShellFailed { kind }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::unsafe_local_wire::{
        HelperTerminalAttachmentClosed, HelperTerminalControlResponse,
        HelperTerminalOperationResult, HelperTerminalRequest, HelperTerminalResponse,
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
        d2b_contracts::unsafe_local_wire::decode_unsafe_local_terminal_frame(&frame).unwrap()
    }

    fn send_response(peer: &mut UnixStream, response: HelperTerminalResponse) {
        peer.write_all(
            &d2b_contracts::unsafe_local_wire::encode_unsafe_local_terminal_frame(&response)
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
                ShellOp::Wait(tw::TerminalWait {
                    session: "wrong-handle".to_owned(),
                    timeout_ms: 0,
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
    fn unsafe_backend_supports_wait_and_close_detach_without_kill() {
        let (backend, mut peer) = backend_pair();
        let backend = Arc::new(backend);
        let wait_backend = Arc::clone(&backend);
        let wait = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            wait_backend.handle_op(
                runtime.handle(),
                &mut 0,
                ShellOp::Wait(tw::TerminalWait {
                    session: "shell-public-handle".to_owned(),
                    timeout_ms: 100,
                }),
            )
        });
        let wait_request = read_request(&mut peer);
        let request_id = wait_request.request_id();
        send_response(
            &mut peer,
            HelperTerminalResponse::Wait(HelperTerminalOperationResult {
                request_id,
                result: tw::TerminalWaitResult {
                    running: true,
                    terminal_status: None,
                },
            }),
        );
        assert!(matches!(
            wait.join().unwrap().unwrap(),
            Some(ShellOpResponse::Wait(tw::TerminalWaitResult {
                running: true,
                ..
            }))
        ));

        let close_backend = Arc::clone(&backend);
        let close = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            close_backend.handle_op(
                runtime.handle(),
                &mut 0,
                ShellOp::CloseAttach(public_wire::ShellCloseAttachArgs {
                    session: "shell-public-handle".to_owned(),
                }),
            )
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
            Some(ShellOpResponse::CloseAttach(
                public_wire::ShellDetachResult {
                    detached: true,
                    cause: Some(public_wire::ShellCloseCause::ClientDetach),
                    ..
                }
            ))
        ));
    }
}
