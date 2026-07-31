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
    /// Closed declared schema version class.
    SchemaVersion,
    /// Closed state persistence class.
    PersistenceClass,
    /// Closed Volume backing class.
    SourceKind,
    /// Closed operation class.
    Operation,
    /// Closed transition outcome.
    Outcome,
    /// Closed snapshot trigger.
    Trigger,
    /// Closed view scope, never a configured view name.
    View,
    /// Closed attachment access class.
    Access,
}

impl MetricLabelKey {
    /// Every allowed metric label key in canonical order.
    pub const ALL: [Self; 10] = [
        Self::Provider,
        Self::SchemaId,
        Self::SchemaVersion,
        Self::PersistenceClass,
        Self::SourceKind,
        Self::Operation,
        Self::Trigger,
        Self::View,
        Self::Access,
        Self::Outcome,
    ];

    /// Return the stable OTEL key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::SchemaId => "schema_id",
            Self::SchemaVersion => "schema_version",
            Self::PersistenceClass => "persistence_class",
            Self::SourceKind => "source_kind",
            Self::Operation => "operation",
            Self::Outcome => "outcome",
            Self::Trigger => "trigger",
            Self::View => "view",
            Self::Access => "access",
        }
    }
}

/// Closed schema version classes used for metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaVersionLabel {
    /// The installed schema is current.
    Current,
    /// The installed schema requires migration.
    MigrationRequired,
}

impl SchemaVersionLabel {
    /// Every allowed schema version class.
    pub const ALL: [Self; 2] = [Self::Current, Self::MigrationRequired];

    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::MigrationRequired => "migration-required",
        }
    }
}

/// Closed state persistence classes used for metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PersistenceLabel {
    /// Durable state.
    Persistent,
    /// Restart-scoped state.
    Ephemeral,
    /// Rebuildable cached state.
    Cache,
    /// Configuration state.
    Config,
}

impl PersistenceLabel {
    /// Every allowed persistence class.
    pub const ALL: [Self; 4] = [Self::Persistent, Self::Ephemeral, Self::Cache, Self::Config];

    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::Ephemeral => "ephemeral",
            Self::Cache => "cache",
            Self::Config => "config",
        }
    }
}

/// Closed Volume source classes used for metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKindLabel {
    /// Host-backed anchored directory.
    LocalPath,
    /// Host-backed block image.
    BlockImage,
    /// Memory-backed filesystem.
    Tmpfs,
}

impl SourceKindLabel {
    /// Every allowed source class.
    pub const ALL: [Self; 3] = [Self::LocalPath, Self::BlockImage, Self::Tmpfs];

    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalPath => "local-path",
            Self::BlockImage => "block-image",
            Self::Tmpfs => "tmpfs",
        }
    }
}

/// Closed operation classes used for metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationLabel {
    /// Initial provisioning.
    Provision,
    /// Layout repair.
    LayoutRepair,
    /// Schema migration.
    Migration,
    /// Snapshot creation.
    Snapshot,
    /// Marker verification.
    MarkerCheck,
    /// Store-view synchronization.
    StoreSync,
    /// Volume relocation.
    Relocation,
    /// Sealing-key rotation.
    SealingRotation,
    /// Unclaimed Volume garbage collection.
    UnclaimedGc,
    /// Descriptor handoff.
    FdHandoff,
}

impl OperationLabel {
    /// Every allowed operation class.
    pub const ALL: [Self; 10] = [
        Self::Provision,
        Self::LayoutRepair,
        Self::Migration,
        Self::Snapshot,
        Self::MarkerCheck,
        Self::StoreSync,
        Self::Relocation,
        Self::SealingRotation,
        Self::UnclaimedGc,
        Self::FdHandoff,
    ];

    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provision => "provision",
            Self::LayoutRepair => "layout-repair",
            Self::Migration => "migration",
            Self::Snapshot => "snapshot",
            Self::MarkerCheck => "marker-check",
            Self::StoreSync => "store-sync",
            Self::Relocation => "relocation",
            Self::SealingRotation => "sealing-rotation",
            Self::UnclaimedGc => "unclaimed-gc",
            Self::FdHandoff => "fd-handoff",
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
    /// Every allowed Provider value.
    pub const ALL: [Self; 1] = [Self::VolumeLocal];

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
    /// Every allowed schema class.
    pub const ALL: [Self; 3] = [Self::ProviderState, Self::StoreView, Self::SwtpmState];

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
    /// The operation is eligible for a bounded retry.
    Retryable,
    /// An idempotent transition recovered its committed result.
    Recovered,
}

impl MetricOutcome {
    /// Every allowed metric outcome.
    pub const ALL: [Self; 7] = [
        Self::Succeeded,
        Self::Failed,
        Self::Verified,
        Self::Missing,
        Self::Replaced,
        Self::Retryable,
        Self::Recovered,
    ];

    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Verified => "verified",
            Self::Missing => "missing",
            Self::Replaced => "replaced",
            Self::Retryable => "retryable",
            Self::Recovered => "recovered",
        }
    }
}

