//! Closed-label metrics and OTEL span projections.

/// Closed telemetry outcome.
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

/// QMP operation label used by metrics and spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QmpOperation {
    /// Capability exchange.
    Capabilities,
    /// Continue.
    Cont,
    /// Graceful shutdown.
    SystemPowerdown,
    /// Status query.
    QueryStatus,
    /// Media attach.
    Attach,
    /// Media detach.
    Detach,
}

impl QmpOperation {
    /// Return the stable operation label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Capabilities => "capabilities",
            Self::Cont => "cont",
            Self::SystemPowerdown => "system-powerdown",
            Self::QueryStatus => "query-status",
            Self::Attach => "attach",
            Self::Detach => "detach",
        }
    }
}

/// Span kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    /// Guest reconcile span.
    Reconcile,
    /// Runner launch span.
    RunnerLaunch,
    /// QMP connect span.
    QmpConnect,
    /// QMP command span.
    QmpCommand,
    /// Media hotplug span.
    MediaHotplug,
    /// Finalization span.
    Finalize,
}

/// One bounded telemetry field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryField {
    /// Fixed field key.
    pub key: &'static str,
    /// Bounded semantic value.
    pub value: String,
}

/// Metrics frame with trusted resource attributes and closed metric labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryFrame {
    resource_attributes: Vec<TelemetryField>,
    metric_labels: Vec<TelemetryField>,
}

impl TelemetryFrame {
    /// Construct the baseline Provider frame.
    pub fn new(zone: &str, outcome: MetricOutcome) -> Self {
        Self {
            resource_attributes: vec![
                TelemetryField {
                    key: "d2b.zone",
                    value: zone.to_owned(),
                },
                TelemetryField {
                    key: "d2b.provider",
                    value: "runtime-qemu-media".to_owned(),
                },
                TelemetryField {
                    key: "service.name",
                    value: "d2b-runtime-qemu-media-controller".to_owned(),
                },
            ],
            metric_labels: vec![
                TelemetryField {
                    key: "provider",
                    value: "runtime-qemu-media".to_owned(),
                },
                TelemetryField {
                    key: "outcome",
                    value: outcome.as_str().to_owned(),
                },
            ],
        }
    }

    /// Borrow trusted resource attributes.
    pub fn resource_attributes(&self) -> &[TelemetryField] {
        &self.resource_attributes
    }

    /// Borrow metric labels.
    pub fn metric_labels(&self) -> &[TelemetryField] {
        &self.metric_labels
    }

    /// Validate all frame fields.
    pub fn validate(&self) -> Result<(), TelemetryError> {
        for field in self.resource_attributes.iter().chain(&self.metric_labels) {
            Self::validate_field(field.key, &field.value)?;
        }
        Ok(())
    }

    /// Validate one metric or resource field.
    pub fn validate_field(key: &str, value: &str) -> Result<(), TelemetryError> {
        let allowed = [
            "d2b.zone",
            "d2b.provider",
            "service.name",
            "provider",
            "outcome",
            "operation",
            "phase",
            "dep_type",
        ];
        if !allowed.contains(&key)
            || value.is_empty()
            || value.len() > 128
            || value.contains('/')
            || value.contains('\n')
            || matches!(key, "vm" | "zone_id" | "zone_uid" | "guest" | "resource")
        {
            return Err(TelemetryError::FieldRejected);
        }
        if key == "outcome" && !["success", "pending", "degraded", "failed"].contains(&value) {
            return Err(TelemetryError::FieldRejected);
        }
        if key == "provider" && value != "runtime-qemu-media" {
            return Err(TelemetryError::FieldRejected);
        }
        Ok(())
    }

    /// Construct a fixed semantic QMP span.
    pub fn span(kind: SpanKind, operation: QmpOperation, outcome: MetricOutcome) -> TelemetrySpan {
        TelemetrySpan {
            kind,
            attributes: vec![
                TelemetryField {
                    key: "operation",
                    value: operation.as_str().to_owned(),
                },
                TelemetryField {
                    key: "outcome",
                    value: outcome.as_str().to_owned(),
                },
            ],
        }
    }

    /// Validate an OTEL span attribute.
    pub fn validate_span_attribute(key: &str, value: &str) -> Result<(), TelemetryError> {
        if !["phase", "outcome", "command", "operation"].contains(&key)
            || value.is_empty()
            || value.len() > 64
            || value.contains('/')
            || value.contains('\n')
        {
            return Err(TelemetryError::SpanAttributeRejected);
        }
        Ok(())
    }
}

/// Fixed semantic OTEL span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySpan {
    /// Span kind.
    pub kind: SpanKind,
    /// Fixed semantic attributes.
    pub attributes: Vec<TelemetryField>,
}

impl TelemetrySpan {
    /// Validate the span attributes.
    pub fn validate(&self) -> Result<(), TelemetryError> {
        for field in &self.attributes {
            TelemetryFrame::validate_span_attribute(field.key, &field.value)?;
        }
        Ok(())
    }
}

/// Telemetry validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryError {
    /// Metric field is not in the closed policy.
    FieldRejected,
    /// Span attribute is not in the closed policy.
    SpanAttributeRejected,
}
