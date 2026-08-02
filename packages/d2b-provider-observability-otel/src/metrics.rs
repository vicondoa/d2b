//! Observability Provider self-metric inventory.

use d2b_telemetry::{MetricDescriptor, meter_registry::label};

/// Provider self-metric names.
pub const SELF_METRICS: &[&str] = &[
    "d2b_telemetry_drop_total",
    "d2b_telemetry_export_total",
    "d2b_otel_ingress_policy_total",
];

/// Build the ingress policy metric descriptor.
pub fn ingress_policy_descriptor() -> MetricDescriptor {
    MetricDescriptor::new(
        "d2b_otel_ingress_policy_total",
        [
            label(
                "ingress",
                &["emitter_unix", "otlp_unix", "otlp_vsock", "import_stream"],
            ),
            label("outcome", &["accepted", "rejected", "quarantined"]),
            label(
                "error_class",
                &[
                    "none",
                    "key_not_allowlisted",
                    "key_forbidden",
                    "key_suffix_forbidden",
                    "value_identity",
                    "malformed",
                    "oversize",
                ],
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_telemetry::validate_descriptor;

    #[test]
    fn self_metric_descriptor_uses_closed_labels() {
        validate_descriptor(&ingress_policy_descriptor()).unwrap();
        assert!(SELF_METRICS.iter().all(|name| !name.contains("vm_state")));
    }
}
