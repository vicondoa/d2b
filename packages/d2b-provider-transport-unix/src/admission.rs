//! Accepted-file-descriptor admission for the local transport portal.

use rustix::{
    fd::AsFd,
    fs::{fcntl_getfd, fcntl_setfd},
    io::FdFlags,
    net::{
        AddressFamily, SocketType,
        sockopt::{get_socket_domain, get_socket_type, set_socket_passcred},
    },
};
use std::{error::Error, fmt};

/// The requested Unix socket type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    /// A packet-preserving Unix seqpacket socket.
    Seqpacket,
    /// A byte-stream Unix socket.
    Stream,
}

/// The caller's closed transport route class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteClass {
    /// A child ZoneLink route that cannot carry descriptor attachments.
    ZoneLink,
    /// A same-Zone portal route that may carry seqpacket attachments.
    LocalPortal,
}

/// Validated arguments for one transport-open request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTransportRequest {
    socket_kind: SocketKind,
    route_class: RouteClass,
    attachments_enabled: bool,
}

impl OpenTransportRequest {
    /// Construct a closed transport request.
    pub const fn new(
        socket_kind: SocketKind,
        route_class: RouteClass,
        attachments_enabled: bool,
    ) -> Self {
        Self {
            socket_kind,
            route_class,
            attachments_enabled,
        }
    }

    /// Return the requested socket kind.
    pub const fn socket_kind(self) -> SocketKind {
        self.socket_kind
    }

    /// Return the route class.
    pub const fn route_class(self) -> RouteClass {
        self.route_class
    }

    /// Return whether descriptor attachments are requested.
    pub const fn attachments_enabled(self) -> bool {
        self.attachments_enabled
    }
}

/// Fail-closed admission outcome for an accepted descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAdmissionError {
    /// The route policy prohibits descriptor attachments.
    AttachmentPolicyConflict,
    /// The descriptor is not an AF_UNIX socket of the declared type.
    SocketKindMismatch,
    /// The descriptor cannot be made close-on-exec.
    Cloexec,
    /// The descriptor's peer credentials could not be read.
    PeerCredentials,
}

impl fmt::Display for TransportAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AttachmentPolicyConflict => "attachment-policy-conflict",
            Self::SocketKindMismatch => "socket-kind-mismatch",
            Self::Cloexec => "cloexec-set-failed",
            Self::PeerCredentials => "peer-credentials-unavailable",
        })
    }
}

impl Error for TransportAdmissionError {}

pub(crate) fn validate_route_class(
    request: OpenTransportRequest,
) -> Result<(), TransportAdmissionError> {
    if (request.route_class == RouteClass::ZoneLink || request.socket_kind == SocketKind::Stream)
        && request.attachments_enabled
    {
        return Err(TransportAdmissionError::AttachmentPolicyConflict);
    }
    Ok(())
}

pub(crate) fn validate_and_prepare(
    fd: impl AsFd,
    request: OpenTransportRequest,
) -> Result<(), TransportAdmissionError> {
    validate_route_class(request)?;
    let fd = fd.as_fd();
    if get_socket_domain(fd).ok() != Some(AddressFamily::UNIX)
        || get_socket_type(fd).ok()
            != Some(match request.socket_kind {
                SocketKind::Seqpacket => SocketType::SEQPACKET,
                SocketKind::Stream => SocketType::STREAM,
            })
    {
        return Err(TransportAdmissionError::SocketKindMismatch);
    }
    let flags = fcntl_getfd(fd).map_err(|_| TransportAdmissionError::Cloexec)?;
    fcntl_setfd(fd, flags | FdFlags::CLOEXEC).map_err(|_| TransportAdmissionError::Cloexec)?;
    if request.socket_kind == SocketKind::Seqpacket {
        set_socket_passcred(fd, true).map_err(|_| TransportAdmissionError::PeerCredentials)?;
    }
    Ok(())
}
