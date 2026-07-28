use std::{error::Error, fmt};

use d2b_contracts::v3::component_session::{
    BinaryError, ContractError, FragmentSequenceError, HandshakeRejectReason, Remediation,
    SequenceError, SessionErrorCode,
};

use crate::TransportError;

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionError {
    code: SessionErrorCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionErrorClass {
    Authentication,
    Authorization,
    Generation,
    Backpressure,
    Deadline,
    Cancellation,
    Transport,
    Protocol,
    Internal,
}

impl SessionErrorClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Generation => "generation",
            Self::Backpressure => "backpressure",
            Self::Deadline => "deadline",
            Self::Cancellation => "cancellation",
            Self::Transport => "transport",
            Self::Protocol => "protocol",
            Self::Internal => "internal",
        }
    }
}

impl SessionError {
    pub const fn new(code: SessionErrorCode) -> Self {
        Self { code }
    }

    pub const fn code(self) -> SessionErrorCode {
        self.code
    }

    pub const fn class(self) -> SessionErrorClass {
        match self.code {
            SessionErrorCode::AuthenticationFailed
            | SessionErrorCode::TranscriptMismatch
            | SessionErrorCode::IdentityEvidenceMismatch => SessionErrorClass::Authentication,
            SessionErrorCode::PolicyDenied
            | SessionErrorCode::PurposeMismatch
            | SessionErrorCode::PurposeClassMismatch
            | SessionErrorCode::RoleMismatch
            | SessionErrorCode::ServiceMismatch
            | SessionErrorCode::SchemaMismatch
            | SessionErrorCode::ChannelBindingMismatch => SessionErrorClass::Authorization,
            SessionErrorCode::GenerationMismatch => SessionErrorClass::Generation,
            SessionErrorCode::QueueBackpressure
            | SessionErrorCode::ControlResourceExhausted
            | SessionErrorCode::AttachmentCreditExceeded
            | SessionErrorCode::ReassemblyLimitExceeded => SessionErrorClass::Backpressure,
            SessionErrorCode::HandshakeTimeout
            | SessionErrorCode::DeadlineInvalid
            | SessionErrorCode::DeadlineExpired
            | SessionErrorCode::KeepaliveTimeout => SessionErrorClass::Deadline,
            SessionErrorCode::Cancelled => SessionErrorClass::Cancellation,
            SessionErrorCode::SessionDisconnected => SessionErrorClass::Transport,
            SessionErrorCode::ArithmeticOverflow | SessionErrorCode::InternalInvariant => {
                SessionErrorClass::Internal
            }
            SessionErrorCode::MalformedPreface
            | SessionErrorCode::UnsupportedVersion
            | SessionErrorCode::MalformedHandshake
            | SessionErrorCode::LimitMismatch
            | SessionErrorCode::AttachmentPolicyMismatch
            | SessionErrorCode::RecordTruncated
            | SessionErrorCode::RecordMalformed
            | SessionErrorCode::RecordReplay
            | SessionErrorCode::RecordOutOfOrder
            | SessionErrorCode::NonceExhausted
            | SessionErrorCode::FragmentTruncated
            | SessionErrorCode::FragmentDuplicate
            | SessionErrorCode::FragmentReordered
            | SessionErrorCode::FragmentOverlap
            | SessionErrorCode::InvalidChannel
            | SessionErrorCode::UnknownControl
            | SessionErrorCode::RequestIdDuplicate
            | SessionErrorCode::AttachmentTruncated
            | SessionErrorCode::AttachmentControlTruncated
            | SessionErrorCode::AttachmentCountMismatch
            | SessionErrorCode::AttachmentDescriptorMismatch
            | SessionErrorCode::AttachmentObjectMismatch
            | SessionErrorCode::AttachmentAccessMismatch
            | SessionErrorCode::AttachmentMissingCloexec
            | SessionErrorCode::SchedulerStalled
            | SessionErrorCode::BootstrapExpired
            | SessionErrorCode::BootstrapReplayed
            | SessionErrorCode::BootstrapOperationMismatch
            | SessionErrorCode::SubjectMismatch
            | SessionErrorCode::TransportMismatch => SessionErrorClass::Protocol,
        }
    }

    pub const fn remediation(self) -> Remediation {
        match self.class() {
            SessionErrorClass::Authentication => Remediation::ReEnrollPeer,
            SessionErrorClass::Authorization => Remediation::RepairConfiguration,
            SessionErrorClass::Generation => Remediation::ReplaceGeneration,
            SessionErrorClass::Backpressure => Remediation::ReduceLoad,
            SessionErrorClass::Deadline => Remediation::RetryBounded,
            SessionErrorClass::Cancellation => Remediation::None,
            SessionErrorClass::Transport => Remediation::RestartAgent,
            SessionErrorClass::Protocol => Remediation::RepairConfiguration,
            SessionErrorClass::Internal => Remediation::RestartAgent,
        }
    }
}

impl fmt::Debug for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionError")
            .field("code", &self.code.as_str())
            .finish()
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "session-error class={} code={} remediation={}",
            self.class().as_str(),
            self.code.as_str(),
            self.remediation().as_str()
        )
    }
}

impl Error for SessionError {}

