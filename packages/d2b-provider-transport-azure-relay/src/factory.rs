use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        ImplementationId, MAX_PROVIDER_REGISTRY_ENTRIES, ProviderDescriptor, ProviderFactoryKey,
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

/// Registry factory for all configured Azure Relay instances in one agent.
///
/// The shared semantic port remains co-located with the credential owner. Each
/// constructed instance receives only its agent-local opaque configuration.
pub struct AzureRelayProviderFactory {
    configurations: BTreeMap<ProviderId, AzureRelayConfiguration>,
    port: Arc<dyn RelayControlPort>,
    clock: Arc<dyn ProviderClock>,
}

impl AzureRelayProviderFactory {
    pub fn new(
        port: Arc<dyn RelayControlPort>,
        configurations: impl IntoIterator<Item = (ProviderId, AzureRelayConfiguration)>,
    ) -> Result<Self, AzureRelayFactoryBuildError> {
        Self::with_clock(port, configurations, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        port: Arc<dyn RelayControlPort>,
        configurations: impl IntoIterator<Item = (ProviderId, AzureRelayConfiguration)>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, AzureRelayFactoryBuildError> {
        let mut by_provider = BTreeMap::new();
        for (provider_id, configuration) in configurations {
            if by_provider.insert(provider_id, configuration).is_some() {
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
        let configuration = self
            .configurations
            .get(&descriptor.provider_id)
            .cloned()
            .ok_or(FactoryError::Rejected)?;
        let provider = AzureRelayTransportProvider::with_clock(
            descriptor.clone(),
            configuration,
            Arc::clone(&self.port),
            Arc::clone(&self.clock),
        )
        .map_err(|_| FactoryError::Rejected)?;
        Ok(ProviderInstance::Transport(Arc::new(provider)))
    }
}
