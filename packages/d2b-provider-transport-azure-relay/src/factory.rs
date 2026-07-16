use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        Fingerprint, ImplementationId, MAX_PROVIDER_REGISTRY_ENTRIES, ProviderDescriptor,
        ProviderFactoryKey,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};

use crate::{
    AZURE_RELAY_IMPLEMENTATION_ID, AzureRelayConfiguration, AzureRelayTransportProvider,
    RelayControlPort,
};

/// Return the canonical typed implementation identifier.
pub fn azure_relay_implementation_id() -> ImplementationId {
    ImplementationId::parse(AZURE_RELAY_IMPLEMENTATION_ID)
        .unwrap_or_else(|_| unreachable!("the static Azure Relay implementation ID is valid"))
}

/// Return the canonical registry key for the Azure Relay implementation.
pub fn azure_relay_factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Transport,
        implementation_id: azure_relay_implementation_id(),
    }
}

/// Failures while binding configured instances to one factory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureRelayFactoryBuildError {
    EmptyConfigurations,
    TooManyConfigurations,
    DuplicateProvider,
}

impl fmt::Display for AzureRelayFactoryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyConfigurations => "Azure Relay factory has no configured providers",
            Self::TooManyConfigurations => {
                "Azure Relay factory exceeds the configured provider bound"
            }
            Self::DuplicateProvider => "Azure Relay factory has a duplicate provider",
        })
    }
}

impl Error for AzureRelayFactoryBuildError {}

/// One configured instance and the exact descriptor digests it was built for.
#[derive(Clone)]
pub struct AzureRelayFactoryEntry {
    provider_id: ProviderId,
    configuration_schema_fingerprint: Fingerprint,
    configured_scope_digest: Fingerprint,
    configuration: AzureRelayConfiguration,
}

impl AzureRelayFactoryEntry {
    pub fn new(
        provider_id: ProviderId,
        configuration_schema_fingerprint: Fingerprint,
        configured_scope_digest: Fingerprint,
        configuration: AzureRelayConfiguration,
    ) -> Self {
        Self {
            provider_id,
            configuration_schema_fingerprint,
            configured_scope_digest,
            configuration,
        }
    }

    pub fn for_descriptor(
        descriptor: &ProviderDescriptor,
        configuration: AzureRelayConfiguration,
    ) -> Self {
        Self::new(
            descriptor.provider_id.clone(),
            descriptor.configuration_schema_fingerprint.clone(),
            descriptor.configured_scope_digest.clone(),
            configuration,
        )
    }
}

impl fmt::Debug for AzureRelayFactoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureRelayFactoryEntry")
            .field("provider_id", &"<redacted>")
            .field("configuration_digests", &"<redacted>")
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

/// Registry factory for all configured Azure Relay instances in one agent.
///
/// The shared semantic port remains co-located with the credential owner. Each
/// constructed instance receives only its agent-local opaque configuration.
pub struct AzureRelayProviderFactory {
    configurations: BTreeMap<ProviderId, AzureRelayFactoryEntry>,
    port: Arc<dyn RelayControlPort>,
    clock: Arc<dyn ProviderClock>,
}

impl AzureRelayProviderFactory {
    pub fn new(
        port: Arc<dyn RelayControlPort>,
        configurations: impl IntoIterator<Item = AzureRelayFactoryEntry>,
    ) -> Result<Self, AzureRelayFactoryBuildError> {
        Self::with_clock(port, configurations, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        port: Arc<dyn RelayControlPort>,
        configurations: impl IntoIterator<Item = AzureRelayFactoryEntry>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, AzureRelayFactoryBuildError> {
        let mut by_provider = BTreeMap::new();
        for entry in configurations {
            if by_provider
                .insert(entry.provider_id.clone(), entry)
                .is_some()
            {
                return Err(AzureRelayFactoryBuildError::DuplicateProvider);
            }
            if by_provider.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
                return Err(AzureRelayFactoryBuildError::TooManyConfigurations);
            }
        }
        if by_provider.is_empty() {
            return Err(AzureRelayFactoryBuildError::EmptyConfigurations);
        }
        Ok(Self {
            configurations: by_provider,
            port,
            clock,
        })
    }

    pub fn key() -> ProviderFactoryKey {
        azure_relay_factory_key()
    }

    pub fn configuration_count(&self) -> usize {
        self.configurations.len()
    }
}

impl fmt::Debug for AzureRelayProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureRelayProviderFactory")
            .field("provider_type", &ProviderType::Transport)
            .field("implementation_id", &AZURE_RELAY_IMPLEMENTATION_ID)
            .field("configuration_count", &self.configurations.len())
            .finish_non_exhaustive()
    }
}

impl ProviderFactory for AzureRelayProviderFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Transport
            || descriptor.implementation_id != azure_relay_implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let entry = self
            .configurations
            .get(&descriptor.provider_id)
            .ok_or(FactoryError::Rejected)?;
        if entry.configuration_schema_fingerprint != descriptor.configuration_schema_fingerprint
            || entry.configured_scope_digest != descriptor.configured_scope_digest
        {
            return Err(FactoryError::Rejected);
        }
        let provider = AzureRelayTransportProvider::with_clock(
            descriptor.clone(),
            entry.configuration.clone(),
            Arc::clone(&self.port),
            Arc::clone(&self.clock),
        )
        .map_err(|_| FactoryError::Rejected)?;
        Ok(ProviderInstance::Transport(Arc::new(provider)))
    }
}
