//! Shared structural metric admission for every telemetry ingress.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use crate::metric_policy::{
    IdentityCanaries, MetricDescriptor, MetricPolicyError, validate_data_point,
    validate_resource_attributes,
};
use d2b_contracts::v3::{TelemetryFrame, TelemetrySignal};

/// Maximum frame bytes accepted before policy evaluation.
pub const MAX_INGRESS_FRAME_BYTES: usize = d2b_contracts::v3::MAX_TELEMETRY_FRAME_BYTES;
/// Maximum metric points in one admitted frame.
pub const MAX_POINTS_PER_FRAME: usize = 1024;
/// Maximum frames quarantined for one stream.
pub const MAX_QUARANTINED_CONNECTIONS: usize = 64;
/// Maximum live connection states retained for policy accounting.
pub const MAX_TRACKED_CONNECTIONS: usize = 4096;
/// Backward-compatible name for the bounded quarantine ceiling.
pub const MAX_QUARANTINED_FRAMES: usize = MAX_QUARANTINED_CONNECTIONS;
/// Number of policy violations before a stream is quarantined.
pub const QUARANTINE_VIOLATION_THRESHOLD: u8 = 3;
/// Maximum time a stream quarantine is intended to remain active.
pub const QUARANTINE_DURATION_SECONDS: u64 = 30;
/// Idle connection state is reclaimed on the same bounded horizon.
pub const CONNECTION_IDLE_SECONDS: u64 = 30;
/// Provider-wide cap on retained metric series. Existing series are never
/// evicted to admit a new one.
pub const MAX_PROVIDER_SERIES: usize = 4096;

/// Telemetry ingress adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ingress {
    /// Compact Unix datagram emitter.
    EmitterUnix,
    /// OTLP over a private Unix socket.
    OtlpUnix,
    /// OTLP over the ZoneLink vsock path.
    OtlpVsock,
    /// D096 imported named stream.
    ImportStream,
}

impl Ingress {
    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmitterUnix => "emitter_unix",
            Self::OtlpUnix => "otlp_unix",
            Self::OtlpVsock => "otlp_vsock",
            Self::ImportStream => "import_stream",
        }
    }
}

/// Bounded policy outcome class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressErrorClass {
    /// No error.
    None,
    /// A label key is outside the allowlist.
    KeyNotAllowlisted,
    /// A key is unconditionally forbidden.
    KeyForbidden,
    /// A key has an identity suffix.
    KeySuffixForbidden,
    /// A value carries a resource identity.
    ValueIdentity,
    /// The frame could not be decoded.
    Malformed,
    /// The frame exceeded the byte bound.
    Oversize,
}

impl IngressErrorClass {
    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::KeyNotAllowlisted => "key_not_allowlisted",
            Self::KeyForbidden => "key_forbidden",
            Self::KeySuffixForbidden => "key_suffix_forbidden",
            Self::ValueIdentity => "value_identity",
            Self::Malformed => "malformed",
            Self::Oversize => "oversize",
        }
    }
}

/// Admission result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressOutcome {
    /// The complete frame passed validation and capacity.
    Accepted,
    /// The complete frame was rejected.
    Rejected,
    /// The stream was quarantined after a policy failure.
    Quarantined,
}

/// Clock used for injected quarantine expiry tests.
pub trait IngressClock: Send + Sync {
    /// Return monotonic milliseconds for policy state.
    fn now_ms(&self) -> u64;
}

#[derive(Debug, Default)]
struct SystemIngressClock;

impl IngressClock for SystemIngressClock {
    fn now_ms(&self) -> u64 {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(Instant::now)
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    }
}

/// One metric data point in a decoded frame.
#[derive(Clone, PartialEq)]
pub struct MetricPoint {
    /// Descriptor shared by the frame.
    pub descriptor: MetricDescriptor,
    /// Data-point labels.
    pub labels: BTreeMap<String, String>,
    /// Finite metric value retained for export.
    pub value: f64,
}

