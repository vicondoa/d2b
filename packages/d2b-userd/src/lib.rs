#![doc = "Per-user interaction and credential agent primitives."]
#![forbid(unsafe_code)]

use d2b_contracts::guest_wire::{
    ExecId, GuestBootId, GuestControlErrorKind, OutputStream, TerminalSize,
};

pub mod services;
pub use services::user::runtime;

pub use services::user::{
    AuthenticatedUser, ClosedOutcome, NoopSecretMetrics, OwnerBinding, SecretMetricEvent,
    SecretMetricSink, UserSecretError,
};

pub const USERD_LISTENS_ON_VSOCK: bool = false;

pub fn userd_listens_on_vsock() -> bool {
    USERD_LISTENS_ON_VSOCK
}

#[derive(Clone, PartialEq, Eq)]
pub struct UserdConfig {
    pub socket_name: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct UserSessionIdentity {
    pub uid: u32,
    pub gid: u32,
    pub session_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct UserAttachRequest {
    pub exec_id: ExecId,
    pub guest_boot_id: GuestBootId,
    pub tty: bool,
    pub initial_size: Option<TerminalSize>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct UserOutputCursor {
    pub exec_id: ExecId,
    pub stream: OutputStream,
    pub offset: u64,
}

pub trait UserExecSession {
    fn attach(&self, request: &UserAttachRequest) -> Result<(), UserdError>;
    fn resize(&self, exec_id: &ExecId, size: TerminalSize) -> Result<(), UserdError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserdError {
    SessionUnavailable,
    ExecNotFound,
    PermissionDenied,
    TerminalSizeRequired,
    TerminalSizeUnexpected,
    InvalidTerminalSize,
    Protocol(GuestControlErrorKind),
}

pub trait UserSocketPolicy {
    fn transport(&self) -> UserdTransport;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserdTransport {
    UnixSocket,
}

pub struct UnixSocketOnly;

impl UserSocketPolicy for UnixSocketOnly {
    fn transport(&self) -> UserdTransport {
        UserdTransport::UnixSocket
    }
}

pub fn validate_attach_request(request: &UserAttachRequest) -> Result<(), UserdError> {
    match (request.tty, request.initial_size) {
        (true, Some(size)) => validate_terminal_size(size),
        (true, None) => Err(UserdError::TerminalSizeRequired),
        (false, Some(_)) => Err(UserdError::TerminalSizeUnexpected),
        (false, None) => Ok(()),
    }
}

pub fn validate_terminal_size(size: TerminalSize) -> Result<(), UserdError> {
    if (1..=65535).contains(&size.rows) && (1..=65535).contains(&size.cols) {
        Ok(())
    } else {
        Err(UserdError::InvalidTerminalSize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userd_never_listens_on_vsock() {
        assert!(!userd_listens_on_vsock());
        assert_eq!(UnixSocketOnly.transport(), UserdTransport::UnixSocket);
    }

    #[test]
    fn attach_request_validates_tty_geometry_contract() {
        let base = UserAttachRequest {
            exec_id: ExecId::new("exec-1"),
            guest_boot_id: GuestBootId::new("boot-1"),
            tty: true,
            initial_size: Some(TerminalSize { rows: 24, cols: 80 }),
        };
        assert!(validate_attach_request(&base).is_ok());

        let missing_size = UserAttachRequest {
            initial_size: None,
            ..base.clone()
        };
        assert_eq!(
            validate_attach_request(&missing_size),
            Err(UserdError::TerminalSizeRequired)
        );

        let invalid_zero = UserAttachRequest {
            initial_size: Some(TerminalSize { rows: 0, cols: 80 }),
            ..base.clone()
        };
        assert_eq!(
            validate_attach_request(&invalid_zero),
            Err(UserdError::InvalidTerminalSize)
        );

        let non_tty_with_size = UserAttachRequest { tty: false, ..base };
        assert_eq!(
            validate_attach_request(&non_tty_with_size),
            Err(UserdError::TerminalSizeUnexpected)
        );
    }
}
