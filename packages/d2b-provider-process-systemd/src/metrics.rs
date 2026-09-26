//! Closed systemd Provider metric labels.

/// Low-cardinality process metric label key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricLabelKey {
    /// The operation class being measured.
    Operation,
    /// The terminal outcome of the operation.
    Outcome,
    /// The execution domain the operation ran in.
    Domain,
}

impl MetricLabelKey {
    /// Return the stable lower-kebab label key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operation => "operation",
            Self::Outcome => "outcome",
            Self::Domain => "domain",
        }
    }
}

/// Validate a metric label set without accepting resource names or units.
pub fn validate_labels(labels: &[(MetricLabelKey, String)]) -> bool {
    labels.iter().all(|(_, value)| {
        value.len() <= 32
            && !value.contains('/')
            && !value.contains(':')
    })
}