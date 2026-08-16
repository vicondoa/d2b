//! Bounded audio audit and telemetry projections.

use std::collections::BTreeMap;

/// Closed audio telemetry operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTelemetryOperation {
    /// AudioService reconciliation.
    ServiceReconcile,
    /// AudioBinding reconciliation.
    BindingReconcile,
    /// Microphone transition.
    MicrophoneTransition,
}

/// Redacted, bounded telemetry record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTelemetryRecord {
    /// Closed operation.
    pub operation: AudioTelemetryOperation,
    /// Stable bounded outcome code.
    pub outcome: &'static str,
    /// Closed low-cardinality labels.
    pub labels: BTreeMap<String, String>,
}

/// Telemetry validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTelemetryError {
    /// A label key or value was not in the closed semantic domain.
    InvalidLabel,
}

impl core::fmt::Display for AudioTelemetryError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("audio-telemetry-label-invalid")
    }
}

impl std::error::Error for AudioTelemetryError {}

/// Construct bounded telemetry after the durable operation boundary.
pub fn record(
    operation: AudioTelemetryOperation,
    outcome: &'static str,
    labels: impl IntoIterator<Item = (String, String)>,
) -> Result<AudioTelemetryRecord, AudioTelemetryError> {
    let labels = labels.into_iter().collect::<BTreeMap<_, _>>();
    for (key, value) in &labels {
        if !matches!(key.as_str(), "role" | "channel" | "outcome")
            || value.len() > 32
            || value.contains('/')
        {
            return Err(AudioTelemetryError::InvalidLabel);
        }
    }
    Ok(AudioTelemetryRecord {
        operation,
        outcome,
        labels,
    })
}
