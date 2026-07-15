use std::{error, fmt};

use d2b_contracts::v2_state::{QuarantineReason, Remediation, StateContractError};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Missing,
    AlreadyExists,
    PathRejected,
    NotRegularFile,
    MetadataMismatch,
    TooLarge,
    Empty,
    NonCanonical,
    InvalidSchema,
    InvalidWriter,
    GenerationRollback,
    GenerationGap,
    ChecksumMismatch,
    QuarantineRequired,
    LockOrder,
    LockContended,
    LockRequired,
    LockMismatch,
    LockReleased,
    Deadline,
    Cancelled,
    TransferDenied,
    AuditInvalid,
    TaskJoin,
    Io,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Error {
    Code(ErrorCode),
    Os {
        code: ErrorCode,
        errno: Option<i32>,
    },
    Contract(StateContractError),
    Quarantine {
        reason: QuarantineReason,
        remediation: Remediation,
    },
    TaskJoin {
        cancelled: bool,
        panicked: bool,
    },
}

impl Error {
    pub(crate) fn io(code: ErrorCode, error: rustix::io::Errno) -> Self {
        Self::Os {
            code,
            errno: Some(error.raw_os_error()),
        }
    }

    pub(crate) fn std_io(code: ErrorCode, error: &std::io::Error) -> Self {
        Self::Os {
            code,
            errno: error.raw_os_error(),
        }
    }

    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Code(code) | Self::Os { code, .. } => *code,
            Self::Contract(contract) => match contract {
                StateContractError::EnvelopeChecksumMismatch => ErrorCode::ChecksumMismatch,
                StateContractError::UnsupportedSchemaVersion
                | StateContractError::UnsupportedSchemaGeneration
                | StateContractError::EnvelopePayloadMismatch => ErrorCode::InvalidSchema,
                StateContractError::AuditOwnerMismatch
                | StateContractError::AuditStreamMismatch
                | StateContractError::AuditSequenceMismatch
                | StateContractError::AuditChainMismatch
                | StateContractError::AuditCheckpointMismatch
                | StateContractError::AuditGap
                | StateContractError::AuditExportRangeInvalid
                | StateContractError::RetentionOutOfBounds
                | StateContractError::RetentionCheckpointRequired => ErrorCode::AuditInvalid,
                StateContractError::LeaseGenerationMismatch => ErrorCode::GenerationRollback,
                StateContractError::LeaseExpired => ErrorCode::Deadline,
                StateContractError::LockOrderViolation
                | StateContractError::LockOrderCycle
                | StateContractError::UnknownLockDependency
                | StateContractError::DuplicateLockOrder
                | StateContractError::DuplicateLockId => ErrorCode::LockOrder,
                _ => ErrorCode::InvalidSchema,
            },
            Self::Quarantine { .. } => ErrorCode::QuarantineRequired,
            Self::TaskJoin { .. } => ErrorCode::TaskJoin,
        }
    }

    #[cfg(feature = "tokio")]
    pub(crate) fn task_join(error: tokio::task::JoinError) -> Self {
        Self::TaskJoin {
            cancelled: error.is_cancelled(),
            panicked: error.is_panic(),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("StateError");
        debug.field("code", &self.code());
        if let Self::Os { errno, .. } = self {
            debug.field("errno", errno);
        }
        debug.finish_non_exhaustive()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Os {
                errno: Some(errno), ..
            } => write!(
                f,
                "d2b state operation failed ({:?}, errno={errno})",
                self.code()
            ),
            _ => write!(f, "d2b state operation failed ({:?})", self.code()),
        }
    }
}

impl error::Error for Error {}

impl From<StateContractError> for Error {
    fn from(value: StateContractError) -> Self {
        Self::Contract(value)
    }
}
