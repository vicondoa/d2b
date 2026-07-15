use std::{fmt, sync::Arc};

use d2b_contracts::v2_provider::{
    ImplementationId, ProviderContractError, ProviderDescriptor, ProviderFactoryKey,
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};

use crate::{HostSubstrateKind, HostSubstratePort, LinuxSubstrateProvider, NixOsSubstrateProvider};

#[derive(Clone)]
pub struct HostSubstrateProviderFactory {
    kind: HostSubstrateKind,
    key: ProviderFactoryKey,
    port: Arc<dyn HostSubstratePort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for HostSubstrateProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostSubstrateProviderFactory")
            .field("kind", &self.kind)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl HostSubstrateProviderFactory {
    pub fn nixos(port: Arc<dyn HostSubstratePort>) -> Result<Self, ProviderContractError> {
        Self::new(HostSubstrateKind::NixOs, port)
    }

    pub fn linux(port: Arc<dyn HostSubstratePort>) -> Result<Self, ProviderContractError> {
        Self::new(HostSubstrateKind::GenericLinux, port)
    }

    pub fn new(
        kind: HostSubstrateKind,
        port: Arc<dyn HostSubstratePort>,
    ) -> Result<Self, ProviderContractError> {
        Self::with_clock(kind, port, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        kind: HostSubstrateKind,
        port: Arc<dyn HostSubstratePort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, ProviderContractError> {
        Ok(Self {
            kind,
            key: kind.factory_key()?,
            port,
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
}

impl ProviderFactory for HostSubstrateProviderFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != self.key.provider_type
            || descriptor.implementation_id != self.key.implementation_id
        {
            return Err(FactoryError::Rejected);
        }

        match self.kind {
            HostSubstrateKind::NixOs => NixOsSubstrateProvider::with_clock(
                descriptor.clone(),
                self.port.clone(),
                self.clock.clone(),
            )
            .map(|provider| ProviderInstance::Substrate(Arc::new(provider)))
            .map_err(|_| FactoryError::Rejected),
            HostSubstrateKind::GenericLinux => LinuxSubstrateProvider::with_clock(
                descriptor.clone(),
                self.port.clone(),
                self.clock.clone(),
            )
            .map(|provider| ProviderInstance::Substrate(Arc::new(provider)))
            .map_err(|_| FactoryError::Rejected),
        }
    }
}
