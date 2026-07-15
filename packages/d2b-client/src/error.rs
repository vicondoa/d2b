use std::{error::Error, fmt};

use d2b_contracts::{v2_component_session::SessionErrorCode, v2_services::ServiceContractError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteErrorKind {
    InvalidRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    ResourceExhausted,
    Unavailable,
    DeadlineExceeded,
    Cancelled,
    FailedPrecondition,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    Never,
    Safe,
    Observe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    InvalidTarget,
    RouteUnavailable,
    TransportPolicyMismatch,
    ConnectFailed,
    SessionEstablishment(SessionErrorCode),
    InvalidService,
    InvalidMethod,
    InvalidMetadata,
    DeadlineExpired,
    IdempotencyRequired,
    RetryLimitExceeded,
    Cancelled,
    SessionLost,
    TransportFailed,
    AmbiguousMutation,
    ContractViolation,
    ServiceContract(ServiceContractError),
    AttachmentMismatch,
    StreamLimitExceeded,
    StreamDetached,
    StreamClosed,
    Remote {
        kind: RemoteErrorKind,
        retry: RetryClass,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidTarget => "client-invalid-target",
            Self::RouteUnavailable => "client-route-unavailable",
            Self::TransportPolicyMismatch => "client-transport-policy-mismatch",
            Self::ConnectFailed => "client-connect-failed",
            Self::SessionEstablishment(code) => {
                return write!(formatter, "client-session-establishment-{}", code.as_str());
            }
            Self::InvalidService => "client-invalid-service",
            Self::InvalidMethod => "client-invalid-method",
            Self::InvalidMetadata => "client-invalid-metadata",
            Self::DeadlineExpired => "client-deadline-expired",
            Self::IdempotencyRequired => "client-idempotency-required",
            Self::RetryLimitExceeded => "client-retry-limit-exceeded",
            Self::Cancelled => "client-cancelled",
            Self::SessionLost => "client-session-lost",
            Self::TransportFailed => "client-transport-failed",
            Self::AmbiguousMutation => "client-ambiguous-mutation",
            Self::ContractViolation => "client-contract-violation",
            Self::ServiceContract(error) => {
                return write!(formatter, "client-{error}");
            }
            Self::AttachmentMismatch => "client-attachment-mismatch",
            Self::StreamLimitExceeded => "client-stream-limit-exceeded",
            Self::StreamDetached => "client-stream-detached",
            Self::StreamClosed => "client-stream-closed",
            Self::Remote { .. } => "client-remote-error",
        };
        formatter.write_str(message)
    }
}

impl Error for ClientError {}
