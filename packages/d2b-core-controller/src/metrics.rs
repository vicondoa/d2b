//! Core-controller metric inventory and bounded emitter adapter.

use std::collections::BTreeMap;

use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, IdentityCanaries, MetricDescriptor, MetricPolicyError, Signal,
    encode_frame, meter_registry::label, validate_data_point,
};

/// Core-controller metrics.
pub const METRIC_INVENTORY: &[&str] = &[
    "d2b_controller_reconcile_total",
    "d2b_controller_reconcile_duration_seconds",
    "d2b_controller_queue_depth",
    "d2b_controller_hint_to_handler_seconds",
    "d2b_controller_watch_revision_lag",
    "d2b_provider_component_phase",
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
/// Reconcile duration boundaries.
pub const RECONCILE_BUCKETS_SECONDS: &[f64] =
    &[0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 2.0];

/// Core-controller metric family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerMetric {
    /// Reconcile completion count.
    ReconcileTotal,
    /// Reconcile duration observation.
    ReconcileDuration,
    /// Handler queue depth.
    QueueDepth,
    /// Commit-to-handler latency observation.
    HintToHandler,
    /// Watch revision lag gauge.
    WatchRevisionLag,
    /// Provider component phase gauge.
    ProviderComponentPhase,
}

impl ControllerMetric {
    /// Stable metric name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReconcileTotal => "d2b_controller_reconcile_total",
            Self::ReconcileDuration => "d2b_controller_reconcile_duration_seconds",
            Self::QueueDepth => "d2b_controller_queue_depth",
            Self::HintToHandler => "d2b_controller_hint_to_handler_seconds",
            Self::WatchRevisionLag => "d2b_controller_watch_revision_lag",
            Self::ProviderComponentPhase => "d2b_provider_component_phase",
        }
    }

    /// Closed descriptor for one metric family.
    pub fn descriptor(self) -> MetricDescriptor {
        match self {
            Self::ReconcileTotal => reconcile_descriptor(),
            Self::ReconcileDuration => MetricDescriptor::new(
                self.name(),
                [
                    label("handler", HANDLERS),
                    label("outcome", &["ok", "requeue", "conflict", "error"]),
                ],
            ),
            Self::QueueDepth | Self::HintToHandler | Self::WatchRevisionLag => {
                MetricDescriptor::new(self.name(), [label("handler", HANDLERS)])
            }
            Self::ProviderComponentPhase => MetricDescriptor::new(
                self.name(),
                [
                    label("component_type", &["controller", "service", "worker"]),
                    label(
                        "phase",
                        &["pending", "ready", "degraded", "failed", "unknown"],
                    ),
                ],
            ),
        }
    }
}

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
    ControllerMetric::HintToHandler.descriptor()
}

/// Bounded metric adapter for controller and Provider lifecycle callsites.
#[derive(Clone, Debug)]
pub struct ControllerMetrics {
    emitter: BoundedEmitter,
}

impl ControllerMetrics {
    /// Construct an emitter-backed controller metric adapter.
    pub fn new(emitter: BoundedEmitter) -> Self {
        Self { emitter }
    }

    /// Emit one validated controller metric observation.
    pub fn observe(
        &self,
        metric: ControllerMetric,
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

    /// Emit one reconcile count and duration pair.
    pub fn reconcile(
        &self,
        handler: impl Into<String>,
        outcome: impl Into<String>,
        duration_seconds: f64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        let handler = handler.into();
        let outcome = outcome.into();
        self.observe(
            ControllerMetric::ReconcileTotal,
            BTreeMap::from([
                ("handler".to_owned(), handler.clone()),
                ("outcome".to_owned(), outcome.clone()),
            ]),
            1.0,
        )?;
        self.observe(
            ControllerMetric::ReconcileDuration,
            BTreeMap::from([
                ("handler".to_owned(), handler),
                ("outcome".to_owned(), outcome),
            ]),
            duration_seconds,
        )
    }

    /// Emit the commit-to-handler hint latency.
    pub fn hint_to_handler(
        &self,
        handler: impl Into<String>,
        duration_seconds: f64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ControllerMetric::HintToHandler,
            BTreeMap::from([("handler".to_owned(), handler.into())]),
            duration_seconds,
        )
    }

    /// Emit one handler queue depth.
    pub fn queue_depth(
        &self,
        handler: impl Into<String>,
        depth: u64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ControllerMetric::QueueDepth,
            BTreeMap::from([("handler".to_owned(), handler.into())]),
            depth as f64,
        )
    }

    /// Emit one watch revision lag.
    pub fn watch_revision_lag(
        &self,
        handler: impl Into<String>,
        lag: u64,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ControllerMetric::WatchRevisionLag,
            BTreeMap::from([("handler".to_owned(), handler.into())]),
            lag as f64,
        )
    }

    /// Emit one Provider component phase.
    pub fn provider_component_phase(
        &self,
        component_type: impl Into<String>,
        phase: impl Into<String>,
    ) -> Result<EmitOutcome, MetricPolicyError> {
        self.observe(
            ControllerMetric::ProviderComponentPhase,
            BTreeMap::from([
                ("component_type".to_owned(), component_type.into()),
                ("phase".to_owned(), phase.into()),
            ]),
            1.0,
        )
    }
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

    #[test]
    fn every_controller_metric_has_a_closed_descriptor_and_emit_port() {
        let emitter = BoundedEmitter::new("/nonexistent", 4096).unwrap();
        let metrics = ControllerMetrics::new(emitter);
        for metric in [
            ControllerMetric::ReconcileTotal,
            ControllerMetric::ReconcileDuration,
            ControllerMetric::QueueDepth,
            ControllerMetric::HintToHandler,
            ControllerMetric::WatchRevisionLag,
            ControllerMetric::ProviderComponentPhase,
        ] {
            validate_descriptor(&metric.descriptor()).unwrap();
        }
        metrics.reconcile("provider", "ok", 0.005).unwrap();
        metrics.hint_to_handler("provider", 0.005).unwrap();
        metrics.queue_depth("provider", 1).unwrap();
        metrics.watch_revision_lag("provider", 0).unwrap();
        metrics
            .provider_component_phase("service", "ready")
            .unwrap();
    }
}
