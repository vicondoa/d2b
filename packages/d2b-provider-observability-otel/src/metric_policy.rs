//! Closed metric-label and OTEL resource-attribute policy for the Provider.
//!
//! The Provider is the boundary that admits telemetry. Keeping this small
//! policy next to the ingress gate means every Provider ingress uses the same
//! structural checks without depending on a core telemetry implementation.

use std::collections::{BTreeMap, BTreeSet};

pub use d2b_contracts::v3::telemetry_policy::{
    FORBIDDEN_LABEL_KEYS, FORBIDDEN_LABEL_SUFFIXES, METRIC_LABEL_POLICY, OTEL_RESOURCE_ATTRIBUTES,
    allowed_values,
};

/// Maximum bytes in one OTEL resource attribute value.
pub const MAX_RESOURCE_ATTRIBUTE_BYTES: usize = 256;

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

/// Construct a descriptor label from a static value domain.
pub fn label(key: impl Into<String>, values: &[&str]) -> LabelDescriptor {
    LabelDescriptor::new(key, values.iter().copied())
}

/// A metric descriptor accepted by the structural policy.
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
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IdentityCanaries {
    names: BTreeSet<String>,
    uids: BTreeSet<String>,
    refs: BTreeSet<String>,
}

impl core::fmt::Debug for IdentityCanaries {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("IdentityCanaries")
            .field("name_count", &self.names.len())
            .field("uid_count", &self.uids.len())
            .field("ref_count", &self.refs.len())
            .finish()
    }
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

/// A metric policy failure. Variants deliberately contain no input text.
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
    /// A metric name is not in the canonical family registry.
    DescriptorNotAllowlisted,
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
            Self::DescriptorNotAllowlisted => "metric-descriptor-not-allowlisted",
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
    let Some(canonical) = canonical_descriptor(&descriptor.name) else {
        return Err(MetricPolicyError::DescriptorNotAllowlisted);
    };

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
    if descriptor.labels.len() != canonical.labels.len()
        || !canonical
            .labels
            .iter()
            .all(|expected| descriptor.labels.iter().any(|actual| actual == expected))
    {
        return Err(MetricPolicyError::DescriptorMalformed);
    }
    Ok(())
}

/// Resolve a metric family from the canonical contract registry.
pub fn canonical_descriptor(name: &str) -> Option<MetricDescriptor> {
    let descriptor = d2b_contracts::v3::metric_descriptor(name)?;
    let labels = descriptor
        .labels
        .iter()
        .map(|(key, values)| label(*key, values))
        .collect::<Vec<_>>();
    Some(MetricDescriptor::new(name, labels))
}

/// Validate a label key before considering any value.
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

/// Validate one set of attributes before it can enter a telemetry frame.
pub fn validate_resource_attributes(
    attributes: &BTreeMap<String, String>,
) -> Result<(), ResourceAttributeError> {
    let mut seen = BTreeSet::new();
    for (key, value) in attributes {
        if !OTEL_RESOURCE_ATTRIBUTES.contains(&key.as_str()) {
            return Err(ResourceAttributeError::NotAllowlisted);
        }
        if value.is_empty()
            || value.len() > MAX_RESOURCE_ATTRIBUTE_BYTES
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_graphic() || byte == b'/')
            || !seen.insert(key)
            || !valid_resource_attribute_value(key, value)
        {
            return Err(ResourceAttributeError::Invalid);
        }

        fn valid_resource_attribute_value(key: &str, value: &str) -> bool {
            let identity_key = matches!(
                key,
                "d2b.zone"
                    | "d2b.provider"
                    | "d2b.component"
                    | "host.name"
                    | "vm.name"
                    | "vm.env"
                    | "vm.role"
            );
            if identity_key {
                return d2b_contracts::v3::is_canonical_digest(value);
            }
            value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
        }
    }
    Ok(())
}

/// Closed resource-attribute validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceAttributeError {
    /// The attribute key is outside the OTEL allowlist.
    NotAllowlisted,
    /// The value or set shape is invalid.
    Invalid,
}

impl core::fmt::Display for ResourceAttributeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::NotAllowlisted => "otel-resource-attribute-not-allowlisted",
            Self::Invalid => "otel-resource-attribute-invalid",
        })
    }
}

impl std::error::Error for ResourceAttributeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_identity_keys_fail_structurally() {
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
    fn descriptor_validation_rejects_identity_canaries() {
        let descriptor =
            canonical_descriptor("d2b_store_write_duration_seconds").expect("store descriptor");
        let canaries = IdentityCanaries::new(["resource-name"], ["uid-value"], ["Process/name"]);
        let labels = BTreeMap::from([
            ("kind".to_owned(), "single".to_owned()),
            ("outcome".to_owned(), "resource-name".to_owned()),
        ]);
        assert_eq!(
            validate_data_point(&descriptor, &labels, &canaries),
            Err(MetricPolicyError::ValueNotAllowlisted)
        );
    }

    #[test]
    fn resource_attributes_have_a_separate_allowlist() {
        let attributes = BTreeMap::from([
            (
                "d2b.zone".to_owned(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                    .to_owned(),
            ),
            ("service.version".to_owned(), "0.0.0".to_owned()),
        ]);
        assert!(validate_resource_attributes(&attributes).is_ok());
        assert!(
            validate_resource_attributes(&BTreeMap::from([("zone".to_owned(), "work".to_owned())]))
                .is_err()
        );
    }
}
