//! v3 Provider composition and bundle-version gates.
//!
//! The daemon consumes this module after the configuration publication path
//! has resolved the private Provider catalog. It owns only typed composition
//! metadata; package paths, executable paths, and host effects remain behind
//! the broker and the signed artifact catalog.

use std::collections::BTreeMap;

use d2b_contracts::v3::{
    ResourceRef,
    identity::{ResourceName, ResourceTypeName, ZoneId},
};

/// Version of the v3 Provider bundle artifact.
pub const PROVIDER_BUNDLE_VERSION: u32 = 3;

/// Schema identity of the v3 Provider bundle artifact.
pub const PROVIDER_BUNDLE_SCHEMA_VERSION: &str = "v3";

/// Maximum Provider entries in one admitted registry snapshot.
pub const MAX_PROVIDER_REGISTRY_ENTRIES: usize = 256;

/// Closed Provider composition failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCompositionError {
    /// The bundle version is not the v3 version.
    BundleVersionMismatch,
    /// The schema identity is not the v3 identity.
    BundleSchemaMismatch,
    /// A Provider reference was not a Provider resource.
    ProviderRefInvalid,
    /// A Provider reference belongs to another Zone.
    ProviderZoneMismatch,
    /// A Provider name was repeated in one snapshot.
    DuplicateProvider,
    /// The registry bound more than its fixed entry limit.
    RegistryBoundExceeded,
    /// A lifecycle operation was requested for an unregistered Provider.
    ProviderNotRegistered,
}

impl ProviderCompositionError {
    /// Stable identity-free failure code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BundleVersionMismatch => "provider-bundle-version-mismatch",
            Self::BundleSchemaMismatch => "provider-bundle-schema-mismatch",
            Self::ProviderRefInvalid => "provider-ref-invalid",
            Self::ProviderZoneMismatch => "provider-zone-mismatch",
            Self::DuplicateProvider => "provider-duplicate",
            Self::RegistryBoundExceeded => "provider-registry-bound-exceeded",
            Self::ProviderNotRegistered => "provider-not-registered",
        }
    }
}

impl core::fmt::Display for ProviderCompositionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProviderCompositionError {}

/// One Provider binding in an admitted Zone snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBinding {
    zone: ZoneId,
    resource: ResourceRef,
    artifact_id: ResourceName,
    schema_fingerprint: String,
}

impl ProviderBinding {
    /// Construct and validate a Zone-local Provider binding.
    pub fn new(
        zone: ZoneId,
        resource: ResourceRef,
        artifact_id: ResourceName,
        schema_fingerprint: impl Into<String>,
    ) -> Result<Self, ProviderCompositionError> {
        if resource.resource_type().as_str() != "Provider" {
            return Err(ProviderCompositionError::ProviderRefInvalid);
        }
        let schema_fingerprint = schema_fingerprint.into();
        if schema_fingerprint.is_empty() || schema_fingerprint.len() > 128 {
            return Err(ProviderCompositionError::BundleSchemaMismatch);
        }
        Ok(Self {
            zone,
            resource,
            artifact_id,
            schema_fingerprint,
        })
    }

    /// Borrow the binding Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the Provider ResourceRef.
    pub const fn resource(&self) -> &ResourceRef {
        &self.resource
    }

    /// Borrow the selected artifact ID.
    pub const fn artifact_id(&self) -> &ResourceName {
        &self.artifact_id
    }

    /// Borrow the signed schema fingerprint.
    pub fn schema_fingerprint(&self) -> &str {
        &self.schema_fingerprint
    }
}

/// An admitted, immutable Provider registry snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRegistrySnapshot {
    zone: ZoneId,
    generation: u64,
    bindings: BTreeMap<ResourceName, ProviderBinding>,
}

impl ProviderRegistrySnapshot {
    /// Compose one v3 snapshot from Zone-local bindings.
    pub fn compose(
        zone: ZoneId,
        generation: u64,
        bindings: impl IntoIterator<Item = ProviderBinding>,
    ) -> Result<Self, ProviderCompositionError> {
        if generation == 0 {
            return Err(ProviderCompositionError::BundleVersionMismatch);
        }
        let mut by_name = BTreeMap::new();
        for binding in bindings {
            if binding.zone() != &zone {
                return Err(ProviderCompositionError::ProviderZoneMismatch);
            }
            let name = binding.resource().name().clone();
            if by_name.insert(name, binding).is_some() {
                return Err(ProviderCompositionError::DuplicateProvider);
            }
            if by_name.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
                return Err(ProviderCompositionError::RegistryBoundExceeded);
            }
        }
        Ok(Self {
            zone,
            generation,
            bindings: by_name,
        })
    }

    /// Borrow the Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the monotone registry generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Resolve a Provider resource by ResourceRef.
    pub fn resolve(
        &self,
        resource: &ResourceRef,
    ) -> Result<&ProviderBinding, ProviderCompositionError> {
        if resource.resource_type().as_str() != "Provider" {
            return Err(ProviderCompositionError::ProviderRefInvalid);
        }
        self.bindings
            .get(resource.name())
            .ok_or(ProviderCompositionError::ProviderNotRegistered)
    }

    /// Number of admitted Providers.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether the snapshot has no Provider entries.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Validate the v3 bundle version and schema before composition.
pub const fn validate_provider_bundle_version(
    version: u32,
    schema: &str,
) -> Result<(), ProviderCompositionError> {
    if version != PROVIDER_BUNDLE_VERSION {
        return Err(ProviderCompositionError::BundleVersionMismatch);
    }
    if schema != PROVIDER_BUNDLE_SCHEMA_VERSION {
        return Err(ProviderCompositionError::BundleSchemaMismatch);
    }
    Ok(())
}

/// Keep the ResourceType identity available to callers without accepting a
/// free-form type alias.
pub fn provider_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse("Provider").expect("Provider is in the v3 catalog")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> ZoneId {
        ZoneId::parse("work").unwrap()
    }

    fn binding(name: &str) -> ProviderBinding {
        ProviderBinding::new(
            zone(),
            ResourceRef::parse(&format!("Provider/{name}")).unwrap(),
            ResourceName::parse(name).unwrap(),
            "sha256:provider",
        )
        .unwrap()
    }

    #[test]
    fn v3_bundle_gate_and_zone_binding_are_closed() {
        validate_provider_bundle_version(3, "v3").unwrap();
        assert!(validate_provider_bundle_version(2, "v2").is_err());
        let snapshot =
            ProviderRegistrySnapshot::compose(zone(), 1, [binding("system-core")]).unwrap();
        assert_eq!(snapshot.generation(), 1);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(
            snapshot
                .resolve(&ResourceRef::parse("Provider/system-core").unwrap())
                .unwrap()
                .artifact_id()
                .as_str(),
            "system-core"
        );
    }

    #[test]
    fn non_provider_resource_ref_cannot_enter_registry() {
        let error = ProviderBinding::new(
            zone(),
            ResourceRef::parse("Guest/workstation").unwrap(),
            ResourceName::parse("guest").unwrap(),
            "sha256:provider",
        )
        .unwrap_err();
        assert_eq!(error, ProviderCompositionError::ProviderRefInvalid);
    }
}
