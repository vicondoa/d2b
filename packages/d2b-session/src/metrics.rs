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
        SessionErrorCode::PolicyDenied => MetricReason::PolicyDenied,
        SessionErrorCode::AuthenticationFailed => MetricReason::Authentication,
        SessionErrorCode::TranscriptMismatch => MetricReason::TranscriptMismatch,
        SessionErrorCode::PurposeMismatch | SessionErrorCode::PurposeClassMismatch => {
            MetricReason::PurposeMismatch
        }
        SessionErrorCode::RoleMismatch => MetricReason::RoleMismatch,
        SessionErrorCode::SchemaMismatch => MetricReason::SchemaMismatch,
        SessionErrorCode::LimitMismatch | SessionErrorCode::ReassemblyLimitExceeded => {
            MetricReason::LimitMismatch
        }
        SessionErrorCode::ChannelBindingMismatch => MetricReason::ChannelBindingMismatch,
        SessionErrorCode::RecordReplay => MetricReason::Replay,
        SessionErrorCode::RecordTruncated
        | SessionErrorCode::FragmentTruncated
        | SessionErrorCode::AttachmentTruncated
        | SessionErrorCode::AttachmentControlTruncated => MetricReason::Truncation,
        SessionErrorCode::DeadlineInvalid | SessionErrorCode::DeadlineExpired => {
            MetricReason::Deadline
        }
        SessionErrorCode::Cancelled => MetricReason::Cancellation,
        SessionErrorCode::QueueBackpressure => MetricReason::Backpressure,
        SessionErrorCode::AttachmentCreditExceeded | SessionErrorCode::ControlResourceExhausted => {
            MetricReason::CreditExhausted
        }
        SessionErrorCode::KeepaliveTimeout => MetricReason::KeepaliveTimeout,
        SessionErrorCode::SessionDisconnected => MetricReason::Transport,
        _ => MetricReason::InternalInvariant,
    }
}
