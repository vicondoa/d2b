//! Post-commit generation transition planning.
//!
//! The planner is intentionally store-agnostic. It accepts an integrity-checked
//! bundle and a snapshot of persisted metadata, then returns typed mutations
//! for the eventual durable store adapter. No production store is wired here.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::{
    ZoneBundle,
    v3::{ConfigurationGeneration, ResourceBundleGenerationId, SchemaFingerprint, Timestamp},
};

use crate::resource_store::{
    ManagedBy, PersistedResourceMetadata, PersistedResourceRecord, ResourceMetadataError,
};

use super::{ConfigurationService, ResourceKey};

/// Closed generation-transition planning failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationTransitionError {
    /// No newly committed generation is awaiting post-commit effects.
    GenerationNotCommitted,
    /// Store-owned metadata could not be constructed.
    ResourceMetadata(ResourceMetadataError),
    /// The bundle's Provider schema projection did not match installed artifacts.
    ProviderSchemaDigestMismatch,
    /// The proof was issued for a different committed bundle.
    CommittedBundleMismatch,
}

impl GenerationTransitionError {
    /// Return the stable failure label without caller-supplied data.
    pub const fn label(self) -> &'static str {
        match self {
            Self::GenerationNotCommitted => "generation-transition-not-committed",
            Self::ResourceMetadata(error) => error.label(),
            Self::ProviderSchemaDigestMismatch => "bundle-provider-schema-digest-mismatch",
            Self::CommittedBundleMismatch => "generation-transition-bundle-mismatch",
        }
    }
}

impl core::fmt::Display for GenerationTransitionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for GenerationTransitionError {}

impl From<ResourceMetadataError> for GenerationTransitionError {
    fn from(error: ResourceMetadataError) -> Self {
        Self::ResourceMetadata(error)
    }
}

/// Evidence that the active generation record committed before this pass.
#[derive(PartialEq, Eq)]
pub struct CommittedConfigurationGeneration {
    ordinal: ConfigurationGeneration,
    content_hash: ResourceBundleGenerationId,
}

impl core::fmt::Debug for CommittedConfigurationGeneration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CommittedConfigurationGeneration")
            .field("ordinal", &self.ordinal.get())
            .finish_non_exhaustive()
    }
}

impl CommittedConfigurationGeneration {
    /// Return the committed monotone activation ordinal.
    pub const fn ordinal(&self) -> ConfigurationGeneration {
        self.ordinal
    }
}

/// Bind a transition to the generation whose post-commit effects are pending.
///
/// The configuration service exposes no pending effects until its durable
/// commit operation returns. This function is therefore unavailable for an
/// uncommitted or already-released pass.
pub fn committed_configuration_generation(
    service: &ConfigurationService,
) -> Result<CommittedConfigurationGeneration, GenerationTransitionError> {
    if service.pending_effects.is_none() {
        return Err(GenerationTransitionError::GenerationNotCommitted);
    }
    let record = service
        .record
        .as_ref()
        .ok_or(GenerationTransitionError::GenerationNotCommitted)?;
    Ok(CommittedConfigurationGeneration {
        ordinal: record.active_ordinal,
        content_hash: record.active_generation_id.clone(),
    })
}

/// One desired resource and the metadata core must persist for it.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceUpsert {
    key: ResourceKey,
    metadata: PersistedResourceMetadata,
}

impl ResourceUpsert {
    /// Borrow the desired resource identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Borrow the core-assigned persisted metadata.
    pub const fn metadata(&self) -> &PersistedResourceMetadata {
        &self.metadata
    }
}

impl core::fmt::Debug for ResourceUpsert {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceUpsert")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

/// A desired item skipped because a non-configuration owner holds its name.
#[derive(Clone, PartialEq, Eq)]
pub struct NameConflict {
    key: ResourceKey,
}

impl NameConflict {
    /// Borrow the skipped desired resource identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Return the stable per-item status projection.
    pub const fn condition(&self) -> &'static str {
        "Degraded/name-conflict"
    }
}

