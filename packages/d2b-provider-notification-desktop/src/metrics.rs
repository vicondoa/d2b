//! Closed-label notification telemetry.

/// A telemetry field with a fixed key and bounded semantic value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationTelemetryField {
    /// Fixed allowlisted field key.
    pub key: &'static str,
    /// Closed semantic value.
    pub value: String,
}

/// Content-free notification telemetry frame.
pub struct NotificationTelemetryFrame {
    resource_attributes: Vec<NotificationTelemetryField>,
    metric_labels: Vec<NotificationTelemetryField>,
}

impl NotificationTelemetryFrame {
    /// Construct a frame retaining only trusted generic resource attributes.
    pub fn new(zone: &str, category: crate::Category, outcome: &str) -> Self {
        Self {
            resource_attributes: vec![
                NotificationTelemetryField {
                    key: "d2b.zone",
                    value: zone.to_owned(),
                },
                NotificationTelemetryField {
                    key: "d2b.provider",
                    value: "notification-desktop".to_owned(),
                },
            ],
            metric_labels: vec![
                NotificationTelemetryField {
                    key: "category",
                    value: category.as_str().to_owned(),
                },
                NotificationTelemetryField {
                    key: "outcome",
                    value: outcome.to_owned(),
                },
            ],
        }
    }

    /// Borrow resource attributes.
    pub fn resource_attributes(&self) -> &[NotificationTelemetryField] {
        &self.resource_attributes
    }

    /// Borrow metric labels.
    pub fn metric_labels(&self) -> &[NotificationTelemetryField] {
        &self.metric_labels
    }

    /// Validate a collector frame and reject identity/content fields.
    pub fn validate_collector_fields(
        fields: impl IntoIterator<Item = NotificationTelemetryField>,
    ) -> Result<(), &'static str> {
        let forbidden = [
            "summary",
            "body",
            "icon",
            "action",
            "notification_id",
            "vm",
            "zone_id",
            "zone_uid",
        ];
        for field in fields {
            if forbidden.contains(&field.key)
                || field.value.contains('\n')
                || field.value.len() > 128
            {
                return Err("notification-telemetry-field-rejected");
            }
        }
        Ok(())
    }
}

impl core::fmt::Debug for NotificationTelemetryFrame {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NotificationTelemetryFrame")
            .field("resource_attribute_count", &self.resource_attributes.len())
            .field("metric_label_count", &self.metric_labels.len())
            .finish()
    }
}
