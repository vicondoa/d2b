#![doc = "Guest user-session agent primitives for nixling exec."]

use nixling_ipc::guest_wire::{
    ExecId, GuestBootId, GuestControlErrorKind, OutputStream, TerminalSize,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UserdError {
    SessionUnavailable,
    ExecNotFound,
    PermissionDenied,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userd_never_listens_on_vsock() {
        assert!(!userd_listens_on_vsock());
        assert_eq!(UnixSocketOnly.transport(), UserdTransport::UnixSocket);
    }
}
