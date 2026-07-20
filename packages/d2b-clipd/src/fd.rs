use std::os::fd::AsFd;

use rustix::fs::{FileType, fstat, fstatfs};
use rustix::io::{FdFlags, fcntl_getfd};
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
    #[error("ComponentSession transfer fd is missing close-on-exec")]
    MissingCloexec,
    #[error("clipboard transfer owner was not authenticated by ComponentSession")]
    UnauthenticatedOwner,
    #[error("clipboard transfer descriptor ownership does not match the authenticated request")]
    OwnershipMismatch,
    #[error("clipboard transfer packet must contain exactly one descriptor")]
    DescriptorCountMismatch,
    #[error("clipboard transfer descriptor was not delivered atomically")]
    NonAtomicDescriptor,
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
    let fd = fd.as_fd();
    if !fcntl_getfd(fd)
        .map_err(|err| FdSafetyError::Io(err.to_string()))?
        .contains(FdFlags::CLOEXEC)
    {
        return Err(FdSafetyError::MissingCloexec);
    }
    let stat = fstat(fd).map_err(|err| FdSafetyError::Io(err.to_string()))?;
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
        let statfs = fstatfs(fd).map_err(|err| FdSafetyError::Io(err.to_string()))?;
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

#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedTransferOwner {
    request_id: [u8; 16],
    operation_id: [u8; 16],
    reconnect_generation: u64,
}

impl std::fmt::Debug for AuthenticatedTransferOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AuthenticatedTransferOwner(<redacted>)")
    }
}

impl AuthenticatedTransferOwner {
    pub fn from_component_session(
        request_id: [u8; 16],
        operation_id: [u8; 16],
        reconnect_generation: u64,
    ) -> Result<Self, FdSafetyError> {
        if request_id == [0; 16] || operation_id == [0; 16] || reconnect_generation == 0 {
            return Err(FdSafetyError::UnauthenticatedOwner);
        }
        Ok(Self {
            request_id,
            operation_id,
            reconnect_generation,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ComponentSessionFdClaim {
    pub request_id: [u8; 16],
    pub operation_id: [u8; 16],
    pub reconnect_generation: u64,
    pub packet_sequence: u64,
    pub descriptor_index: u16,
    pub descriptor_count: u16,
    pub packet_atomic: bool,
    pub cloexec_required: bool,
}

impl std::fmt::Debug for ComponentSessionFdClaim {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComponentSessionFdClaim")
            .field("descriptor_index", &self.descriptor_index)
            .field("descriptor_count", &self.descriptor_count)
            .field("packet_atomic", &self.packet_atomic)
            .field("cloexec_required", &self.cloexec_required)
            .finish_non_exhaustive()
    }
}

pub fn validate_component_session_transfer_fd(
    fd: impl AsFd,
    owner: &AuthenticatedTransferOwner,
    claim: &ComponentSessionFdClaim,
) -> Result<AcceptedTransferFdKind, FdSafetyError> {
    if claim.descriptor_count != 1 || claim.descriptor_index != 0 {
        return Err(FdSafetyError::DescriptorCountMismatch);
    }
    if !claim.packet_atomic || claim.packet_sequence == 0 {
        return Err(FdSafetyError::NonAtomicDescriptor);
    }
    if !claim.cloexec_required {
        return Err(FdSafetyError::MissingCloexec);
    }
    if claim.request_id != owner.request_id
        || claim.operation_id != owner.operation_id
        || claim.reconnect_generation != owner.reconnect_generation
    {
        return Err(FdSafetyError::OwnershipMismatch);
    }
    classify_fd(fd)
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
    use std::os::{fd::AsRawFd, unix::net::UnixStream};

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

    #[test]
    fn missing_cloexec_is_rejected_from_the_live_descriptor() {
        let (left, _right) = UnixStream::pair().expect("socketpair");
        nix::fcntl::fcntl(
            left.as_raw_fd(),
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
        )
        .expect("clear cloexec");

        assert_eq!(classify_fd(&left), Err(FdSafetyError::MissingCloexec));
    }

    fn authenticated_owner() -> AuthenticatedTransferOwner {
        AuthenticatedTransferOwner::from_component_session([1; 16], [2; 16], 7).unwrap()
    }

    fn claim() -> ComponentSessionFdClaim {
        ComponentSessionFdClaim {
            request_id: [1; 16],
            operation_id: [2; 16],
            reconnect_generation: 7,
            packet_sequence: 1,
            descriptor_index: 0,
            descriptor_count: 1,
            packet_atomic: true,
            cloexec_required: true,
        }
    }

    #[test]
    fn component_session_fd_requires_exact_atomic_ownership() {
        let (left, _right) = UnixStream::pair().expect("socketpair");
        assert_eq!(
            validate_component_session_transfer_fd(&left, &authenticated_owner(), &claim()),
            Ok(AcceptedTransferFdKind::Socket)
        );

        for mutation in 0..7 {
            let mut changed = claim();
            let expected = match mutation {
                0 => {
                    changed.request_id = [9; 16];
                    FdSafetyError::OwnershipMismatch
                }
                1 => {
                    changed.operation_id = [9; 16];
                    FdSafetyError::OwnershipMismatch
                }
                2 => {
                    changed.reconnect_generation += 1;
                    FdSafetyError::OwnershipMismatch
                }
                3 => {
                    changed.packet_sequence = 0;
                    FdSafetyError::NonAtomicDescriptor
                }
                4 => {
                    changed.descriptor_index = 1;
                    FdSafetyError::DescriptorCountMismatch
                }
                5 => {
                    changed.descriptor_count = 2;
                    FdSafetyError::DescriptorCountMismatch
                }
                6 => {
                    changed.packet_atomic = false;
                    FdSafetyError::NonAtomicDescriptor
                }
                _ => unreachable!(),
            };
            assert_eq!(
                validate_component_session_transfer_fd(&left, &authenticated_owner(), &changed),
                Err(expected)
            );
        }
    }

    #[test]
    fn unauthenticated_owner_cannot_be_constructed() {
        assert_eq!(
            AuthenticatedTransferOwner::from_component_session([0; 16], [2; 16], 7),
            Err(FdSafetyError::UnauthenticatedOwner)
        );
        assert_eq!(
            AuthenticatedTransferOwner::from_component_session([1; 16], [0; 16], 7),
            Err(FdSafetyError::UnauthenticatedOwner)
        );
        assert_eq!(
            AuthenticatedTransferOwner::from_component_session([1; 16], [2; 16], 0),
            Err(FdSafetyError::UnauthenticatedOwner)
        );
    }
}
