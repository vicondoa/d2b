use d2b_contracts::v2_component_session::MetricLabels;

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
