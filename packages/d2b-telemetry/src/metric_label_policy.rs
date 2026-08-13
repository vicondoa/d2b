//! Closed metric-label and identity-redaction policy.

use std::collections::BTreeMap;

pub use d2b_contracts::v3::telemetry_policy::{
    FORBIDDEN_LABEL_KEYS, FORBIDDEN_LABEL_SUFFIXES, IdentityCanaries, LabelDescriptor,
    METRIC_LABEL_POLICY, MetricDescriptor, MetricPolicyError, OTEL_RESOURCE_ATTRIBUTES,
    allowed_values, canonical_descriptor, validate_canonical_data_point, validate_data_point,
    validate_descriptor, validate_label_key, validate_labels,
};

/// Validate resource attributes with key-specific identity handling.
pub fn validate_resource_attributes(
    attributes: &BTreeMap<String, String>,
) -> Result<(), MetricPolicyError> {
    for (key, value) in attributes {
        if !OTEL_RESOURCE_ATTRIBUTES.contains(&key.as_str())
            || value.is_empty()
            || value.len() > 256
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_graphic() || byte == b'/')
        {
            return Err(MetricPolicyError::DescriptorMalformed);
        }
        if matches!(
            key.as_str(),
            "d2b.zone"
                | "d2b.provider"
                | "d2b.component"
                | "host.name"
                | "vm.name"
                | "vm.env"
                | "vm.role"
        ) && !is_canonical_digest(value)
        {
            return Err(MetricPolicyError::ValueIdentity);
        }
    }
    Ok(())
}

fn is_canonical_digest(value: &str) -> bool {
    d2b_contracts::v3::is_canonical_digest(value)
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
            "d2b_store_compaction_duration_seconds",
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
        assert!(
            validate_resource_attributes(&BTreeMap::from([(
                "d2b.zone".to_owned(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                    .to_owned(),
            )]))
            .is_ok()
        );
    }
}
