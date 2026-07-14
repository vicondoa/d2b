use std::{error::Error, fmt};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnixSessionError {
    Io,
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
        formatter.write_str(match self {
            Self::Io => "unix-session-io",
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
            Self::CreditExceeded => "unix-session-credit-exceeded",
            Self::PacketNotAtomic => "unix-session-packet-not-atomic",
            Self::FairnessBudget => "unix-session-fairness-budget",
        })
    }
}

impl Error for UnixSessionError {}

pub(crate) fn io_error(_: impl std::fmt::Debug) -> UnixSessionError {
    UnixSessionError::Io
}
