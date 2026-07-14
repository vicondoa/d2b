use std::{fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::ProviderType,
    v2_provider::{
        AudioProvider, CredentialProvider, DeviceProvider, DisplayProvider, InfrastructureProvider,
        NetworkProvider, ObservabilityProvider, Provider, ProviderCapabilitySet,
        ProviderDescriptor, RuntimeProvider, StorageProvider, SubstrateProvider, TransportProvider,
    },
};

use crate::FactoryError;

#[derive(Clone)]
pub enum ProviderInstance {
    Runtime(Arc<dyn RuntimeProvider>),
    Infrastructure(Arc<dyn InfrastructureProvider>),
    Transport(Arc<dyn TransportProvider>),
    Substrate(Arc<dyn SubstrateProvider>),
    Credential(Arc<dyn CredentialProvider>),
    Display(Arc<dyn DisplayProvider>),
    Network(Arc<dyn NetworkProvider>),
    Storage(Arc<dyn StorageProvider>),
    Device(Arc<dyn DeviceProvider>),
    Audio(Arc<dyn AudioProvider>),
    Observability(Arc<dyn ObservabilityProvider>),
}

impl fmt::Debug for ProviderInstance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderInstance")
            .field("provider_type", &self.provider_type())
            .finish_non_exhaustive()
    }
}

impl ProviderInstance {
    pub const fn provider_type(&self) -> ProviderType {
        match self {
            Self::Runtime(_) => ProviderType::Runtime,
            Self::Infrastructure(_) => ProviderType::Infrastructure,
            Self::Transport(_) => ProviderType::Transport,
            Self::Substrate(_) => ProviderType::Substrate,
            Self::Credential(_) => ProviderType::Credential,
            Self::Display(_) => ProviderType::Display,
            Self::Network(_) => ProviderType::Network,
            Self::Storage(_) => ProviderType::Storage,
            Self::Device(_) => ProviderType::Device,
            Self::Audio(_) => ProviderType::Audio,
            Self::Observability(_) => ProviderType::Observability,
        }
    }

    pub fn descriptor(&self) -> ProviderDescriptor {
        match self {
            Self::Runtime(provider) => provider.descriptor(),
            Self::Infrastructure(provider) => provider.descriptor(),
            Self::Transport(provider) => provider.descriptor(),
            Self::Substrate(provider) => provider.descriptor(),
            Self::Credential(provider) => provider.descriptor(),
            Self::Display(provider) => provider.descriptor(),
            Self::Network(provider) => provider.descriptor(),
            Self::Storage(provider) => provider.descriptor(),
            Self::Device(provider) => provider.descriptor(),
            Self::Audio(provider) => provider.descriptor(),
            Self::Observability(provider) => provider.descriptor(),
        }
    }

    pub fn capabilities(&self) -> ProviderCapabilitySet {
        match self {
            Self::Runtime(provider) => provider.capabilities(),
            Self::Infrastructure(provider) => provider.capabilities(),
            Self::Transport(provider) => provider.capabilities(),
            Self::Substrate(provider) => provider.capabilities(),
            Self::Credential(provider) => provider.descriptor().capabilities,
            Self::Display(provider) => provider.capabilities(),
            Self::Network(provider) => provider.capabilities(),
            Self::Storage(provider) => provider.capabilities(),
            Self::Device(provider) => provider.capabilities(),
            Self::Audio(provider) => provider.capabilities(),
            Self::Observability(provider) => provider.capabilities(),
        }
    }

    pub fn validate_capability_dispatch(&self) -> bool {
        !self
            .capabilities()
            .contains_method(d2b_contracts::v2_provider::ProviderMethod::RuntimeExecute)
    }

    pub fn provider(&self) -> &dyn Provider {
        match self {
            Self::Runtime(provider) => provider.as_ref(),
            Self::Infrastructure(provider) => provider.as_ref(),
            Self::Transport(provider) => provider.as_ref(),
            Self::Substrate(provider) => provider.as_ref(),
            Self::Credential(provider) => provider.as_ref(),
            Self::Display(provider) => provider.as_ref(),
            Self::Network(provider) => provider.as_ref(),
            Self::Storage(provider) => provider.as_ref(),
            Self::Device(provider) => provider.as_ref(),
            Self::Audio(provider) => provider.as_ref(),
            Self::Observability(provider) => provider.as_ref(),
        }
    }
}

pub trait ProviderFactory: Send + Sync {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError>;
}