/// Closed view scope values that cannot expose configured view names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricView {
    /// A view of the Volume root.
    Root,
    /// A view of a declared subtree.
    Subtree,
}

impl MetricView {
    /// Every allowed view scope.
    pub const ALL: [Self; 2] = [Self::Root, Self::Subtree];

    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Subtree => "subtree",
        }
    }
}

/// Closed attachment access values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricAccess {
    /// Read-only access.
    ReadOnly,
    /// Single-writer access.
    ReadWrite,
    /// Explicit shared-writer access.
    SharedWrite,
}

impl MetricAccess {
    /// Every allowed attachment access class.
    pub const ALL: [Self; 3] = [Self::ReadOnly, Self::ReadWrite, Self::SharedWrite];

    /// Return the stable label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
            Self::SharedWrite => "shared-write",
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
    /// Every allowed snapshot trigger.
    pub const ALL: [Self; 3] = [Self::Manual, Self::PreMigration, Self::PreRelocation];

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
    /// Closed schema version class.
    SchemaVersion(SchemaVersionLabel),
    /// Closed persistence class.
    Persistence(PersistenceLabel),
    /// Closed source class.
    SourceKind(SourceKindLabel),
    /// Closed operation class.
    Operation(OperationLabel),
    /// Closed outcome.
    Outcome(MetricOutcome),
    /// Closed snapshot trigger.
    Trigger(MetricTrigger),
    /// Closed view scope.
    View(MetricView),
    /// Closed attachment access.
    Access(MetricAccess),
}

impl MetricLabelValue {
    /// Return the key paired with this value.
    pub const fn key(self) -> MetricLabelKey {
        match self {
            Self::Provider(_) => MetricLabelKey::Provider,
            Self::Schema(_) => MetricLabelKey::SchemaId,
            Self::SchemaVersion(_) => MetricLabelKey::SchemaVersion,
            Self::Persistence(_) => MetricLabelKey::PersistenceClass,
            Self::SourceKind(_) => MetricLabelKey::SourceKind,
            Self::Operation(_) => MetricLabelKey::Operation,
            Self::Outcome(_) => MetricLabelKey::Outcome,
            Self::Trigger(_) => MetricLabelKey::Trigger,
            Self::View(_) => MetricLabelKey::View,
            Self::Access(_) => MetricLabelKey::Access,
        }
    }

    /// Return the stable closed value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider(value) => value.as_str(),
            Self::Schema(value) => value.as_str(),
            Self::SchemaVersion(value) => value.as_str(),
            Self::Persistence(value) => value.as_str(),
            Self::SourceKind(value) => value.as_str(),
            Self::Operation(value) => value.as_str(),
            Self::Outcome(value) => value.as_str(),
            Self::Trigger(value) => value.as_str(),
            Self::View(value) => value.as_str(),
            Self::Access(value) => value.as_str(),
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
const PROVIDER_PERSISTENCE_SOURCE_OUTCOME: &[MetricLabelKey] = &[
    MetricLabelKey::Provider,
    MetricLabelKey::PersistenceClass,
    MetricLabelKey::SourceKind,
    MetricLabelKey::Outcome,
];
const PROVIDER_SOURCE: &[MetricLabelKey] = &[MetricLabelKey::Provider, MetricLabelKey::SourceKind];
const PROVIDER_PERSISTENCE: &[MetricLabelKey] =
    &[MetricLabelKey::Provider, MetricLabelKey::PersistenceClass];
const PROVIDER_VIEW_ACCESS_OUTCOME: &[MetricLabelKey] = &[
    MetricLabelKey::Provider,
    MetricLabelKey::View,
    MetricLabelKey::Access,
    MetricLabelKey::Outcome,
];

/// Every Volume-state metric definition.
pub const METRICS: [MetricDescriptor; 15] = [
    MetricDescriptor {
        name: "d2b_volume_provision_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_PERSISTENCE_SOURCE_OUTCOME,
    },
    MetricDescriptor {
        name: "d2b_volume_provision_duration_ms",
        kind: MetricKind::Histogram,
        unit: MetricUnit::Milliseconds,
        labels: PROVIDER_SOURCE,
    },
    MetricDescriptor {
        name: "d2b_volume_layout_repair_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_OUTCOME,
    },
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
    MetricDescriptor {
        name: "d2b_volume_store_sync_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_OUTCOME,
    },
    MetricDescriptor {
        name: "d2b_volume_store_sync_duration_ms",
        kind: MetricKind::Histogram,
        unit: MetricUnit::Milliseconds,
        labels: PROVIDER,
    },
    MetricDescriptor {
        name: "d2b_volume_relocation_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_OUTCOME,
    },
    MetricDescriptor {
        name: "d2b_volume_sealing_rotation_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_OUTCOME,
    },
    MetricDescriptor {
        name: "d2b_volume_unclaimed_gc_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_PERSISTENCE,
    },
    MetricDescriptor {
        name: "d2b_volume_fd_handoff_total",
        kind: MetricKind::Counter,
        unit: MetricUnit::Count,
        labels: PROVIDER_VIEW_ACCESS_OUTCOME,
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
