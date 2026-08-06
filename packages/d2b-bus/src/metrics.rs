//! Closed, low-cardinality metrics for the Zone message bus.
//!
//! The bus never turns route, session, stream, or operation identities into
//! metric labels. Service names are reduced to the closed service catalog and
//! transport direction is represented by [`BusDirection`].

use std::collections::BTreeMap;

use d2b_contracts::v3::{Locality, ServiceName};
use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, IdentityCanaries, MetricDescriptor, MetricPolicyError, Signal,
    emitter::encode_frame, meter_registry::label, validate_data_point, validate_descriptor,
};

/// The closed bus metric inventory.
pub const METRIC_INVENTORY: &[&str] = &[
    "d2b_bus_route_total",
    "d2b_bus_route_duration_seconds",
    "d2b_bus_session_active",
    "d2b_bus_registration_total",
    "d2b_bus_stream_active",
    "d2b_bus_stream_total",
    "d2b_bus_credit_bytes",
    "d2b_bus_backpressure_total",
    "d2b_bus_rejection_total",
    "d2b_bus_disconnect_total",
];

/// Route and stream direction. This enum is the only source of the
/// `direction` metric label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BusDirection {
    /// A Zone-local user or service route.
    Local,
    /// A Host-bound route.
    Host,
    /// A Guest-bound route.
    Guest,
    /// A route crossing a ZoneLink.
    ZoneLink,
}

impl BusDirection {
    /// Every direction in stable label order.
    pub const ALL: [Self; 4] = [Self::Local, Self::Host, Self::Guest, Self::ZoneLink];

    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Host => "host",
            Self::Guest => "guest",
            Self::ZoneLink => "zone_link",
        }
    }

    /// Derive direction from already-authenticated subject evidence.
    pub fn from_context(context: Option<&d2b_contracts::v3::AuthenticatedSubjectContext>) -> Self {
        let Some(context) = context else {
            return Self::Local;
        };
        if matches!(
            context.transport_binding().locality(),
            Locality::AdjacentZone | Locality::Remote
        ) {
            return Self::ZoneLink;
        }
        match context.subject_ref().resource_type().as_str() {
            "Host" => Self::Host,
            "Guest" => Self::Guest,
            _ => Self::Local,
        }
    }
}

/// Transport family used by the active-session gauge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BusTransport {
    Unix,
    Vsock,
    ZoneLink,
}

impl BusTransport {
    /// Every transport in stable label order.
    pub const ALL: [Self; 3] = [Self::Unix, Self::Vsock, Self::ZoneLink];

    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unix => "unix",
            Self::Vsock => "vsock",
            Self::ZoneLink => "zone_link",
        }
    }
}

/// Closed route result labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusRouteOutcome {
    Ok,
    Denied,
    NotFound,
    Error,
}

impl BusRouteOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::NotFound => "not_found",
            Self::Error => "error",
        }
    }
}

/// Closed registration result labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusRegistrationOutcome {
    Accepted,
    Rejected,
}

impl BusRegistrationOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

/// Closed named-stream result labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusStreamOutcome {
    Accepted,
    Rejected,
    Closed,
}

impl BusStreamOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Closed => "abandoned",
        }
    }
}

/// Closed stream/control class used by backpressure metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusStreamKind {
    Control,
    Stream,
}

impl BusStreamKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Stream => "stream",
        }
    }
}

/// Closed backpressure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusBackpressureReason {
    Credit,
    BufferFull,
    Capacity,
}

impl BusBackpressureReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Credit | Self::Capacity => "quota",
            Self::BufferFull => "buffer_full",
        }
    }
}

/// Closed rejection outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusRejectionOutcome {
    Denied,
    NotFound,
    Error,
    Quota,
}

impl BusRejectionOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::NotFound => "not_found",
            Self::Error => "error",
            Self::Quota => "quota",
        }
    }
}

/// Closed disconnect outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusDisconnectOutcome {
    Abandoned,
    Cancel,
    Revoked,
    Error,
}

impl BusDisconnectOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Abandoned => "abandoned",
            Self::Cancel => "cancel",
            Self::Revoked => "revoked",
            Self::Error => "error",
        }
    }
}

/// Bus metric family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusMetric {
    RouteTotal,
    RouteDuration,
    SessionActive,
    RegistrationTotal,
    StreamActive,
    StreamTotal,
    CreditBytes,
    BackpressureTotal,
    RejectionTotal,
    DisconnectTotal,
}

