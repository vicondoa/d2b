//! Pure FD safety models for attachment validation.

/// Object type reported by `fstat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdObjectKind {
    /// A pipe.
    Pipe,
    /// A Unix socket.
    Socket,
    /// A regular file.
    Regular,
    /// A block device.
    BlockDevice,
    /// A character device.
    CharacterDevice,
    /// A directory.
    Directory,
    /// A symlink.
    Symlink,
    /// Any unclassified object.
    Other,
}

/// Filesystem classification reported by `fstatfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystemKind {
    /// A memory-backed filesystem.
    MemoryBacked,
    /// A disk-backed filesystem.
    DiskBacked,
    /// A filesystem not recognized by the adapter.
    Unknown,
}

/// Pure metadata model for one received FD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdStatModel {
    /// Object kind.
    pub object_kind: FdObjectKind,
    /// Filesystem kind.
    pub filesystem_kind: FileSystemKind,
}

/// Allowed attachment object class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedTransferFdKind {
    /// Pipe attachment.
    Pipe,
    /// Socket attachment.
    Socket,
    /// Memory-backed regular attachment.
    MemoryBackedRegular,
}

/// FD validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdSafetyError {
    /// Object kind is not allowed.
    RejectedKind(FdObjectKind),
    /// Regular files must be memory-backed.
    RegularNotMemoryBacked(FileSystemKind),
    /// The requested FD cap would consume reserved descriptors.
    CapExceedsRlimit {
        /// Requested cap.
        requested_cap: u64,
        /// Process file descriptor limit.
        rlimit: u64,
        /// Reserved descriptor margin.
        reserved_margin: u64,
    },
    /// Ancillary data was truncated.
    ControlMessageTruncated {
        /// Number of received descriptors to close.
        fds_to_close: usize,
    },
}

impl core::fmt::Display for FdSafetyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::RejectedKind(_) => "fd-safety-violation",
            Self::RegularNotMemoryBacked(_) => "fd-safety-violation",
            Self::CapExceedsRlimit { .. } => "fd-count-exceeded",
            Self::ControlMessageTruncated { .. } => "msg-ctrunc",
        })
    }
}

impl std::error::Error for FdSafetyError {}

/// Classify a pure FD metadata model.
pub fn classify_fd_model(model: FdStatModel) -> Result<AcceptedTransferFdKind, FdSafetyError> {
    match model.object_kind {
        FdObjectKind::Pipe => Ok(AcceptedTransferFdKind::Pipe),
        FdObjectKind::Socket => Ok(AcceptedTransferFdKind::Socket),
        FdObjectKind::Regular if model.filesystem_kind == FileSystemKind::MemoryBacked => {
            Ok(AcceptedTransferFdKind::MemoryBackedRegular)
        }
        FdObjectKind::Regular => Err(FdSafetyError::RegularNotMemoryBacked(model.filesystem_kind)),
        rejected => Err(FdSafetyError::RejectedKind(rejected)),
    }
}

/// FD bound validation model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdCapModel {
    /// Requested concurrent FD cap.
    pub requested_cap: u64,
    /// Process `RLIMIT_NOFILE`.
    pub rlimit_nofile: u64,
    /// Reserved descriptors owned by the service.
    pub base_reserved: u64,
    /// Maximum descriptors in one ancillary message.
    pub max_fds_per_recvmsg: u64,
}

/// Validate a concurrent FD cap.
pub fn validate_fd_cap(model: FdCapModel) -> Result<u64, FdSafetyError> {
    let reserved_margin = model
        .base_reserved
        .saturating_add(model.max_fds_per_recvmsg);
    if model.requested_cap <= model.rlimit_nofile.saturating_sub(reserved_margin) {
        Ok(model.requested_cap)
    } else {
        Err(FdSafetyError::CapExceedsRlimit {
            requested_cap: model.requested_cap,
            rlimit: model.rlimit_nofile,
            reserved_margin,
        })
    }
}

/// Reject a truncated `recvmsg` control message.
pub fn validate_recvmsg_control(
    truncated: bool,
    received_fd_count: usize,
) -> Result<(), FdSafetyError> {
    if truncated {
        Err(FdSafetyError::ControlMessageTruncated {
            fds_to_close: received_fd_count,
        })
    } else {
        Ok(())
    }
}
