//! Resource-store metric inventory and bounded emitter adapter.

use std::collections::BTreeMap;

use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, IdentityCanaries, MetricDescriptor, MetricPolicyError, Signal,
    encode_frame, meter_registry::label, validate_data_point,
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

/// Store-write histogram boundaries.
pub const WRITE_BUCKETS_SECONDS: &[f64] = &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 1.0];
/// Store-read histogram boundaries.
pub const READ_BUCKETS_SECONDS: &[f64] = &[0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1];
/// Group-commit size boundaries.
pub const GROUP_COMMIT_BUCKETS: &[f64] = &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];

/// Label-safe store metric descriptor.
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
            Self::ReadDuration => vec![label("op", &["get", "list", "scan"])],
            Self::Conflict => vec![label(
                "resource_type",
                &[
                    "Zone",
                    "Provider",
                    "Host",
                    "Guest",
                    "Process",
                    "Credential",
                    "Volume",
                    "Network",
                    "Device",
                    "vendor",
                ],
            )],
            Self::CompactionDuration | Self::BackupDuration => {
                vec![label("outcome", &["ok", "error"])]
            }
            Self::QueueDepth => vec![label("queue", &["write", "read"])],
            Self::GroupCommitSize | Self::WatchActive | Self::Revision => Vec::new(),
        };
        MetricDescriptor::new(self.name(), labels)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
