//! Process Provider metric inventory and bounded emitter adapter.

use std::collections::BTreeMap;

use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, IdentityCanaries, MetricDescriptor, MetricPolicyError, Signal,
    encode_frame, meter_registry::label, validate_data_point,
};

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

/// Process Provider metric family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessMetric {
    /// Launch count.
    LaunchTotal,
    /// Launch duration.
    LaunchDuration,
    /// Active process gauge.
    Active,
    /// Restart count.
    RestartTotal,
    /// Adoption count.
    AdoptionTotal,
    /// Active pidfd gauge.
    PidfdActive,
    /// Stop duration.
    StopDuration,
    /// Ready duration.
    ReadyDuration,
}

impl ProcessMetric {
    /// Stable metric name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::LaunchTotal => "d2b_process_launch_total",
            Self::LaunchDuration => "d2b_process_launch_duration_seconds",
            Self::Active => "d2b_process_active",
            Self::RestartTotal => "d2b_process_restart_total",
            Self::AdoptionTotal => "d2b_process_adoption_total",
            Self::PidfdActive => "d2b_process_pidfd_active",
            Self::StopDuration => "d2b_process_stop_duration_seconds",
            Self::ReadyDuration => "d2b_process_ready_duration_seconds",
        }
    }

    /// Closed descriptor for one process metric family.
    pub fn descriptor(self) -> MetricDescriptor {
        match self {
            Self::LaunchTotal => MetricDescriptor::new(
                self.name(),
                [
                    label("provider", PROVIDERS),
                    label("domain", &["system", "user"]),
                    label("outcome", &["ok", "error", "quota"]),
                ],
            ),
            Self::LaunchDuration | Self::ReadyDuration => MetricDescriptor::new(
                self.name(),
                [
                    label("provider", PROVIDERS),
                    label("domain", &["system", "user"]),
                ],
            ),
            Self::Active => MetricDescriptor::new(
                self.name(),
                [
                    label("provider", PROVIDERS),
                    label("domain", &["system", "user"]),
                ],
            ),
            Self::RestartTotal => MetricDescriptor::new(
                self.name(),
                [
                    label("provider", PROVIDERS),
                    label("class", &["exited", "signaled", "killed"]),
                ],
            ),
            Self::AdoptionTotal => MetricDescriptor::new(
                self.name(),
                [
                    label("provider", PROVIDERS),
                    label("outcome", &["ok", "quarantine", "error"]),
                ],
            ),
            Self::PidfdActive => MetricDescriptor::new(self.name(), []),
            Self::StopDuration => MetricDescriptor::new(
                self.name(),
                [
                    label("provider", PROVIDERS),
                    label("stop_class", &["graceful", "forced"]),
                    label("outcome", &["ok", "error"]),
                ],
            ),
        }
    }
}

/// Build the launch metric descriptor.
pub fn launch_descriptor() -> MetricDescriptor {
    ProcessMetric::LaunchTotal.descriptor()
}

/// Build the stop metric descriptor.
pub fn stop_descriptor() -> MetricDescriptor {
    ProcessMetric::StopDuration.descriptor()
}

/// Bounded metric adapter for Process Provider lifecycle callsites.
#[derive(Clone, Debug)]
pub struct ProcessMetrics {
    emitter: BoundedEmitter,
}

impl ProcessMetrics {
    /// Construct an emitter-backed process metric adapter.
    pub fn new(emitter: BoundedEmitter) -> Self {
        Self { emitter }
    }

