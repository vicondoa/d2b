use std::{fmt, sync::Arc};

use d2b_contracts::v2_provider::{
    ConfiguredItemId, ImplementationId, ProviderDescriptor, ProviderFactoryKey,
};
use d2b_host::{ch_argv::ChArgvInput, qemu_media_argv::QemuMediaArgvInput};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};

use crate::{
    LocalRuntimeConfiguration, LocalRuntimeProvider, LocalRuntimeProviderBuildError,
    RuntimeControlPort,
};

#[derive(Clone)]
pub struct LocalRuntimeProviderFactory {
    key: ProviderFactoryKey,
    configuration: LocalRuntimeConfiguration,
    control: Arc<dyn RuntimeControlPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for LocalRuntimeProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRuntimeProviderFactory")
            .field("key", &self.key)
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl LocalRuntimeProviderFactory {
    pub fn new(
        configuration: LocalRuntimeConfiguration,
        control: Arc<dyn RuntimeControlPort>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        Self::with_clock(configuration, control, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        configuration: LocalRuntimeConfiguration,
        control: Arc<dyn RuntimeControlPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        configuration.validate()?;
        let key = configuration.kind().factory_key()?;
        Ok(Self {
            key,
            configuration,
            control,
            clock,
        })
    }

    pub fn cloud_hypervisor(
        input: ChArgvInput,
        control: Arc<dyn RuntimeControlPort>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        Self::new(LocalRuntimeConfiguration::cloud_hypervisor(input)?, control)
    }

    pub fn qemu_media(
        input: QemuMediaArgvInput,
        control: Arc<dyn RuntimeControlPort>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        Self::new(LocalRuntimeConfiguration::qemu_media(input)?, control)
    }

    pub fn systemd_user(
        configured_items: Vec<ConfiguredItemId>,
        control: Arc<dyn RuntimeControlPort>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        Self::new(
            LocalRuntimeConfiguration::systemd_user(configured_items)?,
            control,
        )
    }

    pub fn key(&self) -> ProviderFactoryKey {
        self.key.clone()
    }

    pub fn implementation_id(&self) -> &ImplementationId {
        &self.key.implementation_id
    }
}

impl ProviderFactory for LocalRuntimeProviderFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != self.key.provider_type
            || descriptor.implementation_id != self.key.implementation_id
        {
            return Err(FactoryError::Rejected);
        }
        LocalRuntimeProvider::with_clock(
            descriptor.clone(),
            self.configuration.clone(),
            self.control.clone(),
            self.clock.clone(),
        )
        .map(|provider| ProviderInstance::Runtime(Arc::new(provider)))
        .map_err(|_| FactoryError::Rejected)
    }
}
