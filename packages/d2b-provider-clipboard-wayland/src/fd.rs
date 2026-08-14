//! FD safety models and the checked Unix attachment adapter.

use std::os::fd::AsFd;

use rustix::{
    fs::{FileType, fstat, fstatfs},
    io::{FdFlags, fcntl_getfd},
};

const TMPFS_MAGIC: i64 = 0x0102_1994;
const RAMFS_MAGIC: i64 = 0x8584_58f6u32 as i64;
const HUGETLBFS_MAGIC: i64 = 0x9584_58f6u32 as i64;

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

/// The operation which owns a received descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentClass {
    /// Guest clipboard content flowing into the host.
    GuestTransfer,
    /// Host selection content flowing into a Guest.
    HostSelectionRead,
    /// A descriptor supplied by a host selection writer.
    HostSelectionWrite,
}

/// Metadata observed by the authenticated attachment adapter.
///
/// The model is intentionally separate from [`FdStatModel`], which remains a
/// small catalog classifier used by policy tests.  Callers cannot omit any of
/// the kernel properties required for a live descriptor admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdMetadata {
    /// Classified object kind.
    pub object_kind: FdObjectKind,
    /// Classified filesystem kind.
    pub filesystem_kind: FileSystemKind,
    /// Kernel link count.
    pub link_count: u64,
    /// Reported byte size.
    pub size_bytes: u64,
    /// Permission bits from `st_mode`.
    pub mode: u32,
    /// Whether close-on-exec is set.
    pub close_on_exec: bool,
    /// Operation class asserted by the authenticated stream.
    pub attachment_class: AttachmentClass,
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
    /// A descriptor did not have the required single-link identity.
    InvalidLinkCount {
        /// Observed link count.
        observed: u64,
    },
    /// A descriptor exceeded the bounded transfer size.
    SizeExceeded {
        /// Observed size.
        observed: u64,
        /// Configured limit.
        limit: u64,
    },
    /// A descriptor had unsafe permission bits.
    UnsafeMode {
        /// Observed mode bits.
        mode: u32,
    },
    /// The descriptor was not close-on-exec.
    CloseOnExecRequired,
    /// The stream's attachment class did not match the accepted class.
    AttachmentClassMismatch {
        /// Expected class.
        expected: AttachmentClass,
        /// Observed class.
        observed: AttachmentClass,
    },
    /// A metadata query failed.
    MetadataIo,
}

impl core::fmt::Display for FdSafetyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::RejectedKind(_) => "fd-safety-violation",
            Self::RegularNotMemoryBacked(_) => "fd-safety-violation",
            Self::CapExceedsRlimit { .. } => "fd-count-exceeded",
            Self::ControlMessageTruncated { .. } => "msg-ctrunc",
            Self::InvalidLinkCount { .. }
            | Self::SizeExceeded { .. }
            | Self::UnsafeMode { .. }
            | Self::CloseOnExecRequired
            | Self::AttachmentClassMismatch { .. }
            | Self::MetadataIo => "fd-safety-violation",
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

/// Validate all kernel metadata required for one clipboard attachment.
pub fn validate_fd_metadata(
    metadata: FdMetadata,
    expected_class: AttachmentClass,
    max_size_bytes: u64,
) -> Result<AcceptedTransferFdKind, FdSafetyError> {
    if metadata.link_count != 1 {
        return Err(FdSafetyError::InvalidLinkCount {
            observed: metadata.link_count,
        });
    }
    if metadata.size_bytes > max_size_bytes {
        return Err(FdSafetyError::SizeExceeded {
            observed: metadata.size_bytes,
            limit: max_size_bytes,
        });
    }
    // Clipboard descriptors are private transfer objects.  Reject group/world
    // permissions even when the underlying object is otherwise admissible.
    if metadata.mode & 0o077 != 0 {
        return Err(FdSafetyError::UnsafeMode {
            mode: metadata.mode,
        });
    }
    if !metadata.close_on_exec {
        return Err(FdSafetyError::CloseOnExecRequired);
    }
    if metadata.attachment_class != expected_class {
        return Err(FdSafetyError::AttachmentClassMismatch {
            expected: expected_class,
            observed: metadata.attachment_class,
        });
    }
    classify_fd_model(FdStatModel {
        object_kind: metadata.object_kind,
        filesystem_kind: metadata.filesystem_kind,
    })
}

/// Inspect a received descriptor with `fstat`, `fstatfs`, and `F_GETFD`.
///
/// The returned metadata is only useful when paired with the authenticated
/// operation class supplied by the ComponentSession adapter.
pub fn inspect_fd(
    fd: impl AsFd,
    attachment_class: AttachmentClass,
) -> Result<FdMetadata, FdSafetyError> {
    let stat = fstat(fd.as_fd()).map_err(|_| FdSafetyError::MetadataIo)?;
    let object_kind = match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => FdObjectKind::Regular,
        FileType::Fifo => FdObjectKind::Pipe,
        FileType::Socket => FdObjectKind::Socket,
        FileType::BlockDevice => FdObjectKind::BlockDevice,
        FileType::CharacterDevice => FdObjectKind::CharacterDevice,
        FileType::Directory => FdObjectKind::Directory,
        FileType::Symlink => FdObjectKind::Symlink,
        FileType::Unknown => FdObjectKind::Other,
    };
    let filesystem_kind = if object_kind == FdObjectKind::Regular {
        let filesystem = fstatfs(fd.as_fd()).map_err(|_| FdSafetyError::MetadataIo)?;
        match filesystem.f_type as i64 {
            TMPFS_MAGIC | RAMFS_MAGIC | HUGETLBFS_MAGIC => FileSystemKind::MemoryBacked,
            _ => FileSystemKind::DiskBacked,
        }
    } else {
        FileSystemKind::Unknown
    };
    let flags = fcntl_getfd(fd.as_fd()).map_err(|_| FdSafetyError::MetadataIo)?;
    Ok(FdMetadata {
        object_kind,
        filesystem_kind,
        link_count: stat.st_nlink,
        size_bytes: stat.st_size.try_into().unwrap_or(u64::MAX),
        mode: stat.st_mode,
        close_on_exec: flags.contains(FdFlags::CLOEXEC),
        attachment_class,
    })
}

/// Validate a live descriptor against the authenticated stream class.
pub fn validate_received_fd(
    fd: impl AsFd,
    attachment_class: AttachmentClass,
    max_size_bytes: u64,
) -> Result<AcceptedTransferFdKind, FdSafetyError> {
    let metadata = inspect_fd(fd, attachment_class)?;
    validate_fd_metadata(metadata, attachment_class, max_size_bytes)
}

/// A batch of received descriptors whose ownership remains local until
/// validation succeeds.  Dropping the batch deterministically closes every
/// descriptor, including descriptors from a truncated control message.
#[derive(Debug)]
pub struct ReceivedFdBatch<F> {
    descriptors: Vec<F>,
    truncated: bool,
}

impl<F> ReceivedFdBatch<F> {
    /// Construct a batch from the exact `recvmsg` result.
    pub fn new(descriptors: Vec<F>, truncated: bool) -> Self {
        Self {
            descriptors,
            truncated,
        }
    }

    /// Return the number of descriptors retained by the batch.
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// Whether the batch contains no descriptors.
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }
}

impl<F> ReceivedFdBatch<F>
where
    F: AsFd,
{
    /// Reject control truncation before exposing any descriptor.
    pub fn validate_control(self) -> Result<Vec<F>, FdSafetyError> {
        validate_recvmsg_control(self.truncated, self.descriptors.len())?;
        Ok(self.descriptors)
    }
}