    /// Emit one policy-validated process metric observation.
    pub fn observe(
        &self,
        metric: ProcessMetric,
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

    /// Emit a launch count and its commit-to-spawn duration.
    pub fn launch(
        &self,
        provider: impl Into<String>,
        domain: impl Into<String>,
        outcome: impl Into<String>,
        duration_seconds: f64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        let provider = provider.into();
        let domain = domain.into();
        let outcome = outcome.into();
        self.observe(
            ProcessMetric::LaunchTotal,
            BTreeMap::from([
                ("provider".to_owned(), provider.clone()),
                ("domain".to_owned(), domain.clone()),
                ("outcome".to_owned(), outcome),
            ]),
            1.0,
        )?;
        self.observe(
            ProcessMetric::LaunchDuration,
            BTreeMap::from([
                ("provider".to_owned(), provider),
                ("domain".to_owned(), domain),
            ]),
            duration_seconds,
        )
    }

    /// Emit one stop duration.
    pub fn stop(
        &self,
        provider: impl Into<String>,
        stop_class: impl Into<String>,
        outcome: impl Into<String>,
        duration_seconds: f64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ProcessMetric::StopDuration,
            BTreeMap::from([
                ("provider".to_owned(), provider.into()),
                ("stop_class".to_owned(), stop_class.into()),
                ("outcome".to_owned(), outcome.into()),
            ]),
            duration_seconds,
        )
    }

    /// Emit one adoption result.
    pub fn adoption(
        &self,
        provider: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ProcessMetric::AdoptionTotal,
            BTreeMap::from([
                ("provider".to_owned(), provider.into()),
                ("outcome".to_owned(), outcome.into()),
            ]),
            1.0,
        )
    }

    /// Emit the active process gauge.
    pub fn active(
        &self,
        provider: impl Into<String>,
        domain: impl Into<String>,
        active: u64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ProcessMetric::Active,
            BTreeMap::from([
                ("provider".to_owned(), provider.into()),
                ("domain".to_owned(), domain.into()),
            ]),
            active as f64,
        )
    }

    /// Emit the pidfd gauge without process identity labels.
    pub fn pidfd_active(&self, active: u64) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(ProcessMetric::PidfdActive, BTreeMap::new(), active as f64)
    }

    /// Emit a restart class count.
    pub fn restart(
        &self,
        provider: impl Into<String>,
        class: impl Into<String>,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ProcessMetric::RestartTotal,
            BTreeMap::from([
                ("provider".to_owned(), provider.into()),
                ("class".to_owned(), class.into()),
            ]),
            1.0,
        )
    }

    /// Emit a launch-to-ready duration.
    pub fn ready(
        &self,
        provider: impl Into<String>,
        domain: impl Into<String>,
        duration_seconds: f64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ProcessMetric::ReadyDuration,
            BTreeMap::from([
                ("provider".to_owned(), provider.into()),
                ("domain".to_owned(), domain.into()),
            ]),
            duration_seconds,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_telemetry::validate_descriptor;

    #[test]
    fn launch_metrics_use_provider_not_vm_identity() {
        validate_descriptor(&launch_descriptor()).unwrap();
        validate_descriptor(&stop_descriptor()).unwrap();
        assert!(LAUNCH_BUCKETS_SECONDS.contains(&0.020));
        assert!(!METRIC_INVENTORY.iter().any(|name| name.contains("vm_")));
    }

    #[test]
    fn every_process_metric_has_a_closed_descriptor_and_emit_port() {
        let emitter = BoundedEmitter::new("/nonexistent", 4096).unwrap();
        let metrics = ProcessMetrics::new(emitter);
        for metric in [
            ProcessMetric::LaunchTotal,
            ProcessMetric::LaunchDuration,
            ProcessMetric::Active,
            ProcessMetric::RestartTotal,
            ProcessMetric::AdoptionTotal,
            ProcessMetric::PidfdActive,
            ProcessMetric::StopDuration,
            ProcessMetric::ReadyDuration,
        ] {
            validate_descriptor(&metric.descriptor()).unwrap();
        }
        metrics.launch("systemd", "system", "ok", 0.020).unwrap();
        metrics.stop("systemd", "graceful", "ok", 0.01).unwrap();
        metrics.adoption("systemd", "ok").unwrap();
        metrics.active("systemd", "system", 1).unwrap();
        metrics.pidfd_active(1).unwrap();
        metrics.restart("systemd", "exited").unwrap();
        metrics.ready("systemd", "system", 0.01).unwrap();
    }
}
