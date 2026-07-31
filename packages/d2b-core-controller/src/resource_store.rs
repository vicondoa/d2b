//! Store-owned resource metadata used by configuration generation transitions.
//!
//! Bundle inputs do not carry these fields. Core assigns them only after the
//! active generation has committed, and the resource store persists them with
//! the resource row.

pub use d2b_contracts::v3::ManagedBy;
use d2b_contracts::v3::Timestamp;

use crate::configuration::ResourceKey;

/// Return the stable persisted spelling of the closed lifecycle owner.
pub const fn managed_by_str(value: ManagedBy) -> &'static str {
    match value {
        ManagedBy::Configuration => "configuration",
        ManagedBy::Controller => "controller",
        ManagedBy::Api => "api",
    }
}

/// Return the exact JSON scalar persisted by the resource store.
pub const fn managed_by_json(value: ManagedBy) -> &'static str {
    match value {
        ManagedBy::Configuration => "\"configuration\"",
        ManagedBy::Controller => "\"controller\"",
        ManagedBy::Api => "\"api\"",
    }
}

/// Parse one persisted spelling without accepting an open-ended value.
pub fn parse_managed_by(value: &str) -> Result<ManagedBy, ResourceMetadataError> {
    match value {
        "configuration" => Ok(ManagedBy::Configuration),
        "controller" => Ok(ManagedBy::Controller),
        "api" => Ok(ManagedBy::Api),
        _ => Err(ResourceMetadataError::UnknownManagedBy),
    }
}

/// Parse the exact JSON scalar persisted by the resource store.
pub fn parse_managed_by_json(value: &str) -> Result<ManagedBy, ResourceMetadataError> {
    match value {
        "\"configuration\"" => Ok(ManagedBy::Configuration),
        "\"controller\"" => Ok(ManagedBy::Controller),
        "\"api\"" => Ok(ManagedBy::Api),
        _ => Err(ResourceMetadataError::UnknownManagedBy),
    }
}

/// Closed validation failure for store-owned metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceMetadataError {
    /// A persisted `managedBy` value was outside the closed enum.
    UnknownManagedBy,
    /// Configuration ownership lacked a nonzero generation ordinal.
    ConfigurationGenerationMissing,
    /// A non-configuration owner carried a configuration generation.
    ConfigurationGenerationUnexpected,
}

impl ResourceMetadataError {
    /// Return the stable failure label without caller-supplied data.
    pub const fn label(self) -> &'static str {
        match self {
            Self::UnknownManagedBy => "resource-metadata-managed-by-invalid",
            Self::ConfigurationGenerationMissing => {
                "resource-metadata-configuration-generation-missing"
            }
            Self::ConfigurationGenerationUnexpected => {
                "resource-metadata-configuration-generation-unexpected"
            }
        }
    }
}

impl core::fmt::Display for ResourceMetadataError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for ResourceMetadataError {}

/// The store-owned lifecycle projection persisted with one resource row.
#[derive(Clone, PartialEq, Eq)]
pub struct PersistedResourceMetadata {
    managed_by: ManagedBy,
    configuration_generation: Option<u64>,
    deletion_requested_at: Option<Timestamp>,
}

impl PersistedResourceMetadata {
    /// Validate a metadata projection read from or written to the store.
    pub fn new(
        managed_by: ManagedBy,
        configuration_generation: Option<u64>,
        deletion_requested_at: Option<Timestamp>,
    ) -> Result<Self, ResourceMetadataError> {
        match (managed_by, configuration_generation) {
            (ManagedBy::Configuration, Some(generation)) if generation > 0 => {}
            (ManagedBy::Configuration, _) => {
                return Err(ResourceMetadataError::ConfigurationGenerationMissing);
            }
            (ManagedBy::Controller | ManagedBy::Api, Some(_)) => {
                return Err(ResourceMetadataError::ConfigurationGenerationUnexpected);
            }
            (ManagedBy::Controller | ManagedBy::Api, None) => {}
        }
        Ok(Self {
            managed_by,
            configuration_generation,
            deletion_requested_at,
        })
    }

    /// Build metadata assigned by core after a configuration commit.
    pub fn configuration(configuration_generation: u64) -> Result<Self, ResourceMetadataError> {
        Self::new(
            ManagedBy::Configuration,
            Some(configuration_generation),
            None,
        )
    }

    /// Build metadata for a controller-created resource.
    pub const fn controller() -> Self {
        Self {
            managed_by: ManagedBy::Controller,
            configuration_generation: None,
            deletion_requested_at: None,
        }
    }

    /// Build metadata for a resource created through the resource API.
    pub const fn api() -> Self {
        Self {
            managed_by: ManagedBy::Api,
            configuration_generation: None,
            deletion_requested_at: None,
        }
    }

    /// Return the persisted lifecycle owner.
    pub const fn managed_by(&self) -> ManagedBy {
        self.managed_by
    }

    /// Return the configuration activation ordinal, when applicable.
    pub const fn configuration_generation(&self) -> Option<u64> {
        self.configuration_generation
    }

    /// Borrow the deletion request timestamp, when cleanup is pending.
    pub const fn deletion_requested_at(&self) -> Option<&Timestamp> {
        self.deletion_requested_at.as_ref()
    }

    /// Set the deletion request once, preserving the original timestamp.
    pub fn schedule_deletion(&mut self, now: &Timestamp) -> bool {
        if self.deletion_requested_at.is_some() {
            false
        } else {
            self.deletion_requested_at = Some(now.clone());
            true
        }
    }

    /// Clear a pending deletion when a rollback or later bundle revives it.
    pub fn clear_deletion_request(&mut self) {
        self.deletion_requested_at = None;
    }
}

impl core::fmt::Debug for PersistedResourceMetadata {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PersistedResourceMetadata")
            .field("managed_by", &self.managed_by)
            .field(
                "has_configuration_generation",
                &self.configuration_generation.is_some(),
            )
            .field("pending_cleanup", &self.deletion_requested_at.is_some())
            .finish()
    }
}

/// A resource identity paired with its store-owned lifecycle metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct PersistedResourceRecord {
    key: ResourceKey,
    metadata: PersistedResourceMetadata,
}

impl PersistedResourceRecord {
    /// Pair a resource identity with persisted metadata.
    pub const fn new(key: ResourceKey, metadata: PersistedResourceMetadata) -> Self {
        Self { key, metadata }
    }

    /// Borrow the Zone-local resource identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Borrow the store-owned lifecycle metadata.
    pub const fn metadata(&self) -> &PersistedResourceMetadata {
        &self.metadata
    }
}

impl core::fmt::Debug for PersistedResourceRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PersistedResourceRecord")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}