impl core::fmt::Debug for MetricPoint {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MetricPoint")
            .field("descriptor", &self.descriptor.name())
            .field("label_count", &self.labels.len())
            .finish()
    }
}

/// A bounded frame. All points are admitted or rejected together.
#[derive(Clone, PartialEq)]
pub struct MetricFrame {
    /// Approximate encoded frame size.
    pub encoded_bytes: usize,
    /// Data points.
    pub points: Vec<MetricPoint>,
    /// Trusted resource attributes stamped by the collector.
    pub resource_attributes: BTreeMap<String, String>,
}

impl core::fmt::Debug for MetricFrame {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MetricFrame")
            .field("encoded_bytes", &self.encoded_bytes)
            .field("point_count", &self.points.len())
            .field("resource_attribute_count", &self.resource_attributes.len())
            .finish()
    }
}

impl MetricFrame {
    /// Construct one frame.
    pub fn new(
        encoded_bytes: usize,
        points: impl IntoIterator<Item = MetricPoint>,
        resource_attributes: BTreeMap<String, String>,
    ) -> Self {
        Self {
            encoded_bytes,
            points: points.into_iter().collect(),
            resource_attributes,
        }
    }

    /// Measure the canonical encoded frame instead of trusting caller bytes.
    pub fn measured_encoded_bytes(&self) -> usize {
        serde_json::to_vec(&serde_json::json!({
            "points": self.points.iter().map(|point| {
                serde_json::json!({
                    "descriptor": point.descriptor.name(),
                    "labels": point.labels,
                    "value": point.value,
                })
            }).collect::<Vec<_>>(),
            "resource_attributes": self.resource_attributes,
        }))
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
    }
}

/// A policy gate with bounded stream quarantine state.
pub struct IngressPolicyGate {
    connections: BTreeMap<(Ingress, u64), ConnectionState>,
    quarantined_connections: usize,
    series: std::collections::BTreeSet<(String, Vec<(String, String)>)>,
    clock: Arc<dyn IngressClock>,
}

#[derive(Debug, Default)]
struct ConnectionState {
    violations: u8,
    quarantined: bool,
    quarantined_until_ms: Option<u64>,
    last_seen_ms: u64,
}

impl core::fmt::Debug for IngressPolicyGate {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("IngressPolicyGate")
            .field("tracked_connections", &self.connections.len())
            .field("quarantined_connections", &self.quarantined_connections)
            .field("series", &self.series.len())
            .finish()
    }
}

impl Default for IngressPolicyGate {
    fn default() -> Self {
        Self::with_clock(Arc::new(SystemIngressClock))
    }
}

impl IngressPolicyGate {
    /// Admit one raw shared frame before any queue mutation or eviction.
    pub fn admit_raw(
        &mut self,
        ingress: Ingress,
        connection_id: u64,
        bytes: &[u8],
    ) -> (IngressOutcome, IngressErrorClass) {
        self.prune_expired();
        if bytes.len() > MAX_INGRESS_FRAME_BYTES {
            return self.reject(ingress, connection_id, IngressErrorClass::Oversize);
        }
        let frame = match d2b_contracts::v3::validate_raw_frame(bytes) {
            Ok(frame) => frame,
            Err(_) => return self.reject(ingress, connection_id, IngressErrorClass::Malformed),
        };
        if frame.signal == TelemetrySignal::Metric {
            let Some(metric) = metric_frame_from_raw(&frame, bytes.len()) else {
                return self.reject(ingress, connection_id, IngressErrorClass::Malformed);
            };
            return self.admit_for_connection(
                ingress,
                connection_id,
                &metric,
                &IdentityCanaries::default(),
                true,
            );
        }
        (IngressOutcome::Accepted, IngressErrorClass::None)
    }

    /// Construct a policy gate with an injected clock.
    pub fn with_clock(clock: Arc<dyn IngressClock>) -> Self {
        Self {
            connections: BTreeMap::new(),
            quarantined_connections: 0,
            series: std::collections::BTreeSet::new(),
            clock,
        }
    }

