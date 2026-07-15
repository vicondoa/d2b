use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        ImplementationId, MAX_PROVIDER_REGISTRY_ENTRIES, ProviderContractError, ProviderDescriptor,
        ProviderFactoryKey,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};

use crate::{
    ACA_IMPLEMENTATION_ID, AcaControl, AcaCredentialLeaseClient, AcaProviderBuildError,
    AcaRuntimeConfig, AzureContainerAppsRuntimeProvider,
};

pub fn aca_provider_factory_key() -> Result<ProviderFactoryKey, ProviderContractError> {
    Ok(ProviderFactoryKey {
        provider_type: ProviderType::Runtime,
        implementation_id: ImplementationId::parse(ACA_IMPLEMENTATION_ID)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaFactoryBuildError {
    Provider(AcaProviderBuildError),
    EmptyBindings,
    BoundExceeded,
    DuplicateProvider,
}

impl fmt::Display for AcaFactoryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Provider(_) => "Azure Container Apps factory binding is invalid",
            Self::EmptyBindings => "Azure Container Apps factory has no provider bindings",
            Self::BoundExceeded => "Azure Container Apps factory binding bound exceeded",
            Self::DuplicateProvider => {
                "Azure Container Apps factory has a duplicate provider binding"
            }
        })
    }
}

impl Error for AcaFactoryBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Provider(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AcaProviderBuildError> for AcaFactoryBuildError {
    fn from(value: AcaProviderBuildError) -> Self {
        Self::Provider(value)
    }
}

#[derive(Clone)]
pub struct AcaRuntimeProviderBinding {
    descriptor: ProviderDescriptor,
    configuration: AcaRuntimeConfig,
    credential_client: Arc<dyn AcaCredentialLeaseClient>,
    control: Arc<dyn AcaControl>,
}

impl fmt::Debug for AcaRuntimeProviderBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaRuntimeProviderBinding")
            .field("descriptor", &self.descriptor)
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl AcaRuntimeProviderBinding {
    pub fn new(
        descriptor: ProviderDescriptor,
        configuration: AcaRuntimeConfig,
        credential_client: Arc<dyn AcaCredentialLeaseClient>,
        control: Arc<dyn AcaControl>,
    ) -> Result<Self, AcaFactoryBuildError> {
        AzureContainerAppsRuntimeProvider::validate_descriptor(&descriptor)?;
        AzureContainerAppsRuntimeProvider::validate_credential_descriptor(
            &descriptor,
            &credential_client.descriptor(),
        )?;
        Ok(Self {
            descriptor,
            configuration,
            credential_client,
            control,
        })
    }

    pub fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
}

#[derive(Clone)]
pub struct AzureContainerAppsRuntimeProviderFactory {
    bindings: BTreeMap<ProviderId, AcaRuntimeProviderBinding>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for AzureContainerAppsRuntimeProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureContainerAppsRuntimeProviderFactory")
            .field("binding_count", &self.bindings.len())
            .finish_non_exhaustive()
    }
}

impl AzureContainerAppsRuntimeProviderFactory {
    pub fn new(bindings: Vec<AcaRuntimeProviderBinding>) -> Result<Self, AcaFactoryBuildError> {
        Self::with_clock(bindings, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        bindings: Vec<AcaRuntimeProviderBinding>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, AcaFactoryBuildError> {
        if bindings.is_empty() {
            return Err(AcaFactoryBuildError::EmptyBindings);
        }
        if bindings.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(AcaFactoryBuildError::BoundExceeded);
        }

        let mut by_provider = BTreeMap::new();
        for binding in bindings {
            if by_provider
                .insert(binding.descriptor.provider_id.clone(), binding)
                .is_some()
            {
                return Err(AcaFactoryBuildError::DuplicateProvider);
            }
        }
        Ok(Self {
            bindings: by_provider,
            clock,
        })
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
        let binding = self
            .bindings
            .get(&descriptor.provider_id)
            .ok_or(FactoryError::Rejected)?;
        if descriptor != &binding.descriptor {
            return Err(FactoryError::Rejected);
        }

        AzureContainerAppsRuntimeProvider::with_clock(
            descriptor.clone(),
            binding.configuration.clone(),
            Arc::clone(&binding.credential_client),
            Arc::clone(&binding.control),
            Arc::clone(&self.clock),
        )
        .map(|provider| ProviderInstance::Runtime(Arc::new(provider)))
        .map_err(|_| FactoryError::Rejected)
    }
}