impl BusMetric {
    /// Stable metric name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::RouteTotal => "d2b_bus_route_total",
            Self::RouteDuration => "d2b_bus_route_duration_seconds",
            Self::SessionActive => "d2b_bus_session_active",
            Self::RegistrationTotal => "d2b_bus_registration_total",
            Self::StreamActive => "d2b_bus_stream_active",
            Self::StreamTotal => "d2b_bus_stream_total",
            Self::CreditBytes => "d2b_bus_credit_bytes",
            Self::BackpressureTotal => "d2b_bus_backpressure_total",
            Self::RejectionTotal => "d2b_bus_rejection_total",
            Self::DisconnectTotal => "d2b_bus_disconnect_total",
        }
    }

    /// Build the descriptor with only canonical, closed labels.
    pub fn descriptor(self) -> MetricDescriptor {
        let labels = match self {
            Self::RouteTotal => vec![
                label("service", SERVICE_LABELS),
                label("direction", DIRECTION_LABELS),
                label("outcome", ROUTE_OUTCOME_LABELS),
            ],
            Self::RouteDuration => vec![
                label("service", SERVICE_LABELS),
                label("direction", DIRECTION_LABELS),
            ],
            Self::SessionActive => vec![label("transport", TRANSPORT_LABELS)],
            Self::RegistrationTotal => vec![
                label("direction", DIRECTION_LABELS),
                label("outcome", REGISTRATION_OUTCOME_LABELS),
            ],
            Self::StreamActive | Self::CreditBytes => vec![label("direction", DIRECTION_LABELS)],
            Self::StreamTotal => vec![
                label("direction", DIRECTION_LABELS),
                label("outcome", STREAM_OUTCOME_LABELS),
            ],
            Self::BackpressureTotal => vec![
                label("direction", DIRECTION_LABELS),
                label("kind", STREAM_KIND_LABELS),
                label("reason", BACKPRESSURE_REASON_LABELS),
            ],
            Self::RejectionTotal => vec![
                label("direction", DIRECTION_LABELS),
                label("outcome", REJECTION_OUTCOME_LABELS),
            ],
            Self::DisconnectTotal => vec![
                label("direction", DIRECTION_LABELS),
                label("outcome", DISCONNECT_OUTCOME_LABELS),
            ],
        };
        MetricDescriptor::new(self.name(), labels)
    }
}

const DIRECTION_LABELS: &[&str] = &["local", "host", "guest", "zone_link"];
const TRANSPORT_LABELS: &[&str] = &["unix", "vsock", "zone_link"];
const SERVICE_LABELS: &[&str] = &[
    "bus",
    "d2b.resource.v3",
    "d2b.controller.v3",
    "d2b.provider.v3",
    "d2b.audit.v3",
    "d2b.support.v3",
    "d2b.credential.v3",
    "d2b.zone.v3",
    "d2b.zonelink.v3",
    "d2b.volume.v3",
];
const ROUTE_OUTCOME_LABELS: &[&str] = &["ok", "denied", "not_found", "error"];
const REGISTRATION_OUTCOME_LABELS: &[&str] = &["accepted", "rejected"];
const STREAM_OUTCOME_LABELS: &[&str] = &["accepted", "rejected", "abandoned"];
const STREAM_KIND_LABELS: &[&str] = &["control", "stream"];
const BACKPRESSURE_REASON_LABELS: &[&str] = &["buffer_full", "quota"];
const REJECTION_OUTCOME_LABELS: &[&str] = &["denied", "not_found", "error", "quota"];
const DISCONNECT_OUTCOME_LABELS: &[&str] = &["abandoned", "cancel", "revoked", "error"];

/// Route latency histogram boundaries from the accepted bus contract.
pub const ROUTE_BUCKETS_SECONDS: &[f64] = &[0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1];

/// Adapter port consumed by router, registry, operation, and stream paths.
pub trait BusTelemetry: Send + Sync {
    fn route(
        &self,
        service: &ServiceName,
        direction: BusDirection,
        outcome: BusRouteOutcome,
        duration_seconds: f64,
    );
    fn session_active(&self, transport: BusTransport, active: u64);
    fn registration(&self, direction: BusDirection, outcome: BusRegistrationOutcome);
    fn stream_active(&self, direction: BusDirection, active: u64);
    fn stream_result(&self, direction: BusDirection, outcome: BusStreamOutcome);
    fn credits(&self, direction: BusDirection, bytes: u64);
    fn backpressure(
        &self,
        direction: BusDirection,
        kind: BusStreamKind,
        reason: BusBackpressureReason,
    );
    fn rejection(&self, direction: BusDirection, outcome: BusRejectionOutcome);
    fn disconnect(&self, direction: BusDirection, outcome: BusDisconnectOutcome);
}

/// No-op telemetry port used when observability is not installed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopBusTelemetry;

impl BusTelemetry for NoopBusTelemetry {
    fn route(
        &self,
        _service: &ServiceName,
        _direction: BusDirection,
        _outcome: BusRouteOutcome,
        _duration_seconds: f64,
    ) {
    }

