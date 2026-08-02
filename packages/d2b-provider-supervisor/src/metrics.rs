//! Process Provider metric inventory.

use d2b_telemetry::MetricDescriptor;
use d2b_telemetry::meter_registry::label;

/// Process metric names.
pub const METRIC_INVENTORY: &[&str] = &[
    "d2b_process_launch_total",
    "d2b_process_launch_duration_seconds",
    "d2b_process_active",
    "d2b_process_restart_total",
    "d2b_process_adoption_total",
    "d2b_process_pidfd_active",
    "d2b_process_stop_duration_seconds",
    "d2b_process_ready_duration_seconds",
];

/// Commit-to-first-spawn boundaries. The 20 ms boundary is intentional.
pub const LAUNCH_BUCKETS_SECONDS: &[f64] = &[
    0.001, 0.005, 0.010, 0.015, 0.020, 0.030, 0.050, 0.1, 0.5, 2.0,
];

/// Closed process Provider labels.
pub const PROVIDERS: &[&str] = &["minijail", "systemd"];

/// Build the launch metric descriptor.
pub fn launch_descriptor() -> MetricDescriptor {
    MetricDescriptor::new(
        "d2b_process_launch_total",
        [
            label("provider", PROVIDERS),
            label("domain", &["system", "user"]),
            label("outcome", &["ok", "error", "quota"]),
        ],
    )
}

/// Build the stop metric descriptor.
pub fn stop_descriptor() -> MetricDescriptor {
    MetricDescriptor::new(
        "d2b_process_stop_duration_seconds",
        [
            label("provider", PROVIDERS),
            label("stop_class", &["graceful", "forced"]),
            label("outcome", &["ok", "error"]),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_telemetry::validate_descriptor;

    #[test]
    fn launch_metrics_use_provider_not_vm_identity() {
        validate_descriptor(&launch_descriptor()).unwrap();
        assert!(LAUNCH_BUCKETS_SECONDS.contains(&0.020));
        assert!(!METRIC_INVENTORY.iter().any(|name| name.contains("vm_")));
    }
}
