use std::{collections::BTreeMap, fmt, sync::Arc};

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
    LocalRuntimeConfiguration, LocalRuntimeKind, LocalRuntimeProvider,
    LocalRuntimeProviderBuildError, RuntimeControlPort,
};

#[derive(Clone)]
pub struct LocalRuntimeProviderFactoryEntry {
    descriptor: ProviderDescriptor,
    configuration: LocalRuntimeConfiguration,
    control: Arc<dyn RuntimeControlPort>,
}

impl fmt::Debug for LocalRuntimeProviderFactoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRuntimeProviderFactoryEntry")
            .field("kind", &self.configuration.kind())
            .finish_non_exhaustive()
    }
}

impl LocalRuntimeProviderFactoryEntry {
    pub fn new(
        descriptor: ProviderDescriptor,
        configuration: LocalRuntimeConfiguration,
        control: Arc<dyn RuntimeControlPort>,
    ) -> Self {
        Self {
            descriptor,
            configuration,
            control,
        }
    }

    pub fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.descriptor.provider_id
    }

    pub const fn kind(&self) -> LocalRuntimeKind {
        self.configuration.kind()
    }
}

#[derive(Clone)]
pub struct LocalRuntimeProviderFactory {
    key: ProviderFactoryKey,
    entries: BTreeMap<ProviderId, LocalRuntimeProviderFactoryEntry>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for LocalRuntimeProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRuntimeProviderFactory")
            .field("key", &self.key)
            .field("entry_count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl LocalRuntimeProviderFactory {
    pub fn cloud_hypervisor(
        entries: Vec<LocalRuntimeProviderFactoryEntry>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        Self::with_clock(
            LocalRuntimeKind::CloudHypervisor,
            entries,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn qemu_media(
        entries: Vec<LocalRuntimeProviderFactoryEntry>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        Self::with_clock(
            LocalRuntimeKind::QemuMedia,
            entries,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn systemd_user(
        entries: Vec<LocalRuntimeProviderFactoryEntry>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        Self::with_clock(
            LocalRuntimeKind::SystemdUser,
            entries,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        kind: LocalRuntimeKind,
        entries: Vec<LocalRuntimeProviderFactoryEntry>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, LocalRuntimeProviderBuildError> {
        if entries.is_empty() {
            return Err(LocalRuntimeProviderBuildError::FactoryEntriesEmpty);
        }
        if entries.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(LocalRuntimeProviderBuildError::FactoryEntryBoundExceeded);
        }

        let mut entries_by_provider = BTreeMap::new();
        for entry in entries {
            if entry.kind() != kind {
                return Err(LocalRuntimeProviderBuildError::RuntimeKindMismatch);
            }
            entry.descriptor.validate()?;
            if entry.descriptor.provider_type() != d2b_contracts::v2_identity::ProviderType::Runtime
            {
                return Err(LocalRuntimeProviderBuildError::ProviderTypeMismatch);
            }
            if entry.descriptor.implementation_id.as_str() != kind.implementation_id() {
                return Err(LocalRuntimeProviderBuildError::ImplementationMismatch);
            }
            let provider_id = entry.descriptor.provider_id.clone();
            if entries_by_provider.insert(provider_id, entry).is_some() {
                return Err(LocalRuntimeProviderBuildError::DuplicateProviderEntry);
            }
        }

        Ok(Self {
            key: kind.factory_key()?,
            entries: entries_by_provider,
            clock,
        })
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
        let entry = self
            .entries
            .get(&descriptor.provider_id)
            .ok_or(FactoryError::Rejected)?;
        if descriptor != &entry.descriptor {
            return Err(FactoryError::Rejected);
        }

        LocalRuntimeProvider::with_clock(
            descriptor.clone(),
            entry.configuration.clone(),
            entry.control.clone(),
            self.clock.clone(),
        )
        .map(|provider| ProviderInstance::Runtime(Arc::new(provider)))
        .map_err(|_| FactoryError::Rejected)
    }
}
