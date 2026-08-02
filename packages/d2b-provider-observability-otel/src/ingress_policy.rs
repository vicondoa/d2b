//! Shared structural metric admission for every telemetry ingress.

use std::collections::BTreeMap;

use crate::metric_policy::{
    IdentityCanaries, MetricDescriptor, MetricPolicyError, validate_data_point,
    validate_resource_attributes,
};

/// Maximum frame bytes accepted before policy evaluation.
pub const MAX_INGRESS_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// Maximum frames quarantined for one stream.
pub const MAX_QUARANTINED_CONNECTIONS: usize = 64;
/// Backward-compatible name for the bounded quarantine ceiling.
pub const MAX_QUARANTINED_FRAMES: usize = MAX_QUARANTINED_CONNECTIONS;
/// Number of policy violations before a stream is quarantined.
pub const QUARANTINE_VIOLATION_THRESHOLD: u8 = 3;
/// Maximum time a stream quarantine is intended to remain active.
pub const QUARANTINE_DURATION_SECONDS: u64 = 30;

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

/// One metric data point in a decoded frame.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricPoint {
    /// Descriptor shared by the frame.
    pub descriptor: MetricDescriptor,
    /// Data-point labels.
    pub labels: BTreeMap<String, String>,
}

/// A bounded frame. All points are admitted or rejected together.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricFrame {
    /// Approximate encoded frame size.
    pub encoded_bytes: usize,
    /// Data points.
    pub points: Vec<MetricPoint>,
    /// Trusted resource attributes stamped by the collector.
    pub resource_attributes: BTreeMap<String, String>,
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
}

/// A policy gate with bounded stream quarantine state.
#[derive(Debug, Default)]
pub struct IngressPolicyGate {
    connections: BTreeMap<(Ingress, u64), ConnectionState>,
    quarantined_connections: usize,
}

#[derive(Debug, Default)]
struct ConnectionState {
    violations: u8,
    quarantined: bool,
}

impl IngressPolicyGate {
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
        if self
            .connections
            .get(&(ingress, connection_id))
            .is_some_and(|state| state.quarantined)
        {
            return (IngressOutcome::Quarantined, IngressErrorClass::Malformed);
        }
        if frame.encoded_bytes > MAX_INGRESS_FRAME_BYTES {
            return self.reject(ingress, connection_id, IngressErrorClass::Oversize);
        }
        if !valid_resource_attributes(&frame.resource_attributes) {
            return self.reject(ingress, connection_id, IngressErrorClass::Malformed);
        }
        if frame.points.is_empty() {
            return self.reject(ingress, connection_id, IngressErrorClass::Malformed);
        }
        for point in &frame.points {
            if let Err(error) = validate_data_point(&point.descriptor, &point.labels, canaries) {
                return self.reject(ingress, connection_id, map_policy_error(error));
            }
        }
        if !capacity_available {
            return (IngressOutcome::Rejected, IngressErrorClass::None);
        }
        (IngressOutcome::Accepted, IngressErrorClass::None)
    }

    /// Whether a stream is quarantined.
    pub fn is_quarantined(&self, ingress: Ingress) -> bool {
        self.connections
            .iter()
            .any(|((kind, _), state)| *kind == ingress && state.quarantined)
    }

    /// Whether one opaque connection is quarantined.
    pub fn is_connection_quarantined(&self, ingress: Ingress, connection_id: u64) -> bool {
        self.connections
            .get(&(ingress, connection_id))
            .is_some_and(|state| state.quarantined)
    }

    /// Number of bounded quarantine entries retained.
    pub const fn quarantined_frames(&self) -> usize {
        self.quarantined_connections
    }

    /// Credits available to a quarantined imported stream.
    pub const fn available_import_credits(&self) -> usize {
        // The legacy API has no connection id. A quarantined import means no
        // anonymous import credits can be granted.
        if self.quarantined_connections == 0 {
            1
        } else {
            0
        }
    }

    /// Credits available to one imported stream connection.
    pub fn available_import_credits_for(&self, connection_id: u64) -> usize {
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

    fn reject(
        &mut self,
        ingress: Ingress,
        connection_id: u64,
        error: IngressErrorClass,
    ) -> (IngressOutcome, IngressErrorClass) {
        if matches!(ingress, Ingress::EmitterUnix) {
            return (IngressOutcome::Rejected, error);
        }
        let state = self
            .connections
            .entry((ingress, connection_id))
            .or_default();
        state.violations = state.violations.saturating_add(1);
        if state.violations >= QUARANTINE_VIOLATION_THRESHOLD
            && self.quarantined_connections < MAX_QUARANTINED_CONNECTIONS
        {
            state.quarantined = true;
            self.quarantined_connections += 1;
            return (IngressOutcome::Quarantined, error);
        }
        (IngressOutcome::Rejected, error)
    }
}

fn valid_resource_attributes(attributes: &BTreeMap<String, String>) -> bool {
    validate_resource_attributes(attributes).is_ok()
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

    fn frame(key: &str, value: &str) -> MetricFrame {
        MetricFrame::new(
            64,
            [MetricPoint {
                descriptor: MetricDescriptor::new(
                    "d2b_test_total",
                    [label("outcome", &["ok", "error"])],
                ),
                labels: BTreeMap::from([(key.to_owned(), value.to_owned())]),
            }],
            BTreeMap::from([("d2b.zone".to_owned(), "work".to_owned())]),
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
}