    /// Admit a complete frame before queue/capacity accounting.
    pub fn admit(
        &mut self,
        ingress: Ingress,
        frame: &MetricFrame,
        canaries: &IdentityCanaries,
        capacity_available: bool,
    ) -> (IngressOutcome, IngressErrorClass) {
        self.admit_for_connection(ingress, 0, frame, canaries, capacity_available)
    }

    /// Admit a frame for one opaque stream connection.
    ///
    /// Datagram ingress uses the legacy connection id `0` and is never
    /// quarantined. Stream callers should provide their own bounded opaque
    /// connection id so one noisy producer cannot quarantine its peers.
    pub fn admit_for_connection(
        &mut self,
        ingress: Ingress,
        connection_id: u64,
        frame: &MetricFrame,
        canaries: &IdentityCanaries,
        capacity_available: bool,
    ) -> (IngressOutcome, IngressErrorClass) {
        self.prune_expired();
        if self
            .connections
            .get(&(ingress, connection_id))
            .is_some_and(|state| {
                state.quarantined
                    && state
                        .quarantined_until_ms
                        .is_some_and(|until| self.clock.now_ms() < until)
            })
        {
            return (IngressOutcome::Quarantined, IngressErrorClass::Malformed);
        }
        if frame.measured_encoded_bytes() > MAX_INGRESS_FRAME_BYTES {
            return self.reject(ingress, connection_id, IngressErrorClass::Oversize);
        }
        if !valid_resource_attributes(&frame.resource_attributes) {
            return self.reject(ingress, connection_id, IngressErrorClass::Malformed);
        }
        if frame.points.is_empty() || frame.points.len() > MAX_POINTS_PER_FRAME {
            return self.reject(ingress, connection_id, IngressErrorClass::Malformed);
        }
        for point in &frame.points {
            if !point.value.is_finite() {
                return self.reject(ingress, connection_id, IngressErrorClass::Malformed);
            }
            if let Err(error) = validate_data_point(&point.descriptor, &point.labels, canaries) {
                return self.reject(ingress, connection_id, map_policy_error(error));
            }
        }
        if !capacity_available {
            return (IngressOutcome::Rejected, IngressErrorClass::None);
        }
        let incoming = frame
            .points
            .iter()
            .map(|point| {
                (
                    point.descriptor.name().to_owned(),
                    point
                        .labels
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        let new_series = incoming
            .iter()
            .filter(|series| !self.series.contains(*series))
            .count();
        if self.series.len().saturating_add(new_series) > MAX_PROVIDER_SERIES {
            return (IngressOutcome::Rejected, IngressErrorClass::None);
        }
        self.series.extend(incoming);
        (IngressOutcome::Accepted, IngressErrorClass::None)
    }

    /// Number of retained provider metric series.
    pub fn series_count(&self) -> usize {
        self.series.len()
    }

    /// Whether a stream is quarantined.
    pub fn is_quarantined(&mut self, ingress: Ingress) -> bool {
        self.prune_expired();
        self.connections
            .iter()
            .any(|((kind, _), state)| *kind == ingress && state.quarantined)
    }

    /// Whether one opaque connection is quarantined.
    pub fn is_connection_quarantined(&mut self, ingress: Ingress, connection_id: u64) -> bool {
        self.prune_expired();
        self.connections
            .get(&(ingress, connection_id))
            .is_some_and(|state| state.quarantined)
    }

    /// Number of bounded quarantine entries retained.
    pub fn quarantined_frames(&mut self) -> usize {
        self.prune_expired();
        self.quarantined_connections
    }

    /// Credits available to a quarantined imported stream.
    pub fn available_import_credits(&mut self) -> usize {
        self.prune_expired();
        // The legacy API has no connection id. A quarantined import means no
        // anonymous import credits can be granted.
        if self.quarantined_connections == 0 {
            1
        } else {
            0
        }
    }

    /// Credits available to one imported stream connection.
    pub fn available_import_credits_for(&mut self, connection_id: u64) -> usize {
        if self.is_connection_quarantined(Ingress::ImportStream, connection_id) {
            0
        } else {
            1
        }
    }

    /// Forget a disconnected connection and release its quarantine slot.
    pub fn reset_connection(&mut self, ingress: Ingress, connection_id: u64) {
        if self
            .connections
            .remove(&(ingress, connection_id))
            .is_some_and(|state| state.quarantined)
        {
            self.quarantined_connections = self.quarantined_connections.saturating_sub(1);
        }
    }

    /// Remove expired quarantines and stale connection entries.
    pub fn prune_expired(&mut self) {
        let now = self.clock.now_ms();
        let expired = self
            .connections
            .iter()
            .filter_map(|(key, state)| {
                (state.quarantined_until_ms.is_some_and(|until| now >= until)
                    || (!state.quarantined
                        && now.saturating_sub(state.last_seen_ms)
                            >= CONNECTION_IDLE_SECONDS.saturating_mul(1000)))
                .then_some(*key)
            })
            .collect::<Vec<_>>();
        for key in expired {
            self.reset_connection(key.0, key.1);
        }
    }

    fn reject(
        &mut self,
        ingress: Ingress,
        connection_id: u64,
        error: IngressErrorClass,
    ) -> (IngressOutcome, IngressErrorClass) {
        let now = self.clock.now_ms();
        if matches!(ingress, Ingress::EmitterUnix) {
            return (IngressOutcome::Rejected, error);
        }
        if !self.connections.contains_key(&(ingress, connection_id))
            && self.connections.len() >= MAX_TRACKED_CONNECTIONS
        {
            return (IngressOutcome::Rejected, IngressErrorClass::Malformed);
        }
        let state = self
            .connections
            .entry((ingress, connection_id))
            .or_default();
        state.last_seen_ms = now;
        state.violations = state.violations.saturating_add(1);
        if state.violations >= QUARANTINE_VIOLATION_THRESHOLD
            && self.quarantined_connections < MAX_QUARANTINED_CONNECTIONS
        {
            state.quarantined = true;
            state.quarantined_until_ms = Some(
                self.clock
                    .now_ms()
                    .saturating_add(QUARANTINE_DURATION_SECONDS.saturating_mul(1000)),
            );
            self.quarantined_connections += 1;
            return (IngressOutcome::Quarantined, error);
        }
        (IngressOutcome::Rejected, error)
    }
}

fn valid_resource_attributes(attributes: &BTreeMap<String, String>) -> bool {
    validate_resource_attributes(attributes).is_ok()
}

fn metric_frame_from_raw(frame: &TelemetryFrame, encoded_bytes: usize) -> Option<MetricFrame> {
    let object = frame.value.as_object()?;
    let name = object.get("name")?.as_str()?;
    let labels = object.get("labels")?.as_object()?;
    let labels = labels
        .iter()
        .map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect::<Option<BTreeMap<_, _>>>()?;
    let descriptor = MetricDescriptor::new(
        name,
        labels
            .iter()
            .map(|(key, value)| crate::label(key, &[value.as_str()]))
            .collect::<Vec<_>>(),
    );
    let value = object.get("value")?.as_f64()?;
    let resource_attributes = match object.get("resource_attributes") {
        Some(value) => serde_json::from_value(value.clone()).ok()?,
        None => BTreeMap::new(),
    };
    Some(MetricFrame::new(
        encoded_bytes,
        [MetricPoint {
            descriptor,
            labels,
            value,
        }],
        resource_attributes,
    ))
}

fn map_policy_error(error: MetricPolicyError) -> IngressErrorClass {
    match error {
        MetricPolicyError::KeyNotAllowlisted => IngressErrorClass::KeyNotAllowlisted,
        MetricPolicyError::KeyForbidden => IngressErrorClass::KeyForbidden,
        MetricPolicyError::KeySuffixForbidden => IngressErrorClass::KeySuffixForbidden,
        MetricPolicyError::ValueIdentity => IngressErrorClass::ValueIdentity,
        MetricPolicyError::LabelSetMismatch | MetricPolicyError::ValueNotAllowlisted => {
            IngressErrorClass::Malformed
        }
        MetricPolicyError::DescriptorMalformed => IngressErrorClass::Malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug)]
    struct ManualClock(AtomicU64);

    impl IngressClock for ManualClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    fn frame(key: &str, value: &str) -> MetricFrame {
        MetricFrame::new(
            64,
            [MetricPoint {
                descriptor: MetricDescriptor::new(
                    "d2b_test_total",
                    [label("outcome", &["ok", "error"])],
                ),
                labels: BTreeMap::from([(key.to_owned(), value.to_owned())]),
                value: 1.0,
            }],
            BTreeMap::from([(
                "d2b.zone".to_owned(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                    .to_owned(),
            )]),
        )
    }

    #[test]
    fn policy_runs_before_capacity_and_rejects_the_whole_frame() {
        let mut gate = IngressPolicyGate::default();
        let valid = frame("outcome", "ok");
        assert_eq!(
            gate.admit(
                Ingress::EmitterUnix,
                &valid,
                &IdentityCanaries::default(),
                false
            ),
            (IngressOutcome::Rejected, IngressErrorClass::None)
        );
        let invalid = frame("vm", "work");
        assert_eq!(
            gate.admit(
                Ingress::EmitterUnix,
                &invalid,
                &IdentityCanaries::default(),
                true
            ),
            (IngressOutcome::Rejected, IngressErrorClass::Malformed)
        );
    }

    #[test]
    fn import_stream_has_no_credits_after_quarantine() {
        let mut gate = IngressPolicyGate::default();
        let invalid = frame("vm", "work");
        let outcome = (0..3)
            .map(|_| {
                gate.admit_for_connection(
                    Ingress::ImportStream,
                    7,
                    &invalid,
                    &IdentityCanaries::default(),
                    true,
                )
            })
            .last()
            .unwrap();
        assert_eq!(outcome.0, IngressOutcome::Quarantined);
        assert_eq!(gate.available_import_credits_for(7), 0);
        assert!(gate.is_connection_quarantined(Ingress::ImportStream, 7));
    }

    #[test]
    fn quarantine_expires_on_injected_clock_and_disconnect_releases_state() {
        let clock = Arc::new(ManualClock(AtomicU64::new(0)));
        let mut gate = IngressPolicyGate::with_clock(clock.clone());
        let invalid = frame("vm", "work");
        for _ in 0..QUARANTINE_VIOLATION_THRESHOLD {
            let _ = gate.admit_for_connection(
                Ingress::ImportStream,
                9,
                &invalid,
                &IdentityCanaries::default(),
                true,
            );
        }
        assert!(gate.is_connection_quarantined(Ingress::ImportStream, 9));
        clock.0.store(30_001, Ordering::Relaxed);
        gate.prune_expired();
        assert!(!gate.is_connection_quarantined(Ingress::ImportStream, 9));
        assert_eq!(gate.quarantined_frames(), 0);
        gate.reset_connection(Ingress::ImportStream, 9);
        assert_eq!(gate.available_import_credits_for(9), 1);
    }

    #[test]
    fn raw_emitter_admission_enforces_the_provider_series_cap() {
        let mut gate = IngressPolicyGate::default();
        for index in 0..MAX_PROVIDER_SERIES {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "signal": "metric",
                "value": {
                    "name": format!("d2b_series_{index}"),
                    "labels": {"outcome": "ok"},
                    "value": 1
                }
            }))
            .expect("metric frame");
            assert_eq!(
                gate.admit_raw(Ingress::EmitterUnix, 0, &bytes).0,
                IngressOutcome::Accepted
            );
        }
        let bytes = serde_json::to_vec(&serde_json::json!({
            "signal": "metric",
            "value": {
                "name": "d2b_series_over_cap",
                "labels": {"outcome": "ok"},
                "value": 1
            }
        }))
        .expect("over-cap frame");
        assert_eq!(
            gate.admit_raw(Ingress::EmitterUnix, 0, &bytes),
            (IngressOutcome::Rejected, IngressErrorClass::None)
        );
        assert_eq!(gate.series_count(), MAX_PROVIDER_SERIES);
    }
}