impl From<ContractError> for SessionError {
    fn from(error: ContractError) -> Self {
        use ContractError as C;
        use SessionErrorCode as S;
        Self::new(match error {
            C::ArithmeticOverflow => S::ArithmeticOverflow,
            C::LimitExceeded | C::CreditExceeded => S::QueueBackpressure,
            C::InvalidAttachmentPolicy | C::InvalidAttachment => S::AttachmentDescriptorMismatch,
            C::IdentityEvidenceMismatch => S::IdentityEvidenceMismatch,
            C::InvalidBinding => S::ChannelBindingMismatch,
            C::InvalidGeneration => S::GenerationMismatch,
            C::InvalidChannel => S::InvalidChannel,
            C::InvalidFragment => S::FragmentReordered,
            C::InvalidDeadline => S::DeadlineInvalid,
            C::InvalidId => S::RecordMalformed,
        })
    }
}

impl From<BinaryError> for SessionError {
    fn from(error: BinaryError) -> Self {
        use BinaryError as B;
        use SessionErrorCode as S;
        match error {
            B::InvalidContract(inner) => inner.into(),
            B::Truncated => Self::new(S::RecordTruncated),
            B::LengthExceeded => Self::new(S::ReassemblyLimitExceeded),
            B::TrailingBytes | B::UnknownEnumTag | B::UnsupportedVersion | B::NonCanonical => {
                Self::new(S::RecordMalformed)
            }
        }
    }
}

impl From<SequenceError> for SessionError {
    fn from(error: SequenceError) -> Self {
        use SequenceError as Q;
        use SessionErrorCode as S;
        Self::new(match error {
            Q::Replay => S::RecordReplay,
            Q::OutOfOrder => S::RecordOutOfOrder,
            Q::NonceExhausted => S::NonceExhausted,
        })
    }
}

impl From<FragmentSequenceError> for SessionError {
    fn from(error: FragmentSequenceError) -> Self {
        use FragmentSequenceError as F;
        use SessionErrorCode as S;
        Self::new(match error {
            F::Duplicate => S::FragmentDuplicate,
            F::Reordered | F::DifferentMessage => S::FragmentReordered,
            F::Overlap => S::FragmentOverlap,
            F::Invalid | F::Complete => S::FragmentTruncated,
        })
    }
}

impl From<HandshakeRejectReason> for SessionError {
    fn from(error: HandshakeRejectReason) -> Self {
        use HandshakeRejectReason as H;
        use SessionErrorCode as S;
        Self::new(match error {
            H::MalformedPreface | H::OfferTooLarge => S::MalformedPreface,
            H::UnsupportedVersion => S::UnsupportedVersion,
            H::MalformedOffer | H::ResourceExhausted => S::MalformedHandshake,
            H::PurposeMismatch => S::PurposeMismatch,
            H::PurposeClassMismatch => S::PurposeClassMismatch,
            H::RoleMismatch => S::RoleMismatch,
            H::ServiceMismatch => S::ServiceMismatch,
            H::SchemaMismatch => S::SchemaMismatch,
            H::NoiseProfileMismatch => S::AuthenticationFailed,
            H::LimitProfileMismatch => S::LimitMismatch,
            H::ChannelBindingMismatch => S::ChannelBindingMismatch,
            H::GenerationMismatch => S::GenerationMismatch,
            H::AttachmentPolicyMismatch => S::AttachmentPolicyMismatch,
            H::IdentityEvidenceMismatch => S::IdentityEvidenceMismatch,
            H::AuthenticationFailed => S::AuthenticationFailed,
            H::HandshakeTimeout => S::HandshakeTimeout,
            H::BootstrapExpired => S::BootstrapExpired,
            H::BootstrapReplayed => S::BootstrapReplayed,
            H::BootstrapOperationMismatch => S::BootstrapOperationMismatch,
        })
    }
}

impl From<TransportError> for SessionError {
    fn from(error: TransportError) -> Self {
        use SessionErrorCode as S;
        use TransportError as T;
        Self::new(match error {
            T::Disconnected => S::SessionDisconnected,
            T::WouldBlock => S::QueueBackpressure,
            T::Truncated => S::RecordTruncated,
            T::LimitExceeded => S::ReassemblyLimitExceeded,
            T::InvalidAttachment => S::AttachmentDescriptorMismatch,
            T::Other => S::InternalInvariant,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_exposes_closed_class_and_generic_remediation() {
        let policy = SessionError::new(SessionErrorCode::PolicyDenied);
        assert_eq!(policy.class(), SessionErrorClass::Authorization);
        assert_eq!(policy.remediation(), Remediation::RepairConfiguration);
        assert_eq!(
            policy.to_string(),
            "session-error class=authorization code=policy-denied remediation=repair-configuration"
        );

        let generation = SessionError::new(SessionErrorCode::GenerationMismatch);
        assert_eq!(generation.class(), SessionErrorClass::Generation);
        assert_eq!(generation.remediation(), Remediation::ReplaceGeneration);

        let cancelled = SessionError::new(SessionErrorCode::Cancelled);
        assert_eq!(cancelled.class(), SessionErrorClass::Cancellation);
        assert_eq!(cancelled.remediation(), Remediation::None);
    }
}
