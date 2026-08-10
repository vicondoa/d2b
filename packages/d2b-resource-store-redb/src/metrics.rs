//! Resource-store metric inventory and bounded emitter adapter.

use std::collections::BTreeMap;

use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, IdentityCanaries, MetricDescriptor, MetricPolicyError, Signal,
    TraceContext, emitter::encode_frame, meter_registry::label, validate_data_point,
};

/// Store metric names owned by this backend.
pub const METRIC_INVENTORY: &[&str] = &[
    "d2b_store_write_duration_seconds",
    "d2b_store_read_duration_seconds",
    "d2b_store_group_commit_size",
    "d2b_store_conflict_total",
    "d2b_store_watch_active",
    "d2b_store_revision",
    "d2b_store_compaction_duration_seconds",
    "d2b_store_backup_duration_seconds",
    "d2b_store_queue_depth",
];

/// Store metric for one bounded observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreMetric {
    /// A write duration observation.
    WriteDuration,
    /// A read duration observation.
    ReadDuration,
    /// A group commit size observation.
    GroupCommitSize,
    /// A conflict count.
    Conflict,
    /// A watch gauge.
    WatchActive,
    /// A revision gauge.
    Revision,
    /// A compaction duration observation.
    CompactionDuration,
    /// A backup duration observation.
    BackupDuration,
    /// A queue depth gauge.
    QueueDepth,
}

impl StoreMetric {
    /// Stable metric name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::WriteDuration => "d2b_store_write_duration_seconds",
            Self::ReadDuration => "d2b_store_read_duration_seconds",
            Self::GroupCommitSize => "d2b_store_group_commit_size",
            Self::Conflict => "d2b_store_conflict_total",
            Self::WatchActive => "d2b_store_watch_active",
            Self::Revision => "d2b_store_revision",
            Self::CompactionDuration => "d2b_store_compaction_duration_seconds",
            Self::BackupDuration => "d2b_store_backup_duration_seconds",
            Self::QueueDepth => "d2b_store_queue_depth",
        }
    }

    /// Descriptor with the closed label domains from ADR 0046.
    pub fn descriptor(self) -> MetricDescriptor {
        let labels = match self {
            Self::WriteDuration => vec![
                label("kind", &["single", "group"]),
                label("outcome", &["ok", "conflict", "error"]),
            ],
            Self::ReadDuration => vec![label("operation", &["get", "list", "scan"])],
            Self::Conflict => vec![label(
                "resource_type",
                &[
                    "Zone",
                    "ZoneLink",
                    "Provider",
                    "Role",
                    "RoleBinding",
                    "Quota",
                    "Host",
                    "Guest",
                    "Process",
                    "EphemeralProcess",
                    "Volume",
                    "Network",
                    "Device",
                    "User",
                    "Credential",
                    "Endpoint",
                    "ResourceExport",
                    "ResourceImport",
                    "vendor",
                ],
            )],
            Self::CompactionDuration | Self::BackupDuration => {
                vec![label("outcome", &["ok", "error"])]
            }
            Self::QueueDepth => vec![label("operation", &["write", "read"])],
            Self::GroupCommitSize | Self::WatchActive | Self::Revision => Vec::new(),
        };
        MetricDescriptor::new(self.name(), labels)
    }
}

/// Store-write histogram boundaries.
pub const WRITE_BUCKETS_SECONDS: &[f64] = &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 1.0];
/// Store-read histogram boundaries.
pub const READ_BUCKETS_SECONDS: &[f64] = &[0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1];
/// Group-commit size boundaries.
pub const GROUP_COMMIT_BUCKETS: &[f64] = &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];

/// Store metric adapter. It never embeds an OTEL SDK.
#[derive(Clone, Debug)]
pub struct StoreMetrics {
    emitter: BoundedEmitter,
}

impl StoreMetrics {
    /// Construct an adapter.
    pub fn new(emitter: BoundedEmitter) -> Self {
        Self { emitter }
    }

