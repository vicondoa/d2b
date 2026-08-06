//! Wire-safe Provider registry entries.
//!
//! The runtime registry in `d2b-provider` owns instances and in-flight
//! permits.  This module contains only the signed, identity-safe publication
//! shape that can cross the v3 Provider service boundary.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceGeneration, ResourceRef, SchemaFingerprint, ServiceName,
    execution_policy::{BoundedText, redacted_debug},
};

/// Maximum Provider registry mappings in one publication.
pub const MAX_PROVIDER_REGISTRY_MAPPINGS: usize = 256;
/// Maximum mapping identifier bytes.
pub const MAX_PROVIDER_MAPPING_ID_BYTES: usize = 63;

/// Closed binding axes used by registry entries.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderBindingAxis {
    Zone,
    Provider,
    Controller,
    Session,
    /// Forward-compatible decode marker.  It is never admissible for a
    /// published mapping and therefore cannot become a permissive fallback.
    Unknown,
}

/// Registry publication validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRegistryError {
    WrongProviderRef,
    ZeroGeneration,
    InvalidFingerprint,
    InvalidMappingId,
    DuplicateMappingId,
    MappingBoundExceeded,
    UnknownAxis,
    AxisMismatch,
}

impl core::fmt::Display for ProviderRegistryError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::WrongProviderRef => "provider-registry-provider-ref-invalid",
            Self::ZeroGeneration => "provider-registry-generation-invalid",
            Self::InvalidFingerprint => "provider-registry-fingerprint-invalid",
            Self::InvalidMappingId => "provider-registry-mapping-id-invalid",
            Self::DuplicateMappingId => "provider-registry-mapping-id-duplicate",
            Self::MappingBoundExceeded => "provider-registry-mapping-bound-exceeded",
            Self::UnknownAxis => "provider-registry-axis-unknown",
            Self::AxisMismatch => "provider-registry-axis-mismatch",
        })
    }
}

impl std::error::Error for ProviderRegistryError {}

/// One exact Provider registry publication entry.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegistryEntry {
    provider_ref: ResourceRef,
    service: ServiceName,
    descriptor_fingerprint: SchemaFingerprint,
    provider_generation: ResourceGeneration,
    axis: ProviderBindingAxis,
    mapping_id: String,
}

impl ProviderRegistryEntry {
    /// Construct an exact entry.
    pub fn new(
        provider_ref: ResourceRef,
        service: ServiceName,
        descriptor_fingerprint: SchemaFingerprint,
        provider_generation: ResourceGeneration,
        axis: ProviderBindingAxis,
        mapping_id: impl Into<String>,
    ) -> Result<Self, ProviderRegistryError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(ProviderRegistryError::WrongProviderRef);
        }
        if provider_generation.get() == 0 {
            return Err(ProviderRegistryError::ZeroGeneration);
        }
        if matches!(axis, ProviderBindingAxis::Unknown) {
            return Err(ProviderRegistryError::UnknownAxis);
        }
        let mapping_id = mapping_id.into();
        if mapping_id.is_empty()
            || mapping_id.len() > MAX_PROVIDER_MAPPING_ID_BYTES
            || BoundedText::parse(mapping_id.clone()).is_err()
        {
            return Err(ProviderRegistryError::InvalidMappingId);
        }
        Ok(Self {
            provider_ref,
            service,
            descriptor_fingerprint,
            provider_generation,
            axis,
            mapping_id,
        })
    }

    /// Borrow Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow service.
    pub const fn service(&self) -> &ServiceName {
        &self.service
    }

    /// Borrow descriptor fingerprint.
    pub const fn descriptor_fingerprint(&self) -> &SchemaFingerprint {
        &self.descriptor_fingerprint
    }

    /// Return Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return binding axis.
    pub const fn axis(&self) -> ProviderBindingAxis {
        self.axis
    }

    /// Borrow stable mapping ID.
    pub fn mapping_id(&self) -> &str {
        &self.mapping_id
    }
}

redacted_debug!(ProviderRegistryEntry);

impl<'de> Deserialize<'de> for ProviderRegistryEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            provider_ref: ResourceRef,
            service: ServiceName,
            descriptor_fingerprint: SchemaFingerprint,
            provider_generation: ResourceGeneration,
            axis: ProviderBindingAxis,
            mapping_id: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.provider_ref,
            wire.service,
            wire.descriptor_fingerprint,
            wire.provider_generation,
            wire.axis,
            wire.mapping_id,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// A complete immutable registry publication.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegistryPublication {
    generation: ResourceGeneration,
    entries: Vec<ProviderRegistryEntry>,
}

impl ProviderRegistryPublication {
    /// Construct a sorted, duplicate-free publication.
    pub fn new(
        generation: ResourceGeneration,
        mut entries: Vec<ProviderRegistryEntry>,
    ) -> Result<Self, ProviderRegistryError> {
        if generation.get() == 0 || entries.len() > MAX_PROVIDER_REGISTRY_MAPPINGS {
            return Err(ProviderRegistryError::MappingBoundExceeded);
        }
        if entries
            .iter()
            .any(|entry| entry.provider_generation != generation)
        {
            return Err(ProviderRegistryError::AxisMismatch);
        }
        entries.sort_by(|left, right| left.mapping_id.cmp(&right.mapping_id));
        if entries
            .windows(2)
            .any(|pair| pair[0].mapping_id == pair[1].mapping_id)
        {
            return Err(ProviderRegistryError::DuplicateMappingId);
        }
        Ok(Self {
            generation,
            entries,
        })
    }

    /// Return publication generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Borrow entries.
    pub fn entries(&self) -> &[ProviderRegistryEntry] {
        &self.entries
    }
}

redacted_debug!(ProviderRegistryPublication);

impl<'de> Deserialize<'de> for ProviderRegistryPublication {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            generation: ResourceGeneration,
            entries: Vec<ProviderRegistryEntry>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.generation, wire.entries).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(generation: u64, id: &str) -> ProviderRegistryEntry {
        ProviderRegistryEntry::new(
            ResourceRef::parse("Provider/system-core").unwrap(),
            ServiceName::parse("d2b.provider.v3").unwrap(),
            SchemaFingerprint::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            ResourceGeneration::new(generation).unwrap(),
            ProviderBindingAxis::Provider,
            id,
        )
        .unwrap()
    }

    #[test]
    fn publication_requires_exact_generation_and_unique_mapping_ids() {
        let generation = ResourceGeneration::new(4).unwrap();
        assert!(
            ProviderRegistryPublication::new(generation, vec![entry(4, "one"), entry(4, "two")])
                .is_ok()
        );
        assert_eq!(
            ProviderRegistryPublication::new(generation, vec![entry(3, "one")]).unwrap_err(),
            ProviderRegistryError::AxisMismatch
        );
        assert_eq!(
            ProviderRegistryPublication::new(generation, vec![entry(4, "one"), entry(4, "one")])
                .unwrap_err(),
            ProviderRegistryError::DuplicateMappingId
        );
    }

    #[test]
    fn unknown_axis_is_not_a_permissive_fallback() {
        assert_eq!(
            ProviderRegistryEntry::new(
                ResourceRef::parse("Provider/system-core").unwrap(),
                ServiceName::parse("d2b.provider.v3").unwrap(),
                SchemaFingerprint::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ProviderBindingAxis::Unknown,
                "one",
            )
            .unwrap_err(),
            ProviderRegistryError::UnknownAxis
        );
    }
}
