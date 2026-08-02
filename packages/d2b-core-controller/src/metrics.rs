//! Core-controller metric inventory.

use d2b_telemetry::{MetricDescriptor, meter_registry::label};

/// Core-controller metrics.
pub const METRIC_INVENTORY: &[&str] = &[
    "d2b_controller_reconcile_total",
    "d2b_controller_reconcile_duration_seconds",
    "d2b_controller_queue_depth",
    "d2b_controller_hint_to_handler_seconds",
    "d2b_controller_watch_revision_lag",
];

/// Closed handler catalog used by all controller metrics.
pub const HANDLERS: &[&str] = &[
    "configuration",
    "api_catalog",
    "authz",
    "provider",
    "controller_registration",
    "ownership",
    "watch_maintenance",
    "ephemeral_cleanup",
    "zone_link",
    "budget",
    "store_lifecycle",
    "system_core_host",
    "system_core_user",
];

/// Commit-to-handler boundaries. The 5 ms boundary is intentional.
pub const HINT_TO_HANDLER_BUCKETS_SECONDS: &[f64] =
    &[0.001, 0.002, 0.005, 0.010, 0.015, 0.020, 0.030, 0.050];

/// Descriptor for reconcile counts.
pub fn reconcile_descriptor() -> MetricDescriptor {
    MetricDescriptor::new(
        "d2b_controller_reconcile_total",
        [
            label("handler", HANDLERS),
            label("outcome", &["ok", "requeue", "conflict", "error"]),
        ],
    )
}

/// Descriptor for the hint latency histogram.
pub fn hint_descriptor() -> MetricDescriptor {
    MetricDescriptor::new(
        "d2b_controller_hint_to_handler_seconds",
        [label("handler", HANDLERS)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_telemetry::validate_descriptor;

    #[test]
    fn handler_set_and_latency_target_are_pinned() {
        validate_descriptor(&reconcile_descriptor()).unwrap();
        validate_descriptor(&hint_descriptor()).unwrap();
        assert!(HINT_TO_HANDLER_BUCKETS_SECONDS.contains(&0.005));
        assert!(!HANDLERS.iter().any(|handler| handler.contains("name")));
    }
}
