//! Closed metric-label and OTEL resource-attribute policy for the Provider.
//!
//! The Provider is the boundary that admits telemetry. Keeping this small
//! policy next to the ingress gate means every Provider ingress uses the same
//! structural checks without depending on a core telemetry implementation.

use std::collections::{BTreeMap, BTreeSet};

/// Exact keys which can never be metric dimensions.
pub const FORBIDDEN_LABEL_KEYS: &[&str] = &[
    "vm",
    "zone",
    "zone_id",
    "zone_uid",
    "credential",
    "credential_name",
    "network",
    "network_name",
    "guest",
    "host",
    "user",
    "volume",
    "device",
    "process",
    "link_name_hash",
];

/// Suffixes which identify a resource dimension.
pub const FORBIDDEN_LABEL_SUFFIXES: &[&str] = &["_name", "_name_hash", "_name_digest", "_uid"];

/// Resource attributes allowed on OTEL records.
pub const OTEL_RESOURCE_ATTRIBUTES: &[&str] = &[
    "deployment.environment",
    "host.name",
    "service.name",
    "service.namespace",
    "source",
    "vm.env",
    "vm.name",
    "vm.role",
    "d2b.zone",
    "d2b.provider",
    "d2b.component",
    "service.version",
];

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
        {
            return Err(ResourceAttributeError::Invalid);
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

/// Return the closed value domain for one accepted label key.
pub fn allowed_values(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "service" => Some(&[
            "resource-api",
            "bus",
            "session",
            "store",
            "controller",
            "d2b.resource.v3",
            "d2b.controller.v3",
            "d2b.provider.v3",
            "d2b.audit.v3",
            "d2b.support.v3",
            "d2b.credential.v3",
            "d2b.zone.v3",
            "d2b.zonelink.v3",
            "d2b.volume.v3",
        ]),
        "direction" => Some(&["local", "host", "guest", "zone_link", "send", "recv"]),
        "outcome" => Some(&[
            "ok",
            "error",
            "denied",
            "not_found",
            "conflict",
            "invalid",
            "quota",
            "auth",
            "transcript",
            "policy",
            "timeout",
            "requeue",
            "rate_limited",
            "abandoned",
            "dropped",
            "buffering",
            "accepted",
            "rejected",
            "quarantined",
            "cancel",
            "revoked",
            "degraded",
        ]),
        "resource_type" | "exported_type" | "projection_type" => Some(&[
            "Zone",
            "ZoneLink",
            "Provider",
            "Role",
            "RoleBinding",
            "Quota",
            "Host",
            "Guest",
            "Process",
            "EphemeralProcess",
            "Volume",
            "Network",
            "Device",
            "User",
            "Credential",
            "Endpoint",
            "ResourceExport",
            "ResourceImport",
            "vendor",
        ]),
        "verb" => Some(&[
            "get",
            "list",
            "watch",
            "create",
            "update-spec",
            "update-status",
            "update-metadata",
            "update-finalizers",
            "delete",
            "use-credential",
            "admin-credential",
        ]),
        "profile" => Some(&["NN", "KK", "IKpsk2"]),
        "purpose_class" => Some(&["local", "enrolled", "bootstrap"]),
        "transport" => Some(&["unix", "vsock", "zone_link"]),
        "kind" => Some(&[
            "control",
            "ttrpc",
            "stream",
            "attachment",
            "single",
            "group",
        ]),
        "handler" => Some(&[
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
        ]),
        "provider" => Some(&[
            "minijail",
            "systemd",
            "system-core-user",
            "system-core",
            "observability-otel",
        ]),
        "domain" => Some(&["system", "user"]),
        "operation" => Some(&[
            "get",
            "list",
            "scan",
            "create",
            "update",
            "delete",
            "admit",
            "revoke",
            "reconnect",
            "write",
            "read",
            "cancel",
        ]),
        "record_class" => Some(&[
            "resource-mutation",
            "resource-upgrade",
            "rbac-change",
            "session-connect",
            "route-admission",
            "resource-share",
            "broker-effect",
            "process-effect",
            "state-reset",
            "privileged",
            "unprivileged",
        ]),
        "provider_class" => Some(&[
            "system-core",
            "system-minijail",
            "system-systemd",
            "observability-otel",
        ]),
        "phase" => Some(&[
            "pending", "ready", "degraded", "failed", "unknown", "revoked",
        ]),
        "signal" => Some(&["metric", "trace", "log"]),
        "reason" => Some(&[
            "buffer_full",
            "export_error",
            "policy_violation",
            "ingress_quarantine",
            "auth",
            "quota",
            "conflict",
            "invalid",
            "schema",
        ]),
        "ingress" => Some(&["emitter_unix", "otlp_unix", "otlp_vsock", "import_stream"]),
        "error_class" => Some(&[
            "none",
            "key_not_allowlisted",
            "key_forbidden",
            "key_suffix_forbidden",
            "value_identity",
            "malformed",
            "oversize",
        ]),
        "stop_class" => Some(&["graceful", "forced"]),
        "class" => Some(&["exited", "signaled", "killed"]),
        "arbitration" => Some(&["exclusive", "shared", "multiplexed"]),
        "state" => Some(&[
            "advertised",
            "ready",
            "revoking",
            "degraded",
            "pending",
            "reachable",
            "bound",
            "revoked",
            "active",
        ]),
        "component_type" => Some(&["controller", "service", "worker"]),
        "disposition" => Some(&["none", "reload", "restart", "recycle", "replace"]),
        _ => None,
    }
}

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
            MetricDescriptor::new("d2b_store_write_total", [label("outcome", &["ok"])]);
        let canaries = IdentityCanaries::new(["resource-name"], ["uid-value"], ["Process/name"]);
        let labels = BTreeMap::from([("outcome".to_owned(), "resource-name".to_owned())]);
        assert_eq!(
            validate_data_point(&descriptor, &labels, &canaries),
            Err(MetricPolicyError::ValueNotAllowlisted)
        );
    }

    #[test]
    fn resource_attributes_have_a_separate_allowlist() {
        let attributes = BTreeMap::from([
            ("d2b.zone".to_owned(), "work".to_owned()),
            ("service.version".to_owned(), "0.0.0".to_owned()),
        ]);
        assert!(validate_resource_attributes(&attributes).is_ok());
        assert!(
            validate_resource_attributes(&BTreeMap::from([("zone".to_owned(), "work".to_owned())]))
                .is_err()
        );
    }
}