impl core::fmt::Debug for NameConflict {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NameConflict(<redacted>)")
    }
}

/// One configuration-owned resource omitted by the new bundle.
#[derive(Clone, PartialEq, Eq)]
pub struct RemovalRequest {
    key: ResourceKey,
    metadata: PersistedResourceMetadata,
    newly_scheduled: bool,
}

impl RemovalRequest {
    /// Borrow the resource identity passed to asynchronous Delete.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Borrow metadata carrying the deletion request timestamp.
    pub const fn metadata(&self) -> &PersistedResourceMetadata {
        &self.metadata
    }

    /// Return whether this transition first stamped the deletion request.
    pub const fn newly_scheduled(&self) -> bool {
        self.newly_scheduled
    }
}

impl core::fmt::Debug for RemovalRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemovalRequest")
            .field("metadata", &self.metadata)
            .field("newly_scheduled", &self.newly_scheduled)
            .finish_non_exhaustive()
    }
}

/// Redacted authorized payload for one activated bundle audit event.
#[derive(Clone, PartialEq, Eq)]
pub struct BundleActivatedAudit {
    content_hash: ResourceBundleGenerationId,
    configuration_generation: ConfigurationGeneration,
    resource_count: usize,
    provider_schema_digests: BTreeMap<String, SchemaFingerprint>,
}

impl BundleActivatedAudit {
    /// Borrow the integrity-derived bundle identity.
    pub const fn content_hash(&self) -> &ResourceBundleGenerationId {
        &self.content_hash
    }

    /// Return the committed configuration ordinal.
    pub const fn configuration_generation(&self) -> ConfigurationGeneration {
        self.configuration_generation
    }

    /// Return the number of authored resources without exposing identities.
    pub const fn resource_count(&self) -> usize {
        self.resource_count
    }

    /// Borrow the verified Provider schema digest projection.
    pub const fn provider_schema_digests(&self) -> &BTreeMap<String, SchemaFingerprint> {
        &self.provider_schema_digests
    }
}

impl core::fmt::Debug for BundleActivatedAudit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BundleActivatedAudit")
            .field(
                "configuration_generation",
                &self.configuration_generation.get(),
            )
            .field("resource_count", &self.resource_count)
            .field("provider_schemas", &self.provider_schema_digests.len())
            .finish()
    }
}

/// Closed audit records emitted by a generation transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationTransitionAudit {
    /// The new bundle became active after its durable commit.
    BundleActivated(BundleActivatedAudit),
    /// One desired item was skipped without exposing its identity.
    ResourceConflictSkipped,
    /// One asynchronous resource deletion was newly scheduled.
    ResourceDeletionScheduled,
    /// Bundle application was refused before any resource mutation.
    BundleRejected,
}

impl GenerationTransitionError {
    /// Return the closed rejection audit emitted for a failed bundle check.
    pub const fn rejection_audit(self) -> Option<GenerationTransitionAudit> {
        match self {
            Self::ProviderSchemaDigestMismatch => Some(GenerationTransitionAudit::BundleRejected),
            Self::GenerationNotCommitted
            | Self::ResourceMetadata(_)
            | Self::CommittedBundleMismatch => None,
        }
    }
}

/// Pure post-commit resource-store mutation plan.
#[derive(Clone, PartialEq, Eq)]
pub struct GenerationTransitionPlan {
    upserts: Vec<ResourceUpsert>,
    name_conflicts: Vec<NameConflict>,
    removals: Vec<RemovalRequest>,
    audits: Vec<GenerationTransitionAudit>,
}

impl GenerationTransitionPlan {
    /// Borrow non-conflicting desired resource upserts.
    pub fn upserts(&self) -> &[ResourceUpsert] {
        &self.upserts
    }

    /// Borrow per-item name conflicts.
    pub fn name_conflicts(&self) -> &[NameConflict] {
        &self.name_conflicts
    }

