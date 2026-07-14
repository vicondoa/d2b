use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::ProviderId,
    v2_provider::{ProviderDescriptor, ProviderFactoryKey},
};
use d2b_provider::{
    FactoryError, ProviderFactory, ProviderInstance, ProviderRegistryBuilder, RegistryBuildError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolkitError {
    EmptyRegistration,
    DescriptorInvalid,
    CapabilityMismatch,
    DuplicateProvider,
    Registry(RegistryBuildError),
}

impl fmt::Display for ToolkitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRegistration => "provider registration is empty",
            Self::DescriptorInvalid => "provider descriptor is invalid",
            Self::CapabilityMismatch => "provider capability publication mismatch",
            Self::DuplicateProvider => "duplicate provider registration",
            Self::Registry(_) => "provider registry rejected registration",
        })
    }
}

impl Error for ToolkitError {}

impl From<RegistryBuildError> for ToolkitError {
    fn from(value: RegistryBuildError) -> Self {
        Self::Registry(value)
    }
}

struct ExactFactory {
    instances: BTreeMap<ProviderId, ProviderInstance>,
}

impl ProviderFactory for ExactFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        self.instances
            .get(&descriptor.provider_id)
            .filter(|instance| instance.descriptor() == *descriptor)
            .cloned()
            .ok_or(FactoryError::Rejected)
    }
}

pub fn register_exact_instances(
    builder: &mut ProviderRegistryBuilder,
    instances: impl IntoIterator<Item = ProviderInstance>,
) -> Result<(), ToolkitError> {
    let mut grouped: BTreeMap<ProviderFactoryKey, BTreeMap<ProviderId, ProviderInstance>> =
        BTreeMap::new();
    let mut count = 0usize;
    for instance in instances {
        count += 1;
        let descriptor = instance.descriptor();
        descriptor
            .validate()
            .map_err(|_| ToolkitError::DescriptorInvalid)?;
        if instance.capabilities() != descriptor.capabilities {
            return Err(ToolkitError::CapabilityMismatch);
        }
        let key = ProviderFactoryKey {
            provider_type: descriptor.provider_type(),
            implementation_id: descriptor.implementation_id.clone(),
        };
        if grouped
            .entry(key)
            .or_default()
            .insert(descriptor.provider_id, instance)
            .is_some()
        {
            return Err(ToolkitError::DuplicateProvider);
        }
    }
    if count == 0 {
        return Err(ToolkitError::EmptyRegistration);
    }

    for (key, instances) in grouped {
        let descriptors: Vec<_> = instances
            .values()
            .map(ProviderInstance::descriptor)
            .collect();
        builder.register_factory(
            key,
            Arc::new(ExactFactory {
                instances: instances.clone(),
            }),
        )?;
        for descriptor in descriptors {
            builder.register_instance(descriptor)?;
        }
    }
    Ok(())
}
