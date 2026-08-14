//! Closed-label notification telemetry.

/// Closed notification outcome vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationOutcome {
    /// The sink accepted and presented the notification.
    Accepted,
    /// The sink was unavailable.
    SinkUnavailable,
    /// The bounded projection capacity was exhausted.
    CapacityExceeded,
    /// The request failed validation or admission.
    Rejected,
    /// An observer action capability was invoked.
    ActionInvoked,
}

impl NotificationOutcome {
    /// Return the stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::SinkUnavailable => "sink-unavailable",
            Self::CapacityExceeded => "capacity-exceeded",
            Self::Rejected => "rejected",
            Self::ActionInvoked => "action-invoked",
        }
    }
}

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
    pub fn new(zone: &str, category: crate::Category, outcome: NotificationOutcome) -> Self {
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
                    value: outcome.as_str().to_owned(),
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
        let allowed = ["d2b.zone", "d2b.provider", "category", "outcome"];
        let mut seen = std::collections::BTreeSet::new();
        for field in fields {
            if forbidden.contains(&field.key)
                || !allowed.contains(&field.key)
                || !seen.insert(field.key)
                || field.value.contains('\n')
                || field.value.len() > 128
            {
                return Err("notification-telemetry-field-rejected");
            }
            match field.key {
                "d2b.provider" if field.value != "notification-desktop" => {
                    return Err("notification-telemetry-field-rejected");
                }
                "category"
                    if ![
                        "device.added",
                        "device.removed",
                        "device.error",
                        "network.connected",
                        "network.disconnected",
                        "network.error",
                        "presence.online",
                        "presence.offline",
                        "security.event",
                        "security.error",
                        "transfer.complete",
                        "transfer.error",
                        "transfer.cancelled",
                        "update.available",
                        "update.downloading",
                        "update.ready",
                        "update.error",
                        "system.info",
                        "system.warning",
                        "system.error",
                    ]
                    .contains(&field.value.as_str()) =>
                {
                    return Err("notification-telemetry-field-rejected");
                }
                "outcome"
                    if ![
                        "accepted",
                        "sink-unavailable",
                        "capacity-exceeded",
                        "rejected",
                        "action-invoked",
                    ]
                    .contains(&field.value.as_str()) =>
                {
                    return Err("notification-telemetry-field-rejected");
                }
                _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_outcomes_and_categories_are_closed() {
        let frame = NotificationTelemetryFrame::new(
            "work",
            crate::Category::SystemInfo,
            NotificationOutcome::Accepted,
        );
        assert!(
            NotificationTelemetryFrame::validate_collector_fields(
                frame
                    .resource_attributes()
                    .iter()
                    .chain(frame.metric_labels())
                    .cloned()
            )
            .is_ok()
        );
        assert!(
            NotificationTelemetryFrame::validate_collector_fields([NotificationTelemetryField {
                key: "outcome",
                value: "caller-controlled".to_owned(),
            }])
            .is_err()
        );
    }
}
