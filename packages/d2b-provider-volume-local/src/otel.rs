//! Closed telemetry descriptors for Volume state.
//!
//! Metric names, label keys, and label values are enums. Zone identity is an
//! OTEL resource attribute only and cannot enter a metric label set.

/// Metric instrument kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    /// Current observed value.
    Gauge,
    /// Monotonic event count.
    Counter,
    /// Distribution of observed durations.
    Histogram,
}

/// Metric unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricUnit {
    /// Bytes.
    Bytes,
    /// Milliseconds.
    Milliseconds,
    /// Dimensionless count.
    Count,
}

/// Closed metric label key set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricLabelKey {
    /// Fixed Provider implementation class.
    Provider,
    /// Fixed schema class, never raw schema content.
    SchemaId,
    /// Closed transition outcome.
    Outcome,
    /// Closed snapshot trigger.
    Trigger,
}

impl MetricLabelKey {
    /// Return the stable OTEL key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::SchemaId => "schema_id",
            Self::Outcome => "outcome",
            Self::Trigger => "trigger",
        }
    }
}

/// Fixed Provider label values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderLabel {
    /// The local Volume Provider.
    VolumeLocal,
}

impl ProviderLabel {
    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VolumeLocal => "volume-local",
        }
    }
}

/// Closed schema classes used for metrics.
///
/// Audit may carry the bounded declared schema ID. Metrics use this smaller
/// semantic class to prevent installed resource names from creating series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaLabel {
    /// A declared Provider payload state schema.
    ProviderState,
    /// The closure-only store-view layout.
    StoreView,
    /// Persistent TPM state.
    SwtpmState,
}

impl SchemaLabel {
    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderState => "provider-state",
            Self::StoreView => "store-view",
            Self::SwtpmState => "swtpm-state",
        }
    }
}

/// Closed metric outcome values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricOutcome {
    /// The operation completed.
    Succeeded,
    /// The operation failed closed.
    Failed,
    /// Marker identity verified.
    Verified,
    /// Marker was missing.
    Missing,
    /// Root identity was replaced.
    Replaced,
}

impl MetricOutcome {
    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Verified => "verified",
            Self::Missing => "missing",
            Self::Replaced => "replaced",
        }
    }
}

/// Closed snapshot trigger values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricTrigger {
    /// Explicit operator request.
    Manual,
    /// Automatic pre-migration snapshot.
    PreMigration,
    /// Automatic pre-relocation snapshot.
    PreRelocation,
}

impl MetricTrigger {
    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PreMigration => "pre-migration",
            Self::PreRelocation => "pre-relocation",
        }
    }
}

/// One closed label value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricLabelValue {
    /// Fixed Provider class.
    Provider(ProviderLabel),
    /// Closed schema class.
    Schema(SchemaLabel),
    /// Closed outcome.
    Outcome(MetricOutcome),
    /// Closed snapshot trigger.
    Trigger(MetricTrigger),
}

impl MetricLabelValue {
    /// Return the key paired with this value.
    pub const fn key(self) -> MetricLabelKey {
        match self {
            Self::Provider(_) => MetricLabelKey::Provider,
            Self::Schema(_) => MetricLabelKey::SchemaId,
            Self::Outcome(_) => MetricLabelKey::Outcome,
            Self::Trigger(_) => MetricLabelKey::Trigger,
        }
    }

    /// Return the stable closed value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider(value) => value.as_str(),
            Self::Schema(value) => value.as_str(),
            Self::Outcome(value) => value.as_str(),
            Self::Trigger(value) => value.as_str(),
        }
    }
}

/// One fixed metric definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDescriptor {
    /// Stable instrument name.
    pub name: &'static str,
    /// Instrument kind.
    pub kind: MetricKind,
    /// Instrument unit.
    pub unit: MetricUnit,
    /// Exact allowed label keys in canonical order.
    pub labels: &'static [MetricLabelKey],
}

const PROVIDER_SCHEMA: &[MetricLabelKey] = &[MetricLabelKey::Provider, MetricLabelKey::SchemaId];
const PROVIDER_SCHEMA_OUTCOME: &[MetricLabelKey] = &[
    MetricLabelKey::Provider,
    MetricLabelKey::SchemaId,
    MetricLabelKey::Outcome,
];
const PROVIDER_SCHEMA_TRIGGER: &[MetricLabelKey] = &[
    MetricLabelKey::Provider,
    MetricLabelKey::SchemaId,
    MetricLabelKey::Trigger,
];
const PROVIDER_OUTCOME: &[MetricLabelKey] = &[MetricLabelKey::Provider, MetricLabelKey::Outcome];
const PROVIDER: &[MetricLabelKey] = &[MetricLabelKey::Provider];

/// Every Volume-state metric definition.
pub const METRICS: [MetricDescriptor; 6] = [
    MetricDescriptor {
        name: "d2b_volume_state_size_bytes",
        kind: MetricKind::Gauge,
        unit: MetricUnit::Bytes,
        labels: PROVIDER_SCHEMA,
    },
    MetricDescriptor {
        name: "d2b_volume_state_migration_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_SCHEMA_OUTCOME,
    },
    MetricDescriptor {
        name: "d2b_volume_state_migration_duration_ms",
        kind: MetricKind::Histogram,
        unit: MetricUnit::Milliseconds,
        labels: PROVIDER_SCHEMA,
    },
    MetricDescriptor {
        name: "d2b_volume_state_snapshot_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_SCHEMA_TRIGGER,
    },
    MetricDescriptor {
        name: "d2b_volume_state_marker_check_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_OUTCOME,
    },
    MetricDescriptor {
        name: "d2b_volume_state_quota_exceeded_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER,
    },
];

/// Closed OTEL resource attribute keys emitted by this Provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceAttributeKey {
    /// Zone identity belongs only on the OTEL Resource.
    D2bZone,
    /// Provider implementation identity.
    D2bProvider,
    /// Provider component class.
    D2bComponent,
    /// Standard service name.
    ServiceName,
    /// Standard service version.
    ServiceVersion,
}

impl ResourceAttributeKey {
    /// Return the stable OTEL Resource key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::D2bZone => "d2b.zone",
            Self::D2bProvider => "d2b.provider",
            Self::D2bComponent => "d2b.component",
            Self::ServiceName => "service.name",
            Self::ServiceVersion => "service.version",
        }
    }
}

/// Every Resource attribute key this Provider may emit.
pub const RESOURCE_ATTRIBUTE_KEYS: [ResourceAttributeKey; 5] = [
    ResourceAttributeKey::D2bZone,
    ResourceAttributeKey::D2bProvider,
    ResourceAttributeKey::D2bComponent,
    ResourceAttributeKey::ServiceName,
    ResourceAttributeKey::ServiceVersion,
];

/// Validate a sample's label keys against its fixed descriptor.
pub fn validate_labels(descriptor: &MetricDescriptor, labels: &[MetricLabelValue]) -> bool {
    labels.len() == descriptor.labels.len()
        && labels
            .iter()
            .zip(descriptor.labels)
            .all(|(value, key)| value.key() == *key)
}
