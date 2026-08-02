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
        MetricDescriptor::new(
            "d2b_api_request_total",
            [
                label("verb", API_VERBS),
                label(
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
        )
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
        let descriptor = Self::request_descriptor();
        validate_data_point(&descriptor, &labels, &IdentityCanaries::default())?;
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_api_request_total",
                "labels": labels,
                "duration_seconds": duration_seconds,
                "trace_id": trace.map(TraceContext::trace_id),
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
}
