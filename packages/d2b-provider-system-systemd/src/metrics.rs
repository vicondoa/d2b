//! Closed systemd Provider metric labels.

/// Allowed low-cardinality process metric labels.
pub const LABEL_KEYS: &[&str] = &["operation", "outcome", "domain"];

/// Validate a metric label set without accepting resource names or units.
pub fn validate_labels(labels: &[(String, String)]) -> bool {
    labels.iter().all(|(key, value)| {
        LABEL_KEYS.contains(&key.as_str())
            && value.len() <= 32
            && !value.contains('/')
            && !value.contains(':')
    })
}
