use std::{error::Error, fmt};

use d2b_contracts::{
    v2_identity::ProviderType,
    v2_provider::{ProviderDescriptor, ProviderFailure, ProviderMethod},
};
use d2b_provider::ProviderInstance;

use crate::Fixture;

#[derive(Clone, PartialEq, Eq)]
pub enum ConformanceError {
    Descriptor,
    CapabilityPublication,
    FixtureMismatch,
    Provider(Box<ProviderFailure>),
    Observation,
}

impl fmt::Debug for ConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Descriptor => formatter.write_str("Descriptor"),
            Self::CapabilityPublication => formatter.write_str("CapabilityPublication"),
            Self::FixtureMismatch => formatter.write_str("FixtureMismatch"),
            Self::Provider(error) => formatter.debug_tuple("Provider").field(error).finish(),
            Self::Observation => formatter.write_str("Observation"),
        }
    }
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Descriptor => "provider descriptor conformance failed",
            Self::CapabilityPublication => "provider capability publication conformance failed",
            Self::FixtureMismatch => "provider conformance fixture mismatch",
            Self::Provider(_) => "provider conformance call failed",
            Self::Observation => "provider observation conformance failed",
        })
    }
}

impl Error for ConformanceError {}

impl From<ProviderFailure> for ConformanceError {
    fn from(value: ProviderFailure) -> Self {
        Self::Provider(Box::new(value))
    }
}

pub fn check_descriptor_conformance(
    instance: &ProviderInstance,
) -> Result<ProviderDescriptor, ConformanceError> {
    let descriptor = instance.descriptor();
    descriptor
        .validate()
        .map_err(|_| ConformanceError::Descriptor)?;
    if instance.provider_type() != descriptor.provider_type() {
        return Err(ConformanceError::Descriptor);
    }
    let capabilities = instance.capabilities();
    if capabilities != descriptor.capabilities {
        return Err(ConformanceError::CapabilityPublication);
    }
    Ok(descriptor)
}

pub async fn check_provider_conformance(
    instance: &ProviderInstance,
    fixture: &Fixture,
) -> Result<(), ConformanceError> {
    let descriptor = check_descriptor_conformance(instance)?;
    if descriptor != fixture.descriptor {
        return Err(ConformanceError::FixtureMismatch);
    }
    let method = inspection_method(descriptor.provider_type());
    let request = fixture
        .request(method)
        .map_err(|_| ConformanceError::FixtureMismatch)?;
    let context = fixture.call_context(&request.context);
    let health = instance.provider().health(&context).await?;
    health
        .validate()
        .map_err(|_| ConformanceError::Observation)?;
    if health.provider_id != descriptor.provider_id
        || health.registry_generation != descriptor.registry_generation
    {
        return Err(ConformanceError::Observation);
    }
    let observation = match instance {
        ProviderInstance::Runtime(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Infrastructure(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Transport(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Substrate(provider) => provider.check(&context, &request).await?,
        ProviderInstance::Credential(provider) => provider.status(&context, &request).await?,
        ProviderInstance::Display(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Network(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Storage(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Device(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Audio(provider) => provider.inspect(&context, &request).await?,
        ProviderInstance::Observability(provider) => provider.status(&context, &request).await?,
    };
    observation
        .validate()
        .map_err(|_| ConformanceError::Observation)?;
    if observation.provider_id != descriptor.provider_id
        || observation.provider_generation != descriptor.registry_generation
        || observation.realm_id != *request.target.realm_id()
        || observation.workload_id.as_ref() != request.target.workload_id()
    {
        return Err(ConformanceError::Observation);
    }
    Ok(())
}

const fn inspection_method(provider_type: ProviderType) -> ProviderMethod {
    match provider_type {
        ProviderType::Runtime => ProviderMethod::RuntimeInspect,
        ProviderType::Infrastructure => ProviderMethod::InfrastructureInspect,
        ProviderType::Transport => ProviderMethod::TransportInspect,
        ProviderType::Substrate => ProviderMethod::SubstrateCheck,
        ProviderType::Credential => ProviderMethod::CredentialStatus,
        ProviderType::Display => ProviderMethod::DisplayInspect,
        ProviderType::Network => ProviderMethod::NetworkInspect,
        ProviderType::Storage => ProviderMethod::StorageInspect,
        ProviderType::Device => ProviderMethod::DeviceInspect,
        ProviderType::Audio => ProviderMethod::AudioInspect,
        ProviderType::Observability => ProviderMethod::ObservabilityStatus,
    }
}