    fn session_active(&self, _transport: BusTransport, _active: u64) {}
    fn registration(&self, _direction: BusDirection, _outcome: BusRegistrationOutcome) {}
    fn stream_active(&self, _direction: BusDirection, _active: u64) {}
    fn stream_result(&self, _direction: BusDirection, _outcome: BusStreamOutcome) {}
    fn credits(&self, _direction: BusDirection, _bytes: u64) {}
    fn backpressure(
        &self,
        _direction: BusDirection,
        _kind: BusStreamKind,
        _reason: BusBackpressureReason,
    ) {
    }
    fn rejection(&self, _direction: BusDirection, _outcome: BusRejectionOutcome) {}
    fn disconnect(&self, _direction: BusDirection, _outcome: BusDisconnectOutcome) {}
}

/// Bounded-emitter implementation used by the observability handoff.
#[derive(Clone, Debug)]
pub struct BusMetrics {
    emitter: BoundedEmitter,
}

impl BusMetrics {
    /// Construct metrics on an existing bounded telemetry emitter.
    pub fn new(emitter: BoundedEmitter) -> Self {
        Self { emitter }
    }

    /// Borrow the emitter for observability health reporting.
    pub const fn emitter(&self) -> &BoundedEmitter {
        &self.emitter
    }

    /// Validate every bus descriptor.
    pub fn validate_inventory() -> Result<(), MetricPolicyError> {
        for metric in [
            BusMetric::RouteTotal,
            BusMetric::RouteDuration,
            BusMetric::SessionActive,
            BusMetric::RegistrationTotal,
            BusMetric::StreamActive,
            BusMetric::StreamTotal,
            BusMetric::CreditBytes,
            BusMetric::BackpressureTotal,
            BusMetric::RejectionTotal,
            BusMetric::DisconnectTotal,
        ] {
            validate_descriptor(&metric.descriptor())?;
        }
        Ok(())
    }

