use d2b_contracts::v3::component_session::{MetricLabels, MetricReason, SessionErrorCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricEvent {
    ActiveSessions,
    Handshake,
    ConnectAttempt,
    ReconnectAttempt,
    Close,
    ControlCreditExhaustion,
    QueueDepth,
    QueueCapacity,
    SchedulingDelay,
    RejectedRecord,
    CleanupFailure,
}

pub trait MetricsSink: Send + Sync {
    fn record(&self, event: MetricEvent, labels: MetricLabels, value: u64);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopMetrics;

impl MetricsSink for NoopMetrics {
    fn record(&self, _event: MetricEvent, _labels: MetricLabels, _value: u64) {}
}

pub(crate) const fn reason_for_error(code: SessionErrorCode) -> MetricReason {
    match code {
        SessionErrorCode::MalformedPreface
        | SessionErrorCode::MalformedHandshake
        | SessionErrorCode::RecordMalformed
        | SessionErrorCode::RecordOutOfOrder
        | SessionErrorCode::FragmentDuplicate
        | SessionErrorCode::FragmentReordered
        | SessionErrorCode::FragmentOverlap
        | SessionErrorCode::InvalidChannel
        | SessionErrorCode::UnknownControl
        | SessionErrorCode::AttachmentCountMismatch
        | SessionErrorCode::AttachmentDescriptorMismatch
        | SessionErrorCode::AttachmentObjectMismatch
        | SessionErrorCode::AttachmentAccessMismatch
        | SessionErrorCode::AttachmentMissingCloexec => MetricReason::Malformed,
        SessionErrorCode::UnsupportedVersion => MetricReason::UnsupportedVersion,
        SessionErrorCode::PolicyDenied => MetricReason::PolicyDenied,
        SessionErrorCode::AuthenticationFailed => MetricReason::Authentication,
        SessionErrorCode::TranscriptMismatch => MetricReason::TranscriptMismatch,
        SessionErrorCode::PurposeMismatch
        | SessionErrorCode::PurposeClassMismatch
        | SessionErrorCode::ServiceMismatch => MetricReason::PurposeMismatch,
        SessionErrorCode::RoleMismatch => MetricReason::RoleMismatch,
        SessionErrorCode::SchemaMismatch => MetricReason::SchemaMismatch,
        SessionErrorCode::LimitMismatch
        | SessionErrorCode::AttachmentPolicyMismatch
        | SessionErrorCode::ReassemblyLimitExceeded => MetricReason::LimitMismatch,
        SessionErrorCode::ChannelBindingMismatch => MetricReason::ChannelBindingMismatch,
        SessionErrorCode::GenerationMismatch => MetricReason::GenerationMismatch,
        SessionErrorCode::IdentityEvidenceMismatch => MetricReason::IdentityEvidenceMismatch,
        SessionErrorCode::HandshakeTimeout => MetricReason::Deadline,
        SessionErrorCode::RecordReplay
        | SessionErrorCode::RequestIdDuplicate
        | SessionErrorCode::BootstrapReplayed => MetricReason::Replay,
        SessionErrorCode::RecordTruncated
        | SessionErrorCode::FragmentTruncated
        | SessionErrorCode::AttachmentTruncated
        | SessionErrorCode::AttachmentControlTruncated => MetricReason::Truncation,
        SessionErrorCode::DeadlineInvalid
        | SessionErrorCode::DeadlineExpired
        | SessionErrorCode::BootstrapExpired => MetricReason::Deadline,
        SessionErrorCode::Cancelled => MetricReason::Cancellation,
        SessionErrorCode::QueueBackpressure => MetricReason::Backpressure,
        SessionErrorCode::AttachmentCreditExceeded | SessionErrorCode::ControlResourceExhausted => {
            MetricReason::CreditExhausted
        }
        SessionErrorCode::KeepaliveTimeout => MetricReason::KeepaliveTimeout,
        SessionErrorCode::SessionDisconnected => MetricReason::Transport,
        SessionErrorCode::BootstrapOperationMismatch => MetricReason::PolicyDenied,
        SessionErrorCode::SubjectMismatch => MetricReason::Authentication,
        SessionErrorCode::TransportMismatch => MetricReason::TransportMismatch,
        SessionErrorCode::NonceExhausted
        | SessionErrorCode::SchedulerStalled
        | SessionErrorCode::ArithmeticOverflow
        | SessionErrorCode::InternalInvariant => MetricReason::InternalInvariant,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_handshake_failures_have_specific_closed_reasons() {
        let cases = [
            (SessionErrorCode::HandshakeTimeout, MetricReason::Deadline),
            (
                SessionErrorCode::MalformedHandshake,
                MetricReason::Malformed,
            ),
            (
                SessionErrorCode::UnsupportedVersion,
                MetricReason::UnsupportedVersion,
            ),
            (
                SessionErrorCode::IdentityEvidenceMismatch,
                MetricReason::IdentityEvidenceMismatch,
            ),
            (
                SessionErrorCode::GenerationMismatch,
                MetricReason::GenerationMismatch,
            ),
            (
                SessionErrorCode::TransportMismatch,
                MetricReason::TransportMismatch,
            ),
        ];

        for (code, expected) in cases {
            assert_eq!(reason_for_error(code), expected, "{}", code.as_str());
            assert_ne!(reason_for_error(code), MetricReason::InternalInvariant);
        }
    }
}
