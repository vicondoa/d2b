use std::{fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::ProviderType,
    v2_provider::{
        ImplementationId, ProviderContractError, ProviderDescriptor, ProviderFactoryKey,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};

use crate::{
    ACA_IMPLEMENTATION_ID, AcaControl, AcaCredentialLeaseClient, AcaRuntimeConfig,
    AzureContainerAppsRuntimeProvider,
};

pub fn aca_provider_factory_key() -> Result<ProviderFactoryKey, ProviderContractError> {
    Ok(ProviderFactoryKey {
        provider_type: ProviderType::Runtime,
        implementation_id: ImplementationId::parse(ACA_IMPLEMENTATION_ID)?,
    })
}

#[derive(Clone)]
pub struct AzureContainerAppsRuntimeProviderFactory {
    configuration: AcaRuntimeConfig,
    credential_client: Arc<dyn AcaCredentialLeaseClient>,
    control: Arc<dyn AcaControl>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for AzureContainerAppsRuntimeProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureContainerAppsRuntimeProviderFactory")
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl AzureContainerAppsRuntimeProviderFactory {
    pub fn new(
        configuration: AcaRuntimeConfig,
        credential_client: Arc<dyn AcaCredentialLeaseClient>,
        control: Arc<dyn AcaControl>,
    ) -> Self {
        Self::with_clock(
            configuration,
            credential_client,
            control,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        configuration: AcaRuntimeConfig,
        credential_client: Arc<dyn AcaCredentialLeaseClient>,
        control: Arc<dyn AcaControl>,
        clock: Arc<dyn ProviderClock>,
    ) -> Self {
        Self {
            configuration,
            credential_client,
            control,
            clock,
        }
    }

    pub fn key() -> Result<ProviderFactoryKey, ProviderContractError> {
        aca_provider_factory_key()
    }
}

impl ProviderFactory for AzureContainerAppsRuntimeProviderFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Runtime
            || descriptor.implementation_id.as_str() != ACA_IMPLEMENTATION_ID
        {
            return Err(FactoryError::Rejected);
        }

        AzureContainerAppsRuntimeProvider::with_clock(
            descriptor.clone(),
            self.configuration.clone(),
            Arc::clone(&self.credential_client),
            Arc::clone(&self.control),
            Arc::clone(&self.clock),
        )
        .map(|provider| ProviderInstance::Runtime(Arc::new(provider)))
        .map_err(|_| FactoryError::Rejected)
    }
}
