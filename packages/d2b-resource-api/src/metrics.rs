//! Resource API metric inventory and trace-safe projections.

use std::collections::BTreeMap;

use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, IdentityCanaries, MetricDescriptor, MetricPolicyError, Signal,
    TraceContext, encode_frame, meter_registry::label, validate_data_point,
};

/// Resource API metric names.
pub const METRIC_INVENTORY: &[&str] = &[
    "d2b_api_request_total",
    "d2b_api_request_duration_seconds",
    "d2b_api_watch_active",
    "d2b_api_admission_rejected_total",
];

/// API verbs in the closed service catalog.
pub const API_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",
    "use-credential",
    "admin-credential",
];

/// Resource API metric family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiMetric {
    /// Number of completed API requests.
    RequestTotal,
    /// Duration of one API request.
    RequestDuration,
    /// Number of active watches.
    WatchActive,
    /// Number of rejected admissions.
    AdmissionRejected,
}

impl ApiMetric {
    /// Stable metric name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::RequestTotal => "d2b_api_request_total",
            Self::RequestDuration => "d2b_api_request_duration_seconds",
            Self::WatchActive => "d2b_api_watch_active",
            Self::AdmissionRejected => "d2b_api_admission_rejected_total",
        }
    }

    /// Descriptor with the closed API label domains.
    pub fn descriptor(self) -> MetricDescriptor {
        let labels = match self {
            Self::RequestTotal => vec![
                label("verb", API_VERBS),
                label(
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
                ),
                label(
                    "outcome",
                    &[
                        "ok",
                        "conflict",
                        "invalid",
                        "denied",
                        "not_found",
                        "quota",
                        "error",
                    ],
                ),
            ],
            Self::RequestDuration => vec![
                label("verb", API_VERBS),
                label(
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
                ),
            ],
            Self::WatchActive => Vec::new(),
            Self::AdmissionRejected => vec![label(
                "reason",
                &["auth", "quota", "conflict", "invalid", "schema"],
            )],
        };
        MetricDescriptor::new(self.name(), labels)
    }
}

/// Request metric adapter.
#[derive(Clone, Debug)]
pub struct ApiMetrics {
    emitter: BoundedEmitter,
}

impl ApiMetrics {
    /// Construct an adapter.
    pub fn new(emitter: BoundedEmitter) -> Self {
        Self { emitter }
    }

    /// Request counter descriptor.
    pub fn request_descriptor() -> MetricDescriptor {
        ApiMetric::RequestTotal.descriptor()
    }

    /// Request duration descriptor.
    pub fn request_duration_descriptor() -> MetricDescriptor {
        ApiMetric::RequestDuration.descriptor()
    }

    /// Request duration boundaries.
    pub const fn request_duration_buckets() -> &'static [f64] {
        &[0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5]
    }

    /// Emit a request metric and preserve the incoming trace context in the
    /// frame as an opaque correlation value only.
    pub fn request(
        &self,
        labels: BTreeMap<String, String>,
        duration_seconds: f64,
        trace: Option<&TraceContext>,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.emit_value(
            ApiMetric::RequestTotal,
            labels.clone(),
            serde_json::json!({
                "count": 1,
                "duration_seconds": duration_seconds,
                "trace_id": trace.map(TraceContext::trace_id),
            }),
        )?;
        let duration_labels = labels
            .into_iter()
            .filter(|(key, _)| key != "outcome")
            .collect();
        self.emit_value(
            ApiMetric::RequestDuration,
            duration_labels,
            serde_json::json!(duration_seconds),
        )
    }

    /// Emit one request metric without a trace context.
    pub fn observe(
        &self,
        metric: ApiMetric,
        labels: BTreeMap<String, String>,
        value: f64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.emit_value(metric, labels, serde_json::json!(value))
    }

    /// Emit the current active-watch gauge.
    pub fn watch_active(&self, active: u64) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(ApiMetric::WatchActive, BTreeMap::new(), active as f64)
    }

    /// Emit one bounded admission rejection.
    pub fn admission_rejected(
        &self,
        reason: impl Into<String>,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ApiMetric::AdmissionRejected,
            BTreeMap::from([("reason".to_owned(), reason.into())]),
            1.0,
        )
    }

    fn emit_value(
        &self,
        metric: ApiMetric,
        labels: BTreeMap<String, String>,
        value: serde_json::Value,
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
    use d2b_telemetry::validate_descriptor;

    #[test]
    fn request_inventory_has_closed_verbs_and_no_realm_label() {
        let descriptor = ApiMetrics::request_descriptor();
        assert!(
            descriptor
                .labels()
                .iter()
                .all(|label| label.key() != "realm")
        );
        assert!(ApiMetrics::request_duration_buckets().contains(&0.005));
        assert_eq!(METRIC_INVENTORY.len(), 4);
    }

    #[test]
    fn every_api_metric_has_a_closed_descriptor_and_emit_port() {
        let emitter = BoundedEmitter::new("/nonexistent", 2048).unwrap();
        let metrics = ApiMetrics::new(emitter);
        let labels = BTreeMap::from([
            ("verb".to_owned(), "get".to_owned()),
            ("resource_type".to_owned(), "Provider".to_owned()),
            ("outcome".to_owned(), "ok".to_owned()),
        ]);
        for metric in [
            ApiMetric::RequestTotal,
            ApiMetric::RequestDuration,
            ApiMetric::WatchActive,
            ApiMetric::AdmissionRejected,
        ] {
            validate_descriptor(&metric.descriptor()).unwrap();
        }
        metrics.request(labels, 0.005, None).unwrap();
        metrics.watch_active(1).unwrap();
        metrics.admission_rejected("auth").unwrap();
    }
}
