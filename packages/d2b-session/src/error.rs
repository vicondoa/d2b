use std::{error::Error, fmt};

use d2b_contracts::v2_component_session::{
    BinaryError, ContractError, FragmentSequenceError, HandshakeRejectReason, SequenceError,
    SessionErrorCode,
};

use crate::TransportError;

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionError {
    code: SessionErrorCode,
}

impl SessionError {
    pub const fn new(code: SessionErrorCode) -> Self {
        Self { code }
    }

    pub const fn code(self) -> SessionErrorCode {
        self.code
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
        formatter.write_str(self.code.as_str())
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
            T::Disconnected | T::WouldBlock => S::SessionDisconnected,
            T::Truncated => S::RecordTruncated,
            T::LimitExceeded => S::ReassemblyLimitExceeded,
            T::InvalidAttachment => S::AttachmentDescriptorMismatch,
            T::Other => S::InternalInvariant,
        })
    }
}
