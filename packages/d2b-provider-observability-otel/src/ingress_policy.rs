//! Shared structural metric admission for every telemetry ingress.

use std::collections::BTreeMap;

use d2b_telemetry::{
    IdentityCanaries, MetricDescriptor, MetricPolicyError, RedactionGuard, validate_data_point,
};

/// Maximum frame bytes accepted before policy evaluation.
pub const MAX_INGRESS_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// Maximum frames quarantined for one stream.
pub const MAX_QUARANTINED_FRAMES: usize = 32;

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
    quarantined: std::collections::BTreeSet<Ingress>,
    quarantined_frames: usize,
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
        if self.quarantined.contains(&ingress) {
            return (IngressOutcome::Quarantined, IngressErrorClass::Malformed);
        }
        if frame.encoded_bytes > MAX_INGRESS_FRAME_BYTES {
            return self.reject(ingress, IngressErrorClass::Oversize);
        }
        if !valid_resource_attributes(&frame.resource_attributes) {
            return self.reject(ingress, IngressErrorClass::Malformed);
        }
        for point in &frame.points {
            if let Err(error) = validate_data_point(&point.descriptor, &point.labels, canaries) {
                return self.reject(ingress, map_policy_error(error));
            }
        }
        if !capacity_available {
            return (IngressOutcome::Rejected, IngressErrorClass::None);
        }
        (IngressOutcome::Accepted, IngressErrorClass::None)
    }

    /// Whether a stream is quarantined.
    pub fn is_quarantined(&self, ingress: Ingress) -> bool {
        self.quarantined.contains(&ingress)
    }

    /// Number of frames retained while quarantined.
    pub const fn quarantined_frames(&self) -> usize {
        self.quarantined_frames
    }

    /// Credits available to a quarantined imported stream.
    pub const fn available_import_credits(&self) -> usize {
        0
    }

    fn reject(
        &mut self,
        ingress: Ingress,
        error: IngressErrorClass,
    ) -> (IngressOutcome, IngressErrorClass) {
        if matches!(ingress, Ingress::ImportStream)
            && self.quarantined_frames < MAX_QUARANTINED_FRAMES
        {
            self.quarantined.insert(ingress);
            self.quarantined_frames += 1;
            return (IngressOutcome::Quarantined, error);
        }
        (IngressOutcome::Rejected, error)
    }
}

fn valid_resource_attributes(attributes: &BTreeMap<String, String>) -> bool {
    RedactionGuard::new(
        attributes
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    )
    .is_ok()
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
    use d2b_telemetry::meter_registry::label;

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
        let outcome = gate.admit(
            Ingress::ImportStream,
            &invalid,
            &IdentityCanaries::default(),
            true,
        );
        assert_eq!(outcome.0, IngressOutcome::Quarantined);
        assert_eq!(gate.available_import_credits(), 0);
        assert!(gate.is_quarantined(Ingress::ImportStream));
    }
}