    /// Emit a validated metric frame.
    pub fn observe(
        &self,
        metric: StoreMetric,
        labels: BTreeMap<String, String>,
        value: f64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        let descriptor = metric.descriptor();
        validate_data_point(&descriptor, &labels, &IdentityCanaries::default())?;
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": metric.name(),
                "labels": labels,
                "value": value,
            }),
        )
        .map_err(|_| MetricPolicyError::DescriptorMalformed)?;
        self.emitter
            .emit(Signal::Metric, &frame)
            .map_err(|_| MetricPolicyError::DescriptorMalformed)
    }
}

/// Production telemetry port used by the store actor and read workers.
pub trait StoreTelemetry: Send + Sync {
    /// Record one policy-validated metric observation.
    fn metric(&self, metric: StoreMetric, labels: BTreeMap<String, String>, value: f64);

    /// Record one bounded span projection.
    fn span(
        &self,
        name: &'static str,
        fields: BTreeMap<String, String>,
        trace: Option<TraceContext>,
    );
}

/// No-op telemetry port used until the Zone observability Provider is present.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopStoreTelemetry;

impl StoreTelemetry for NoopStoreTelemetry {
    fn metric(&self, _metric: StoreMetric, _labels: BTreeMap<String, String>, _value: f64) {}

    fn span(
        &self,
        _name: &'static str,
        _fields: BTreeMap<String, String>,
        _trace: Option<TraceContext>,
    ) {
    }
}

/// Bounded-emitter implementation of the store telemetry port.
#[derive(Clone, Debug)]
pub struct EmitterStoreTelemetry {
    metrics: StoreMetrics,
    emitter: BoundedEmitter,
}

impl EmitterStoreTelemetry {
    /// Construct an emitter-backed store telemetry port.
    pub fn new(emitter: BoundedEmitter) -> Self {
        Self {
            metrics: StoreMetrics::new(emitter.clone()),
            emitter,
        }
    }

    /// Borrow the metric adapter.
    pub const fn metrics(&self) -> &StoreMetrics {
        &self.metrics
    }
}

impl StoreTelemetry for EmitterStoreTelemetry {
    fn metric(&self, metric: StoreMetric, labels: BTreeMap<String, String>, value: f64) {
        let _ = self.metrics.observe(metric, labels, value);
    }

    fn span(
        &self,
        name: &'static str,
        fields: BTreeMap<String, String>,
        trace: Option<TraceContext>,
    ) {
        if let Ok(span) = crate::tracing::StoreSpan::new(name, fields, trace) {
            let _ = span.emit(&self.emitter);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_telemetry::validate_descriptor;

    #[test]
    fn inventory_has_no_vm_identity_dimension() {
        assert!(
            METRIC_INVENTORY
                .iter()
                .all(|metric| !metric.contains("vm_state"))
        );
        let descriptor = StoreMetric::WriteDuration.descriptor();
        assert!(descriptor.labels().iter().all(|label| label.key() != "vm"));
        assert!(WRITE_BUCKETS_SECONDS.contains(&0.01));
    }

    #[test]
    fn every_store_metric_has_a_valid_descriptor() {
        for metric in [
            StoreMetric::WriteDuration,
            StoreMetric::ReadDuration,
            StoreMetric::GroupCommitSize,
            StoreMetric::Conflict,
            StoreMetric::WatchActive,
            StoreMetric::Revision,
            StoreMetric::CompactionDuration,
            StoreMetric::BackupDuration,
            StoreMetric::QueueDepth,
        ] {
            validate_descriptor(&metric.descriptor())
                .unwrap_or_else(|error| panic!("{}: {error:?}", metric.name()));
        }
    }

    #[test]
    fn emitter_telemetry_has_a_non_test_metric_and_span_port() {
        let emitter = BoundedEmitter::new("/nonexistent", 1024).unwrap();
        let telemetry = EmitterStoreTelemetry::new(emitter);
        telemetry.metric(
            StoreMetric::QueueDepth,
            BTreeMap::from([("operation".to_owned(), "write".to_owned())]),
            1.0,
        );
        telemetry.span(
            crate::tracing::STORE_WRITE_SPAN,
            BTreeMap::from([("kind".to_owned(), "single".to_owned())]),
            None,
        );
    }
}
