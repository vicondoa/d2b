//! The v3 Provider descriptor.

use d2b_contracts::v3::{
    identity::{ConfigurationGeneration, ResourceGeneration, ServiceName},
    resource_ref::ResourceRef,
    zone_routing::ZonePath,
};

use crate::{
    error::RegistryBuildError,
    identity::{
        PROVIDER_RESOURCE_TYPE, PROVIDER_SCHEMA_VERSION, ProviderCapabilitySet, ProviderClass,
        ProviderImplementationId,
    },
};

/// What one installed Provider publishes to its Zone's registry generation.
///
/// The descriptor is derived from the Provider's signed manifest and catalog
/// entry. It names the Provider only by its Zone path and its
/// `Provider/<name>` reference; it carries no package, executable, path,
/// socket, or credential.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    schema_version: u32,
    zone: ZonePath,
    provider_ref: ResourceRef,
    class: ProviderClass,
    implementation_id: ProviderImplementationId,
    registry_generation: ConfigurationGeneration,
    provider_generation: ResourceGeneration,
    service: ServiceName,
    capabilities: ProviderCapabilitySet,
}

impl ProviderDescriptor {
    /// Build a descriptor at the current Provider schema version.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZonePath,
        provider_ref: ResourceRef,
        class: ProviderClass,
        implementation_id: ProviderImplementationId,
        registry_generation: ConfigurationGeneration,
        provider_generation: ResourceGeneration,
        service: ServiceName,
        capabilities: ProviderCapabilitySet,
    ) -> Result<Self, RegistryBuildError> {
        let descriptor = Self {
            schema_version: PROVIDER_SCHEMA_VERSION,
            zone,
            provider_ref,
            class,
            implementation_id,
            registry_generation,
            provider_generation,
            service,
            capabilities,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Re-check every descriptor invariant.
    pub fn validate(&self) -> Result<(), RegistryBuildError> {
        if self.schema_version != PROVIDER_SCHEMA_VERSION {
            return Err(RegistryBuildError::UnsupportedSchemaVersion);
        }
        if self.provider_ref.resource_type().as_str() != PROVIDER_RESOURCE_TYPE {
            return Err(RegistryBuildError::NotAProviderRef);
        }
        if self.capabilities.is_empty() {
            return Err(RegistryBuildError::InvalidDescriptor);
        }
        Ok(())
    }

    /// The published schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The Zone this Provider is installed in.
    pub const fn zone(&self) -> &ZonePath {
        &self.zone
    }

    /// The `Provider/<name>` reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// The Provider family.
    pub const fn class(&self) -> ProviderClass {
        self.class
    }

    /// The signed implementation selector.
    pub const fn implementation_id(&self) -> &ProviderImplementationId {
        &self.implementation_id
    }

    /// The registry generation this descriptor was published into.
    pub const fn registry_generation(&self) -> ConfigurationGeneration {
        self.registry_generation
    }

    /// The Provider resource generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// The service the published methods belong to.
    pub const fn service(&self) -> &ServiceName {
        &self.service
    }

    /// The published capability set.
    pub const fn capabilities(&self) -> &ProviderCapabilitySet {
        &self.capabilities
    }
}

impl std::fmt::Debug for ProviderDescriptor {
    /// The Zone path and the exact service are routing detail, so the
    /// descriptor renders only its family, generations, and capability count.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderDescriptor")
            .field("schema_version", &self.schema_version)
            .field("class", &self.class)
            .field("registry_generation", &self.registry_generation)
            .field("provider_generation", &self.provider_generation)
            .field("capability_count", &self.capabilities.len())
            .finish_non_exhaustive()
    }
}
