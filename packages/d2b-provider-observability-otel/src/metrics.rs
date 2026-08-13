//! Observability Provider self-metric inventory.

use crate::metric_policy::{MetricDescriptor, canonical_descriptor};

/// Provider self-metric names.
pub const SELF_METRICS: &[&str] = &[
    "d2b_telemetry_drop_total",
    "d2b_telemetry_export_total",
    "d2b_otel_ingress_policy_total",
];

/// Build the ingress policy metric descriptor.
pub fn ingress_policy_descriptor() -> MetricDescriptor {
    canonical_descriptor("d2b_otel_ingress_policy_total")
        .expect("self metric must be in the canonical descriptor registry")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate_descriptor;

    #[test]
    fn self_metric_descriptor_uses_closed_labels() {
        validate_descriptor(&ingress_policy_descriptor()).unwrap();
        assert!(
            SELF_METRICS
                .iter()
                .all(|name| { !name.contains("vm_state") && canonical_descriptor(name).is_some() })
        );
    }
}
