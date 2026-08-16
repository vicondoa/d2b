//! Closed-label display telemetry.

use std::collections::BTreeMap;

/// Closed display metric outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricOutcome {
    /// Operation succeeded.
    Success,
    /// Operation is pending.
    Pending,
    /// Operation degraded.
    Degraded,
    /// Operation failed.
    Failed,
}

impl MetricOutcome {
    /// Return the stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Pending => "pending",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }
}

/// A validated telemetry field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayTelemetryField {
    /// Field key from the fixed allowlist.
    pub key: &'static str,
    /// Bounded semantic value.
    pub value: String,
}

/// A display telemetry frame with closed labels and trusted resource attrs.
pub struct DisplayTelemetryFrame {
    resource_attributes: Vec<DisplayTelemetryField>,
    metric_labels: Vec<DisplayTelemetryField>,
}

impl DisplayTelemetryFrame {
    /// Construct a frame. Zone is a trusted resource attribute, not a metric
    /// dimension.
    pub fn new(zone: &str, outcome: MetricOutcome) -> Self {
        Self {
            resource_attributes: vec![
                DisplayTelemetryField {
                    key: "d2b.zone",
                    value: zone.to_owned(),
                },
                DisplayTelemetryField {
                    key: "d2b.provider",
                    value: "display-wayland".to_owned(),
                },
                DisplayTelemetryField {
                    key: "service.name",
                    value: "d2b-display-wayland-controller".to_owned(),
                },
            ],
            metric_labels: vec![DisplayTelemetryField {
                key: "outcome",
                value: outcome.as_str().to_owned(),
            }],
        }
    }

    /// Borrow trusted resource attributes.
    pub fn resource_attributes(&self) -> &[DisplayTelemetryField] {
        &self.resource_attributes
    }

    /// Borrow closed metric labels.
    pub fn metric_labels(&self) -> &[DisplayTelemetryField] {
        &self.metric_labels
    }

    /// Reject an entire frame containing identity/path-bearing fields.
    pub fn validate_collector_fields(
        fields: impl IntoIterator<Item = DisplayTelemetryField>,
    ) -> Result<(), &'static str> {
        let forbidden_keys = [
            "vm",
            "vm.name",
            "zone_id",
            "zone_uid",
            "guest",
            "user",
            "socket_path",
            "window_title",
            "app_id",
        ];
        for field in fields {
            if forbidden_keys.contains(&field.key)
                || field.value.contains('/')
                || field.value.contains('\n')
                || field.value.len() > 128
            {
                return Err("display-telemetry-field-rejected");
            }
        }
        Ok(())
    }

    /// Return metric labels as a fixed map.
    pub fn metric_map(&self) -> BTreeMap<&'static str, &str> {
        self.metric_labels
            .iter()
            .map(|field| (field.key, field.value.as_str()))
            .collect()
    }
}

impl core::fmt::Debug for DisplayTelemetryFrame {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DisplayTelemetryFrame")
            .field("resource_attribute_count", &self.resource_attributes.len())
            .field("metric_label_count", &self.metric_labels.len())
            .finish()
    }
}
