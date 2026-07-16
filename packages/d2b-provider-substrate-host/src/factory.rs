use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::ProviderId,
    v2_provider::{
        ImplementationId, MAX_PROVIDER_REGISTRY_ENTRIES, ProviderDescriptor, ProviderFactoryKey,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};

use crate::{
    HostSubstrateConfiguration, HostSubstrateKind, HostSubstratePort, LinuxSubstrateProvider,
    NixOsSubstrateProvider, provider::validate_host_descriptor,
};

pub const MAX_HOST_SUBSTRATE_FACTORY_PROVIDERS: usize = MAX_PROVIDER_REGISTRY_ENTRIES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSubstrateFactoryError {
    EmptyEntries,
    ProviderLimit,
    DuplicateProvider,
    ConfigurationMismatch,
    DescriptorInvalid,
}

impl fmt::Display for HostSubstrateFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyEntries => "host substrate factory has no provider entries",
            Self::ProviderLimit => "host substrate factory provider limit exceeded",
            Self::DuplicateProvider => "host substrate factory has a duplicate provider",
            Self::ConfigurationMismatch => {
                "host substrate factory entry configuration does not match its implementation"
            }
            Self::DescriptorInvalid => "host substrate factory descriptor is invalid",
        })
    }
}

impl Error for HostSubstrateFactoryError {}

/// One exact registry descriptor and its injected semantic host port.
///
/// The descriptor binds the provider ID, generation, configuration and scope
/// fingerprints, and trusted local-root realm placement before construction.
#[derive(Clone)]
pub struct HostSubstrateFactoryEntry {
    descriptor: ProviderDescriptor,
    configuration: HostSubstrateConfiguration,
    port: Arc<dyn HostSubstratePort>,
}

impl fmt::Debug for HostSubstrateFactoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostSubstrateFactoryEntry")
            .field("descriptor", &self.descriptor)
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl HostSubstrateFactoryEntry {
    pub fn nixos(
        descriptor: ProviderDescriptor,
        port: Arc<dyn HostSubstratePort>,
    ) -> Result<Self, HostSubstrateFactoryError> {
        Self::new(descriptor, HostSubstrateConfiguration::nixos(), port)
    }

    pub fn linux(
        descriptor: ProviderDescriptor,
        port: Arc<dyn HostSubstratePort>,
    ) -> Result<Self, HostSubstrateFactoryError> {
        Self::new(
            descriptor,
            HostSubstrateConfiguration::generic_linux(),
            port,
        )
    }

    pub fn new(
        descriptor: ProviderDescriptor,
        configuration: HostSubstrateConfiguration,
        port: Arc<dyn HostSubstratePort>,
    ) -> Result<Self, HostSubstrateFactoryError> {
        validate_host_descriptor(&descriptor, configuration)
            .map_err(|_| HostSubstrateFactoryError::DescriptorInvalid)?;
        Ok(Self {
            descriptor,
            configuration,
            port,
        })
    }

    pub fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    pub const fn configuration(&self) -> HostSubstrateConfiguration {
        self.configuration
    }

    fn accepts(&self, descriptor: &ProviderDescriptor) -> bool {
        self.descriptor.provider_id == descriptor.provider_id
            && self.descriptor.configuration_schema_fingerprint
                == descriptor.configuration_schema_fingerprint
            && self.descriptor.configured_scope_digest == descriptor.configured_scope_digest
            && self.descriptor.placement == descriptor.placement
            && self.descriptor.placement.realm_id() == descriptor.placement.realm_id()
            && self.descriptor == *descriptor
    }
}

#[derive(Clone)]
pub struct HostSubstrateProviderFactory {
    kind: HostSubstrateKind,
    key: ProviderFactoryKey,
    entries: BTreeMap<ProviderId, HostSubstrateFactoryEntry>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for HostSubstrateProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostSubstrateProviderFactory")
            .field("kind", &self.kind)
            .field("key", &self.key)
            .field("provider_count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl HostSubstrateProviderFactory {
    pub fn nixos(
        entries: impl IntoIterator<Item = HostSubstrateFactoryEntry>,
    ) -> Result<Self, HostSubstrateFactoryError> {
        Self::new(HostSubstrateKind::NixOs, entries)
    }

    pub fn linux(
        entries: impl IntoIterator<Item = HostSubstrateFactoryEntry>,
    ) -> Result<Self, HostSubstrateFactoryError> {
        Self::new(HostSubstrateKind::GenericLinux, entries)
    }

    pub fn new(
        kind: HostSubstrateKind,
        entries: impl IntoIterator<Item = HostSubstrateFactoryEntry>,
    ) -> Result<Self, HostSubstrateFactoryError> {
        Self::with_clock(kind, entries, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        kind: HostSubstrateKind,
        entries: impl IntoIterator<Item = HostSubstrateFactoryEntry>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, HostSubstrateFactoryError> {
        let mut providers = BTreeMap::new();
        for entry in entries {
            if entry.configuration.substrate() != kind {
                return Err(HostSubstrateFactoryError::ConfigurationMismatch);
            }
            let provider_id = entry.descriptor.provider_id.clone();
            if providers.contains_key(&provider_id) {
                return Err(HostSubstrateFactoryError::DuplicateProvider);
            }
            if providers.len() >= MAX_HOST_SUBSTRATE_FACTORY_PROVIDERS {
                return Err(HostSubstrateFactoryError::ProviderLimit);
            }
            providers.insert(provider_id, entry);
        }
        if providers.is_empty() {
            return Err(HostSubstrateFactoryError::EmptyEntries);
        }
        let key = kind
            .factory_key()
            .map_err(|_| HostSubstrateFactoryError::DescriptorInvalid)?;
        Ok(Self {
            kind,
            key,
            entries: providers,
            clock,
        })
    }

    pub const fn kind(&self) -> HostSubstrateKind {
        self.kind
    }

    pub fn key(&self) -> ProviderFactoryKey {
        self.key.clone()
    }

    pub fn implementation_id(&self) -> &ImplementationId {
        &self.key.implementation_id
    }

    pub fn registered_provider_count(&self) -> usize {
        self.entries.len()
    }
}

impl ProviderFactory for HostSubstrateProviderFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != self.key.provider_type
            || descriptor.implementation_id != self.key.implementation_id
        {
            return Err(FactoryError::Rejected);
        }
        let entry = self
            .entries
            .get(&descriptor.provider_id)
            .filter(|entry| entry.accepts(descriptor))
            .ok_or(FactoryError::Rejected)?;

        match self.kind {
            HostSubstrateKind::NixOs => NixOsSubstrateProvider::with_clock(
                descriptor.clone(),
                entry.port.clone(),
                self.clock.clone(),
            )
            .map(|provider| ProviderInstance::Substrate(Arc::new(provider)))
            .map_err(|_| FactoryError::Rejected),
            HostSubstrateKind::GenericLinux => LinuxSubstrateProvider::with_clock(
                descriptor.clone(),
                entry.port.clone(),
                self.clock.clone(),
            )
            .map(|provider| ProviderInstance::Substrate(Arc::new(provider)))
            .map_err(|_| FactoryError::Rejected),
        }
    }
}
