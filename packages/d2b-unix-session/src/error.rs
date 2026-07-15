use std::{error::Error, fmt};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnixSessionError {
    Io { errno: Option<i32> },
    Closed,
    InvalidSocket,
    BlockingSocket,
    MissingCloexec,
    PasscredNotPrearmed,
    EmptyPacket,
    PayloadLimit,
    AncillaryCapacity,
    MessageTruncated,
    ControlTruncated,
    UnknownControl,
    ControlMismatch,
    CredentialMismatch,
    DescriptorMismatch,
    DuplicateObject,
    PidfdEvidenceUnavailable,
    PidfdIdentityMismatch,
    CreditExceeded,
    PacketNotAtomic,
    FairnessBudget,
}

impl fmt::Debug for UnixSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for UnixSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Io { errno: Some(errno) } => {
                return write!(formatter, "unix-session-io(errno={errno})");
            }
            Self::Io { errno: None } => "unix-session-io",
            Self::Closed => "unix-session-closed",
            Self::InvalidSocket => "unix-session-invalid-socket",
            Self::BlockingSocket => "unix-session-blocking-socket",
            Self::MissingCloexec => "unix-session-missing-cloexec",
            Self::PasscredNotPrearmed => "unix-session-passcred-not-prearmed",
            Self::EmptyPacket => "unix-session-empty-packet",
            Self::PayloadLimit => "unix-session-payload-limit",
            Self::AncillaryCapacity => "unix-session-ancillary-capacity",
            Self::MessageTruncated => "unix-session-message-truncated",
            Self::ControlTruncated => "unix-session-control-truncated",
            Self::UnknownControl => "unix-session-unknown-control",
            Self::ControlMismatch => "unix-session-control-mismatch",
            Self::CredentialMismatch => "unix-session-credential-mismatch",
            Self::DescriptorMismatch => "unix-session-descriptor-mismatch",
            Self::DuplicateObject => "unix-session-duplicate-object",
            Self::PidfdEvidenceUnavailable => "unix-session-pidfd-evidence-unavailable",
            Self::PidfdIdentityMismatch => "unix-session-pidfd-identity-mismatch",
            Self::CreditExceeded => "unix-session-credit-exceeded",
            Self::PacketNotAtomic => "unix-session-packet-not-atomic",
            Self::FairnessBudget => "unix-session-fairness-budget",
        };
        formatter.write_str(message)
    }
}

impl Error for UnixSessionError {}

#[cfg(feature = "host-socket")]
pub(crate) trait IoErrno {
    fn errno(&self) -> Option<i32>;
}

#[cfg(feature = "host-socket")]
impl IoErrno for std::io::Error {
    fn errno(&self) -> Option<i32> {
        self.raw_os_error()
    }
}

#[cfg(feature = "host-socket")]
impl IoErrno for rustix::io::Errno {
    fn errno(&self) -> Option<i32> {
        Some(self.raw_os_error())
    }
}

#[cfg(feature = "host-socket")]
pub(crate) fn io_error(error: impl IoErrno) -> UnixSessionError {
    UnixSessionError::Io {
        errno: error.errno(),
    }
}
