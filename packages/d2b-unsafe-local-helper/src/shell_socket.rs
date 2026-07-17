use nix::sys::socket::{getsockopt, sockopt};
use rustix::io::{FdFlags, fcntl_getfd};
use std::fmt;
use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use crate::supervisor_protocol::ATTACH_METHOD_ID;

pub const TERMINAL_ATTACHMENT_INDEX: u32 = 0;

pub struct AuthenticatedTerminalAttachment {
    pub index: u32,
    pub owner_uid: u32,
    pub session_generation: u64,
    pub request_id: [u8; 16],
    pub method_id: u32,
    pub connected_stream: bool,
    pub cloexec: bool,
    fd: OwnedFd,
}

impl AuthenticatedTerminalAttachment {
    pub fn new(fd: OwnedFd, owner_uid: u32, session_generation: u64, request_id: [u8; 16]) -> Self {
        Self {
            index: TERMINAL_ATTACHMENT_INDEX,
            owner_uid,
            session_generation,
            request_id,
            method_id: ATTACH_METHOD_ID,
            connected_stream: true,
            cloexec: true,
            fd,
        }
    }
}

impl fmt::Debug for AuthenticatedTerminalAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedTerminalAttachment")
            .field("index", &self.index)
            .field("owner_uid", &"<redacted>")
            .field("session_generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("method_id", &self.method_id)
            .field("connected_stream", &self.connected_stream)
            .field("cloexec", &self.cloexec)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachmentError {
    CountMismatch,
    BindingMismatch,
    DescriptorInvalid,
}

pub(crate) fn validate_runtime_directory(
    directory: &Path,
    expected_uid: u32,
) -> Result<(), AttachmentError> {
    let metadata =
        fs::symlink_metadata(directory).map_err(|_| AttachmentError::DescriptorInvalid)?;
    let mode = metadata.permissions().mode() & 0o7777;
    if !directory.is_absolute()
        || !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != expected_uid
        || mode & 0o700 != 0o700
        || mode & 0o027 != 0
        || mode & 0o7000 != 0
    {
        return Err(AttachmentError::DescriptorInvalid);
    }
    Ok(())
}

pub(crate) fn validate_exact_terminal_attachment(
    mut attachments: Vec<AuthenticatedTerminalAttachment>,
    expected_uid: u32,
    expected_generation: u64,
    expected_request_id: [u8; 16],
) -> Result<OwnedFd, AttachmentError> {
    if attachments.len() != 1 {
        return Err(AttachmentError::CountMismatch);
    }
    let attachment = attachments.pop().ok_or(AttachmentError::CountMismatch)?;
    if attachment.index != TERMINAL_ATTACHMENT_INDEX
        || attachment.owner_uid != expected_uid
        || attachment.session_generation != expected_generation
        || attachment.request_id != expected_request_id
        || attachment.method_id != ATTACH_METHOD_ID
        || !attachment.connected_stream
        || !attachment.cloexec
    {
        return Err(AttachmentError::BindingMismatch);
    }
    let flags = fcntl_getfd(&attachment.fd).map_err(|_| AttachmentError::DescriptorInvalid)?;
    let socket_type = getsockopt(&attachment.fd, sockopt::SockType)
        .map_err(|_| AttachmentError::DescriptorInvalid)?;
    let peer = getsockopt(&attachment.fd, sockopt::PeerCredentials)
        .map_err(|_| AttachmentError::DescriptorInvalid)?;
    if !flags.contains(FdFlags::CLOEXEC)
        || socket_type != nix::sys::socket::SockType::Stream
        || peer.uid() as u32 != expected_uid
    {
        return Err(AttachmentError::DescriptorInvalid);
    }
    Ok(attachment.fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::getuid;
    use std::os::unix::net::UnixStream;

    fn attachment() -> AuthenticatedTerminalAttachment {
        let (stream, _peer) = UnixStream::pair().unwrap();
        AuthenticatedTerminalAttachment::new(stream.into(), getuid().as_raw(), 9, [7; 16])
    }

    #[test]
    fn accepts_exact_connected_cloexec_stream() {
        let fd =
            validate_exact_terminal_attachment(vec![attachment()], getuid().as_raw(), 9, [7; 16])
                .unwrap();
        drop(fd);
    }

    #[test]
    fn rejects_extra_or_metadata_only_descriptors() {
        assert_eq!(
            validate_exact_terminal_attachment(
                vec![attachment(), attachment()],
                getuid().as_raw(),
                9,
                [7; 16],
            )
            .unwrap_err(),
            AttachmentError::CountMismatch
        );
        let mut wrong = attachment();
        wrong.cloexec = false;
        assert_eq!(
            validate_exact_terminal_attachment(vec![wrong], getuid().as_raw(), 9, [7; 16])
                .unwrap_err(),
            AttachmentError::BindingMismatch
        );
    }

    #[test]
    fn debug_never_exposes_identity() {
        let attachment = attachment();
        let debug = format!("{attachment:?}");
        assert!(!debug.contains(&getuid().as_raw().to_string()));
        assert!(!debug.contains("[7"));
    }
}