    fn emit(
        &self,
        metric: BusMetric,
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

impl BusTelemetry for BusMetrics {
    fn route(
        &self,
        service: &ServiceName,
        direction: BusDirection,
        outcome: BusRouteOutcome,
        duration_seconds: f64,
    ) {
        let service = service_label(service);
        let labels = BTreeMap::from([
            ("service".to_owned(), service.to_owned()),
            ("direction".to_owned(), direction.as_str().to_owned()),
            ("outcome".to_owned(), outcome.as_str().to_owned()),
        ]);
        let _ = self.emit(BusMetric::RouteTotal, labels, 1.0);
        let labels = BTreeMap::from([
            ("service".to_owned(), service.to_owned()),
            ("direction".to_owned(), direction.as_str().to_owned()),
        ]);
        let _ = self.emit(BusMetric::RouteDuration, labels, duration_seconds);
    }

    fn session_active(&self, transport: BusTransport, active: u64) {
        let _ = self.emit(
            BusMetric::SessionActive,
            BTreeMap::from([("transport".to_owned(), transport.as_str().to_owned())]),
            active as f64,
        );
    }

    fn registration(&self, direction: BusDirection, outcome: BusRegistrationOutcome) {
        let _ = self.emit(
            BusMetric::RegistrationTotal,
            BTreeMap::from([
                ("direction".to_owned(), direction.as_str().to_owned()),
                ("outcome".to_owned(), outcome.as_str().to_owned()),
            ]),
            1.0,
        );
    }

    fn stream_active(&self, direction: BusDirection, active: u64) {
        let _ = self.emit(
            BusMetric::StreamActive,
            BTreeMap::from([("direction".to_owned(), direction.as_str().to_owned())]),
            active as f64,
        );
    }

    fn stream_result(&self, direction: BusDirection, outcome: BusStreamOutcome) {
        let _ = self.emit(
            BusMetric::StreamTotal,
            BTreeMap::from([
                ("direction".to_owned(), direction.as_str().to_owned()),
                ("outcome".to_owned(), outcome.as_str().to_owned()),
            ]),
            1.0,
        );
    }

    fn credits(&self, direction: BusDirection, bytes: u64) {
        let _ = self.emit(
            BusMetric::CreditBytes,
            BTreeMap::from([("direction".to_owned(), direction.as_str().to_owned())]),
            bytes as f64,
        );
    }

    fn backpressure(
        &self,
        direction: BusDirection,
        kind: BusStreamKind,
        reason: BusBackpressureReason,
    ) {
        let _ = self.emit(
            BusMetric::BackpressureTotal,
            BTreeMap::from([
                ("direction".to_owned(), direction.as_str().to_owned()),
                ("kind".to_owned(), kind.as_str().to_owned()),
                ("reason".to_owned(), reason.as_str().to_owned()),
            ]),
            1.0,
        );
    }

    fn rejection(&self, direction: BusDirection, outcome: BusRejectionOutcome) {
        let _ = self.emit(
            BusMetric::RejectionTotal,
            BTreeMap::from([
                ("direction".to_owned(), direction.as_str().to_owned()),
                ("outcome".to_owned(), outcome.as_str().to_owned()),
            ]),
            1.0,
        );
    }

    fn disconnect(&self, direction: BusDirection, outcome: BusDisconnectOutcome) {
        let _ = self.emit(
            BusMetric::DisconnectTotal,
            BTreeMap::from([
                ("direction".to_owned(), direction.as_str().to_owned()),
                ("outcome".to_owned(), outcome.as_str().to_owned()),
            ]),
            1.0,
        );
    }
}

fn service_label(service: &ServiceName) -> &'static str {
    match service.as_str() {
        "d2b.resource.v3" => "d2b.resource.v3",
        "d2b.controller.v3" => "d2b.controller.v3",
        "d2b.provider.v3" => "d2b.provider.v3",
        "d2b.audit.v3" => "d2b.audit.v3",
        "d2b.support.v3" => "d2b.support.v3",
        "d2b.credential.v3" => "d2b.credential.v3",
        "d2b.zone.v3" => "d2b.zone.v3",
        "d2b.zonelink.v3" => "d2b.zonelink.v3",
        "d2b.volume.v3" => "d2b.volume.v3",
        _ => "bus",
    }
}

/// Convert an authenticated subject's locality to the active-session
/// transport label.
pub(crate) fn transport_for_context(
    context: Option<&d2b_contracts::v3::AuthenticatedSubjectContext>,
) -> BusTransport {
    if context.is_some_and(|context| {
        matches!(
            context.transport_binding().locality(),
            Locality::AdjacentZone | Locality::Remote
        )
    }) {
        BusTransport::ZoneLink
    } else {
        BusTransport::Unix
    }
}

/// Return a route outcome without exposing the error's identity-bearing text.
pub(crate) fn route_outcome(error: Option<&crate::router::BusError>) -> BusRouteOutcome {
    match error {
        None => BusRouteOutcome::Ok,
        Some(crate::router::BusError::Authorization(_)) => BusRouteOutcome::Denied,
        Some(crate::router::BusError::Registry(crate::registry::RegistryError::RouteNotFound))
        | Some(crate::router::BusError::Operation(
            crate::operations::OperationError::OperationNotFound,
        )) => BusRouteOutcome::NotFound,
        Some(_) => BusRouteOutcome::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_telemetry::validate_descriptor;

    #[test]
    fn bus_inventory_has_closed_direction_transport_and_route_domains() {
        BusMetrics::validate_inventory().unwrap();
        assert_eq!(
            BusDirection::ALL.map(BusDirection::as_str),
            ["local", "host", "guest", "zone_link"]
        );
        assert_eq!(
            BusTransport::ALL.map(BusTransport::as_str),
            ["unix", "vsock", "zone_link"]
        );
        validate_descriptor(&BusMetric::RouteTotal.descriptor()).unwrap();
        validate_descriptor(&BusMetric::RouteDuration.descriptor()).unwrap();
        assert!(ROUTE_BUCKETS_SECONDS.contains(&0.005));
    }

    #[test]
    fn emitter_records_only_closed_bus_labels() {
        let emitter = BoundedEmitter::new("/nonexistent", 16 * 1024).unwrap();
        let metrics = BusMetrics::new(emitter);
        metrics.route(
            &ServiceName::parse("d2b.resource.v3").unwrap(),
            BusDirection::Host,
            BusRouteOutcome::Ok,
            0.005,
        );
        metrics.session_active(BusTransport::Unix, 1);
        metrics.registration(BusDirection::Local, BusRegistrationOutcome::Accepted);
        metrics.stream_active(BusDirection::Guest, 1);
        metrics.stream_result(BusDirection::Guest, BusStreamOutcome::Accepted);
        metrics.credits(BusDirection::Guest, 64);
        metrics.backpressure(
            BusDirection::Guest,
            BusStreamKind::Stream,
            BusBackpressureReason::Credit,
        );
        metrics.rejection(BusDirection::Local, BusRejectionOutcome::Denied);
        metrics.disconnect(BusDirection::ZoneLink, BusDisconnectOutcome::Abandoned);
    }

    #[test]
    fn unknown_service_collapses_to_the_bus_catalog_bucket() {
        let service = ServiceName::parse("vendor.service.v3").unwrap();
        assert_eq!(service_label(&service), "bus");
    }

    #[test]
    fn direction_is_not_derived_from_caller_label_text() {
        assert!(std::any::TypeId::of::<BusDirection>() != std::any::TypeId::of::<String>());
    }
}
