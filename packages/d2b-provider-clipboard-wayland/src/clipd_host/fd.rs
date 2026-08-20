use std::os::fd::AsFd;

use rustix::fs::{FileType, fstat, fstatfs};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const TMPFS_MAGIC: i64 = 0x0102_1994;
const RAMFS_MAGIC: i64 = 0x8584_58f6u32 as i64;
const HUGETLBFS_MAGIC: i64 = 0x9584_58f6u32 as i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FdObjectKind {
    Pipe,
    Socket,
    Regular,
    BlockDevice,
    CharacterDevice,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileSystemKind {
    MemoryBacked,
    DiskBacked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FdStatModel {
    pub object_kind: FdObjectKind,
    pub filesystem_kind: FileSystemKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcceptedTransferFdKind {
    Pipe,
    Socket,
    MemoryBackedRegular,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FdSafetyError {
    #[error("fd metadata query failed: {0}")]
    Io(String),
    #[error("transfer fd kind is rejected: {0:?}")]
    RejectedKind(FdObjectKind),
    #[error("regular transfer fd is not memory-backed: {0:?}")]
    RegularNotMemoryBacked(FileSystemKind),
    #[error("fd cap leaves fewer than {reserved_margin} reserved descriptors below rlimit")]
    CapExceedsRlimit {
        requested_cap: u64,
        rlimit: u64,
        reserved_margin: u64,
    },
    #[error("control message was truncated; close {fds_to_close} partially received fds")]
    ControlMessageTruncated { fds_to_close: usize },
}

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

pub fn classify_fd(fd: impl AsFd) -> Result<AcceptedTransferFdKind, FdSafetyError> {
    let stat = fstat(fd.as_fd()).map_err(|err| FdSafetyError::Io(err.to_string()))?;
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
        let statfs = fstatfs(fd.as_fd()).map_err(|err| FdSafetyError::Io(err.to_string()))?;
        match statfs.f_type as i64 {
            TMPFS_MAGIC | RAMFS_MAGIC | HUGETLBFS_MAGIC => FileSystemKind::MemoryBacked,
            _ => FileSystemKind::DiskBacked,
        }
    } else {
        FileSystemKind::Unknown
    };
    classify_fd_model(FdStatModel {
        object_kind,
        filesystem_kind,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdCapModel {
    pub requested_cap: u64,
    pub rlimit_nofile: u64,
    pub base_reserved: u64,
    pub max_fds_per_recvmsg: u64,
}

impl FdCapModel {
    pub fn reserved_margin(self) -> u64 {
        self.base_reserved.saturating_add(self.max_fds_per_recvmsg)
    }
}

pub fn validate_fd_cap(model: FdCapModel) -> Result<u64, FdSafetyError> {
    let reserved_margin = model.reserved_margin();
    let max_allowed = model.rlimit_nofile.saturating_sub(reserved_margin);
    if model.requested_cap <= max_allowed {
        Ok(model.requested_cap)
    } else {
        Err(FdSafetyError::CapExceedsRlimit {
            requested_cap: model.requested_cap,
            rlimit: model.rlimit_nofile,
            reserved_margin,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvmsgControlStatus {
    pub msg_ctrunc: bool,
    pub received_fd_count: usize,
}

pub fn validate_recvmsg_control(status: RecvmsgControlStatus) -> Result<(), FdSafetyError> {
    if status.msg_ctrunc {
        Err(FdSafetyError::ControlMessageTruncated {
            fds_to_close: status.received_fd_count,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn pure_model_accepts_only_pipe_socket_and_memory_regular() {
        assert_eq!(
            classify_fd_model(FdStatModel {
                object_kind: FdObjectKind::Pipe,
                filesystem_kind: FileSystemKind::Unknown
            }),
            Ok(AcceptedTransferFdKind::Pipe)
        );
        assert_eq!(
            classify_fd_model(FdStatModel {
                object_kind: FdObjectKind::Socket,
                filesystem_kind: FileSystemKind::Unknown
            }),
            Ok(AcceptedTransferFdKind::Socket)
        );
        assert_eq!(
            classify_fd_model(FdStatModel {
                object_kind: FdObjectKind::Regular,
                filesystem_kind: FileSystemKind::MemoryBacked
            }),
            Ok(AcceptedTransferFdKind::MemoryBackedRegular)
        );
        assert!(matches!(
            classify_fd_model(FdStatModel {
                object_kind: FdObjectKind::Regular,
                filesystem_kind: FileSystemKind::DiskBacked
            }),
            Err(FdSafetyError::RegularNotMemoryBacked(
                FileSystemKind::DiskBacked
            ))
        ));
        assert!(matches!(
            classify_fd_model(FdStatModel {
                object_kind: FdObjectKind::BlockDevice,
                filesystem_kind: FileSystemKind::Unknown
            }),
            Err(FdSafetyError::RejectedKind(FdObjectKind::BlockDevice))
        ));
    }

    #[test]
    fn fd_caps_must_leave_reserved_margin() {
        assert_eq!(
            validate_fd_cap(FdCapModel {
                requested_cap: 64,
                rlimit_nofile: 256,
                base_reserved: 64,
                max_fds_per_recvmsg: 16,
            }),
            Ok(64)
        );
        assert!(matches!(
            validate_fd_cap(FdCapModel {
                requested_cap: 200,
                rlimit_nofile: 256,
                base_reserved: 64,
                max_fds_per_recvmsg: 16,
            }),
            Err(FdSafetyError::CapExceedsRlimit { .. })
        ));
    }

    #[test]
    fn msg_ctrunc_is_fail_closed_and_reports_fds_to_close() {
        assert_eq!(
            validate_recvmsg_control(RecvmsgControlStatus {
                msg_ctrunc: true,
                received_fd_count: 3
            }),
            Err(FdSafetyError::ControlMessageTruncated { fds_to_close: 3 })
        );
    }

    #[test]
    fn unix_socket_fd_is_accepted_without_root() {
        let (left, _right) = UnixStream::pair().expect("socketpair");
        assert_eq!(
            classify_fd(&left).expect("classify socket"),
            AcceptedTransferFdKind::Socket
        );
    }
}
