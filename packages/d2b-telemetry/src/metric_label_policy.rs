//! Closed metric-label and identity-redaction policy.

use std::collections::{BTreeMap, BTreeSet};

pub use d2b_contracts::v3::telemetry_policy::{
    FORBIDDEN_LABEL_KEYS, FORBIDDEN_LABEL_SUFFIXES, METRIC_LABEL_POLICY, OTEL_RESOURCE_ATTRIBUTES,
    allowed_values,
};

/// A descriptor label and its closed value domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDescriptor {
    key: String,
    values: BTreeSet<String>,
}

impl LabelDescriptor {
    /// Construct a descriptor label.
    pub fn new(
        key: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            key: key.into(),
            values: values.into_iter().map(Into::into).collect(),
        }
    }

    /// Borrow the label key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Borrow the closed value domain.
    pub fn values(&self) -> &BTreeSet<String> {
        &self.values
    }
}

/// A metric descriptor accepted by the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricDescriptor {
    name: String,
    labels: Vec<LabelDescriptor>,
}

impl MetricDescriptor {
    /// Construct a metric descriptor.
    pub fn new(name: impl Into<String>, labels: impl IntoIterator<Item = LabelDescriptor>) -> Self {
        Self {
            name: name.into(),
            labels: labels.into_iter().collect(),
        }
    }

    /// Borrow the metric name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow descriptor labels.
    pub fn labels(&self) -> &[LabelDescriptor] {
        &self.labels
    }
}

/// Identity values which must not enter a metric data point.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityCanaries {
    names: BTreeSet<String>,
    uids: BTreeSet<String>,
    refs: BTreeSet<String>,
}

impl IdentityCanaries {
    /// Construct canaries from trusted resource observations.
    pub fn new(
        names: impl IntoIterator<Item = impl Into<String>>,
        uids: impl IntoIterator<Item = impl Into<String>>,
        refs: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            names: names.into_iter().map(Into::into).collect(),
            uids: uids.into_iter().map(Into::into).collect(),
            refs: refs.into_iter().map(Into::into).collect(),
        }
    }

    fn contains(&self, value: &str) -> bool {
        self.names.contains(value) || self.uids.contains(value) || self.refs.contains(value)
    }
}

/// A metric policy failure. The variants deliberately contain no input text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricPolicyError {
    /// A descriptor key is not in the policy.
    KeyNotAllowlisted,
    /// A descriptor key is unconditionally forbidden.
    KeyForbidden,
    /// A descriptor key has a forbidden identity suffix.
    KeySuffixForbidden,
    /// A data point key does not match its descriptor.
    LabelSetMismatch,
    /// A value is outside its closed domain.
    ValueNotAllowlisted,
    /// A value is a resource identity canary.
    ValueIdentity,
    /// A metric name is empty or malformed.
    DescriptorMalformed,
}

impl core::fmt::Display for MetricPolicyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::KeyNotAllowlisted => "metric-label-key-not-allowlisted",
            Self::KeyForbidden => "metric-label-key-forbidden",
            Self::KeySuffixForbidden => "metric-label-key-suffix-forbidden",
            Self::LabelSetMismatch => "metric-label-set-mismatch",
            Self::ValueNotAllowlisted => "metric-label-value-not-allowlisted",
            Self::ValueIdentity => "metric-label-value-identity",
            Self::DescriptorMalformed => "metric-descriptor-malformed",
        })
    }
}

impl std::error::Error for MetricPolicyError {}

/// Validate one metric descriptor against the closed registry.
pub fn validate_descriptor(descriptor: &MetricDescriptor) -> Result<(), MetricPolicyError> {
    if descriptor.name.is_empty()
        || descriptor.name.len() > 128
        || !descriptor
            .name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(MetricPolicyError::DescriptorMalformed);
    }

    let mut seen = BTreeSet::new();
    if descriptor.labels.len() > 16 {
        return Err(MetricPolicyError::DescriptorMalformed);
    }
    for label in &descriptor.labels {
        if label.key.is_empty() || label.key.len() > 64 {
            return Err(MetricPolicyError::DescriptorMalformed);
        }
        if !seen.insert(label.key.clone()) {
            return Err(MetricPolicyError::DescriptorMalformed);
        }
        validate_label_key(&label.key)?;
        let Some(allowed) = allowed_values(&label.key) else {
            return Err(MetricPolicyError::KeyNotAllowlisted);
        };
        if label.values.is_empty()
            || label
                .values
                .iter()
                .any(|value| !allowed.iter().any(|candidate| candidate == value))
        {
            return Err(MetricPolicyError::ValueNotAllowlisted);
        }
    }
    Ok(())
}