    /// Borrow asynchronous removal requests.
    pub fn removals(&self) -> &[RemovalRequest] {
        &self.removals
    }

    /// Borrow closed audit projections.
    pub fn audits(&self) -> &[GenerationTransitionAudit] {
        &self.audits
    }
}

impl core::fmt::Debug for GenerationTransitionPlan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GenerationTransitionPlan")
            .field("upserts", &self.upserts.len())
            .field("name_conflicts", &self.name_conflicts.len())
            .field("removals", &self.removals.len())
            .field("audits", &self.audits.len())
            .finish()
    }
}

/// Plan the resource metadata projection and absent-resource cleanup set.
///
/// The deletion loop is structurally bounded to `configuration_managed`: rows
/// owned by controllers or the API never enter the iterable deletion set.
pub fn plan_generation_transition(
    bundle: &ZoneBundle,
    committed: CommittedConfigurationGeneration,
    stored: &[PersistedResourceRecord],
    installed_provider_schema_digests: &BTreeMap<String, SchemaFingerprint>,
    now: &Timestamp,
) -> Result<GenerationTransitionPlan, GenerationTransitionError> {
    if bundle.content_hash() != &committed.content_hash {
        return Err(GenerationTransitionError::CommittedBundleMismatch);
    }
    bundle
        .verify_provider_schema_digests(installed_provider_schema_digests)
        .map_err(|_| GenerationTransitionError::ProviderSchemaDigestMismatch)?;
    let declared: Vec<ResourceKey> = bundle
        .resources()
        .iter()
        .map(|resource| {
            ResourceKey::new(
                resource.resource_type().clone(),
                resource.metadata().name().clone(),
            )
        })
        .collect();
    let declared_set: BTreeSet<ResourceKey> = declared.iter().cloned().collect();

    let mut foreign_occupied_names = BTreeSet::new();
    for record in stored {
        if record.metadata().managed_by() != ManagedBy::Configuration {
            foreign_occupied_names.insert(record.key().name());
        }
    }
    let configuration_managed: BTreeMap<_, _> = stored
        .iter()
        .filter(|record| record.metadata().managed_by() == ManagedBy::Configuration)
        .map(|record| (record.key(), record))
        .collect();

    let mut upserts = Vec::new();
    let mut name_conflicts = Vec::new();
    for key in declared {
        let conflicts = foreign_occupied_names.contains(key.name());
        if conflicts {
            name_conflicts.push(NameConflict { key });
        } else {
            upserts.push(ResourceUpsert {
                key,
                metadata: PersistedResourceMetadata::configuration(committed.ordinal.get())?,
            });
        }
    }

    let mut removals = Vec::new();
    for (key, record) in configuration_managed {
        if declared_set.contains(key) {
            continue;
        }
        let mut metadata = record.metadata().clone();
        let newly_scheduled = metadata.schedule_deletion(now);
        removals.push(RemovalRequest {
            key: key.clone(),
            metadata,
            newly_scheduled,
        });
    }

    let mut audits = Vec::with_capacity(1 + name_conflicts.len() + removals.len());
    audits.push(GenerationTransitionAudit::BundleActivated(
        BundleActivatedAudit {
            content_hash: bundle.content_hash().clone(),
            configuration_generation: committed.ordinal,
            resource_count: bundle.resources().len(),
            provider_schema_digests: bundle.provider_schema_digests().clone(),
        },
    ));
    audits.extend(
        name_conflicts
            .iter()
            .map(|_| GenerationTransitionAudit::ResourceConflictSkipped),
    );
    audits.extend(
        removals
            .iter()
            .filter(|removal| removal.newly_scheduled)
            .map(|_| GenerationTransitionAudit::ResourceDeletionScheduled),
    );

    Ok(GenerationTransitionPlan {
        upserts,
        name_conflicts,
        removals,
        audits,
    })
}
