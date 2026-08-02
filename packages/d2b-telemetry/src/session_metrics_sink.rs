//! Adapter shape for ComponentSession metrics.

use std::collections::BTreeMap;

use crate::{
    emitter::{BoundedEmitter, EmitOutcome, Signal, encode_frame},
    metric_label_policy::{IdentityCanaries, MetricDescriptor, validate_data_point},
};

/// Session event inventory kept independent of the wire crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMetricEvent {
    /// A completed handshake.
    Connect,
    /// A reconnect attempt.
    Reconnect,
    /// A protected record.
    Record,
    /// A live session gauge update.
    Active,
}

impl SessionMetricEvent {
    /// Stable metric name.
    pub const fn metric_name(self) -> &'static str {
        match self {
            Self::Connect => "d2b_session_connect_total",
            Self::Reconnect => "d2b_session_reconnect_total",
            Self::Record => "d2b_session_record_total",
            Self::Active => "d2b_session_active",
        }
    }
}

/// Lightweight session sink that serializes validated frames to an emitter.
#[derive(Clone, Debug)]
pub struct SessionMetricsSink {
    emitter: BoundedEmitter,
}

impl SessionMetricsSink {
    /// Construct an adapter around the bounded emitter.
    pub fn new(emitter: BoundedEmitter) -> Self {
        Self { emitter }
    }

    /// Record a session event with closed labels.
    pub fn record(
        &self,
        event: SessionMetricEvent,
        labels: BTreeMap<String, String>,
    ) -> Result<EmitOutcome, SessionMetricsError> {
        let descriptor = descriptor_for(event)?;
        validate_data_point(&descriptor, &labels, &IdentityCanaries::default())
            .map_err(SessionMetricsError::Policy)?;
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": event.metric_name(),
                "labels": labels,
                "value": 1,
            }),
        )
        .map_err(SessionMetricsError::Encode)?;
        self.emitter
            .emit(Signal::Metric, &frame)
            .map_err(SessionMetricsError::Emitter)
    }

    /// Borrow the underlying emitter for health reporting.
    pub fn emitter(&self) -> &BoundedEmitter {
        &self.emitter
    }
}

/// Session sink failure.
#[derive(Debug)]
pub enum SessionMetricsError {
    /// Label policy rejected the frame.
    Policy(crate::metric_label_policy::MetricPolicyError),
    /// Frame encoding failed.
    Encode(std::io::Error),
    /// Emitter failed.
    Emitter(crate::emitter::EmitterError),
}

impl core::fmt::Display for SessionMetricsError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Policy(_) => "session-metric-policy-rejected",
            Self::Encode(_) => "session-metric-encode-failed",
            Self::Emitter(_) => "session-metric-emitter-failed",
        })
    }
}

impl std::error::Error for SessionMetricsError {}

fn descriptor_for(event: SessionMetricEvent) -> Result<MetricDescriptor, SessionMetricsError> {
    use crate::meter_registry::label;
    let labels = match event {
        SessionMetricEvent::Connect => vec![
            label("profile", &["NN", "KK", "IKpsk2"]),
            label("purpose_class", &["local", "enrolled", "bootstrap"]),
            label(
                "outcome",
                &["ok", "auth", "transcript", "policy", "timeout", "error"],
            ),
        ],
        SessionMetricEvent::Reconnect => vec![label("outcome", &["ok", "error", "abandoned"])],
        SessionMetricEvent::Record => vec![
            label("direction", &["send", "recv"]),
            label("kind", &["control", "ttrpc", "stream", "attachment"]),
        ],
        SessionMetricEvent::Active => vec![label("transport", &["unix", "vsock", "zone_link"])],
    };
    Ok(MetricDescriptor::new(event.metric_name(), labels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn session_metric_descriptors_have_no_resource_identity_labels() {
        let emitter = BoundedEmitter::new(PathBuf::from("/nonexistent"), 128).unwrap();
        let sink = SessionMetricsSink::new(emitter);
        let result = sink.record(
            SessionMetricEvent::Connect,
            BTreeMap::from([
                ("profile".to_owned(), "NN".to_owned()),
                ("purpose_class".to_owned(), "local".to_owned()),
                ("outcome".to_owned(), "ok".to_owned()),
            ]),
        );
        assert!(result.is_ok());
    }
}