/// Validate a label key before any value is considered.
pub fn validate_label_key(key: &str) -> Result<(), MetricPolicyError> {
    if FORBIDDEN_LABEL_KEYS.contains(&key) {
        return Err(MetricPolicyError::KeyForbidden);
    }
    if FORBIDDEN_LABEL_SUFFIXES
        .iter()
        .any(|suffix| key.ends_with(suffix))
    {
        return Err(MetricPolicyError::KeySuffixForbidden);
    }
    if allowed_values(key).is_none() {
        return Err(MetricPolicyError::KeyNotAllowlisted);
    }
    Ok(())
}

/// Validate one data point against its descriptor and identity canaries.
pub fn validate_data_point(
    descriptor: &MetricDescriptor,
    labels: &BTreeMap<String, String>,
    canaries: &IdentityCanaries,
) -> Result<(), MetricPolicyError> {
    validate_descriptor(descriptor)?;
    let descriptor_keys = descriptor
        .labels
        .iter()
        .map(|label| label.key.as_str())
        .collect::<BTreeSet<_>>();
    let actual_keys = labels.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if descriptor_keys != actual_keys {
        return Err(MetricPolicyError::LabelSetMismatch);
    }
    for label in &descriptor.labels {
        let value = labels
            .get(label.key())
            .ok_or(MetricPolicyError::LabelSetMismatch)?;
        if !label.values.contains(value) {
            return Err(MetricPolicyError::ValueNotAllowlisted);
        }
        if canaries.contains(value) {
            return Err(MetricPolicyError::ValueIdentity);
        }
    }
    Ok(())
}

/// Validate labels when a frame does not carry a full descriptor.
///
/// The emitter uses this defense-in-depth check for compact metric frames.
/// Descriptor-bearing collector ingress should use [`validate_data_point`]
/// instead so it also enforces the exact label set.
pub fn validate_labels(
    labels: &BTreeMap<String, String>,
    canaries: &IdentityCanaries,
) -> Result<(), MetricPolicyError> {
    if labels.len() > 16 {
        return Err(MetricPolicyError::DescriptorMalformed);
    }
    for (key, value) in labels {
        validate_label_key(key)?;
        let Some(allowed) = allowed_values(key) else {
            return Err(MetricPolicyError::KeyNotAllowlisted);
        };
        if !allowed.iter().any(|candidate| candidate == value) {
            return Err(MetricPolicyError::ValueNotAllowlisted);
        }
        if canaries.contains(value) {
            return Err(MetricPolicyError::ValueIdentity);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_identity_keys_fail_before_values() {
        for key in FORBIDDEN_LABEL_KEYS {
            assert_eq!(
                validate_label_key(key),
                Err(MetricPolicyError::KeyForbidden)
            );
        }
        for key in ["resource_name", "zone_uid", "link_name_hash"] {
            assert!(validate_label_key(key).is_err());
        }
    }

    #[test]
    fn descriptor_and_identity_canary_validation_are_structural() {
        let descriptor = MetricDescriptor::new(
            "d2b_store_write_total",
            [LabelDescriptor::new("outcome", ["ok", "error"])],
        );
        validate_descriptor(&descriptor).unwrap();
        let canaries = IdentityCanaries::new(["resource-name"], ["uid-value"], ["Process/name"]);
        let mut labels = BTreeMap::from([("outcome".to_owned(), "ok".to_owned())]);
        validate_data_point(&descriptor, &labels, &canaries).unwrap();
        labels.insert("outcome".to_owned(), "resource-name".to_owned());
        assert_eq!(
            validate_data_point(&descriptor, &labels, &canaries),
            Err(MetricPolicyError::ValueNotAllowlisted)
        );
    }

    #[test]
    fn untyped_labels_reject_out_of_policy_keys_and_identity_values() {
        let canaries = IdentityCanaries::new(["work"], ["uid"], ["Zone/work"]);
        let labels = BTreeMap::from([("zone".to_owned(), "work".to_owned())]);
        assert_eq!(
            validate_labels(&labels, &canaries),
            Err(MetricPolicyError::KeyForbidden)
        );
        let labels = BTreeMap::from([("outcome".to_owned(), "work".to_owned())]);
        assert_eq!(
            validate_labels(&labels, &canaries),
            Err(MetricPolicyError::ValueNotAllowlisted)
        );
    }

    #[test]
    fn resource_attributes_have_a_separate_allowlist() {
        assert!(OTEL_RESOURCE_ATTRIBUTES.contains(&"d2b.zone"));
        assert!(!OTEL_RESOURCE_ATTRIBUTES.contains(&"zone"));
    }
}
