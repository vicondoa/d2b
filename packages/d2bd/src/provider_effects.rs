//! Descriptor-bound daemon effect ports for first-party providers.
//!
//! Provider crates receive only their semantic port traits. This module is the
//! trusted composition seam that binds those ports to one exact descriptor
//! before registry construction. The port implementations remain responsible
//! for resolving generated opaque IDs into current daemon, host, and broker
//! behavior; a missing or stale binding is an error, never a no-op.

use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use d2b_contracts::{v2_identity::ProviderId, v2_provider::ProviderDescriptor};
use d2b_provider_audio_pipewire_vhost_user::{AudioEffectPort, AudioQueryPort};
use d2b_provider_device_host_mediated::{DeviceEffectPort, DeviceQueryPort};
use d2b_provider_display_wayland::DisplayEffectPort;
use d2b_provider_network_local_realm::NetworkEffectPort;
use d2b_provider_observability_local::{ObservabilityExportPort, ObservabilityQueryPort};
use d2b_provider_runtime_local::RuntimeControlPort;
use d2b_provider_storage_local::StorageEffectPort;
use d2b_provider_substrate_host::HostSubstratePort;
use d2b_provider_transport_local::LocalEndpointPort;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonEffectAdapterError {
    DuplicateBinding,
    MappingUnavailable,
    ConfigurationMismatch,
    TransactionAborted,
}

impl fmt::Display for DaemonEffectAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicateBinding => "duplicate daemon provider effect binding",
            Self::MappingUnavailable => "generated daemon provider mapping is unavailable",
            Self::ConfigurationMismatch => {
                "daemon provider effect binding does not match the accepted descriptor"
            }
            Self::TransactionAborted => "daemon provider effect binding transaction was aborted",
        })
    }
}

impl Error for DaemonEffectAdapterError {}

struct ExactEffect<T: ?Sized> {
    descriptor: ProviderDescriptor,
    effect: Arc<T>,
}

impl<T: ?Sized> Clone for ExactEffect<T> {
    fn clone(&self) -> Self {
        Self {
            descriptor: self.descriptor.clone(),
            effect: Arc::clone(&self.effect),
        }
    }
}

#[derive(Clone)]
pub struct DeviceEffectAdapter {
    effects: Arc<dyn DeviceEffectPort>,
    queries: Arc<dyn DeviceQueryPort>,
}

impl DeviceEffectAdapter {
    pub fn effects(&self) -> Arc<dyn DeviceEffectPort> {
        Arc::clone(&self.effects)
    }

    pub fn queries(&self) -> Arc<dyn DeviceQueryPort> {
        Arc::clone(&self.queries)
    }
}

#[derive(Clone)]
pub struct AudioEffectAdapter {
    effects: Arc<dyn AudioEffectPort>,
    queries: Arc<dyn AudioQueryPort>,
}

impl AudioEffectAdapter {
    pub fn effects(&self) -> Arc<dyn AudioEffectPort> {
        Arc::clone(&self.effects)
    }

    pub fn queries(&self) -> Arc<dyn AudioQueryPort> {
        Arc::clone(&self.queries)
    }
}

#[derive(Clone)]
pub struct ObservabilityEffectAdapter {
    queries: Arc<dyn ObservabilityQueryPort>,
    exports: Arc<dyn ObservabilityExportPort>,
}

impl ObservabilityEffectAdapter {
    pub fn queries(&self) -> Arc<dyn ObservabilityQueryPort> {
        Arc::clone(&self.queries)
    }

    pub fn exports(&self) -> Arc<dyn ObservabilityExportPort> {
        Arc::clone(&self.exports)
    }
}

#[derive(Clone, Default)]
pub struct DaemonEffectAdapters {
    runtime: BTreeMap<ProviderId, ExactEffect<dyn RuntimeControlPort>>,
    transport: BTreeMap<ProviderId, ExactEffect<dyn LocalEndpointPort>>,
    substrate: BTreeMap<ProviderId, ExactEffect<dyn HostSubstratePort>>,
    display: BTreeMap<ProviderId, ExactEffect<dyn DisplayEffectPort>>,
    network: BTreeMap<ProviderId, ExactEffect<dyn NetworkEffectPort>>,
    storage: BTreeMap<ProviderId, ExactEffect<dyn StorageEffectPort>>,
    device: BTreeMap<ProviderId, ExactEffect<DeviceEffectAdapter>>,
    audio: BTreeMap<ProviderId, ExactEffect<AudioEffectAdapter>>,
    observability: BTreeMap<ProviderId, ExactEffect<ObservabilityEffectAdapter>>,
}

impl fmt::Debug for DaemonEffectAdapters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonEffectAdapters")
            .field("runtime", &self.runtime.len())
            .field("transport", &self.transport.len())
            .field("substrate", &self.substrate.len())
            .field("display", &self.display.len())
            .field("network", &self.network.len())
            .field("storage", &self.storage.len())
            .field("device", &self.device.len())
            .field("audio", &self.audio.len())
            .field("observability", &self.observability.len())
            .finish()
    }
}

impl DaemonEffectAdapters {
    pub fn builder() -> DaemonEffectAdaptersBuilder {
        DaemonEffectAdaptersBuilder::default()
    }

    pub fn runtime(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn RuntimeControlPort>, DaemonEffectAdapterError> {
        resolve(&self.runtime, descriptor)
    }

    pub fn transport(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn LocalEndpointPort>, DaemonEffectAdapterError> {
        resolve(&self.transport, descriptor)
    }

    pub fn substrate(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn HostSubstratePort>, DaemonEffectAdapterError> {
        resolve(&self.substrate, descriptor)
    }

    pub fn display(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn DisplayEffectPort>, DaemonEffectAdapterError> {
        resolve(&self.display, descriptor)
    }

    pub fn network(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn NetworkEffectPort>, DaemonEffectAdapterError> {
        resolve(&self.network, descriptor)
    }

    pub fn storage(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn StorageEffectPort>, DaemonEffectAdapterError> {
        resolve(&self.storage, descriptor)
    }

    pub fn device(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<DeviceEffectAdapter, DaemonEffectAdapterError> {
        resolve(&self.device, descriptor).map(|adapter| adapter.as_ref().clone())
    }

    pub fn audio(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<AudioEffectAdapter, DaemonEffectAdapterError> {
        resolve(&self.audio, descriptor).map(|adapter| adapter.as_ref().clone())
    }

    pub fn observability(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<ObservabilityEffectAdapter, DaemonEffectAdapterError> {
        resolve(&self.observability, descriptor).map(|adapter| adapter.as_ref().clone())
    }
}

#[derive(Default)]
pub struct DaemonEffectAdaptersBuilder {
    adapters: DaemonEffectAdapters,
    failed: bool,
}

impl fmt::Debug for DaemonEffectAdaptersBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonEffectAdaptersBuilder")
            .field("adapters", &self.adapters)
            .field("failed", &self.failed)
            .finish()
    }
}

macro_rules! bind_single_effect {
    ($method:ident, $field:ident, $trait:path) => {
        pub fn $method(
            &mut self,
            descriptor: ProviderDescriptor,
            effect: Arc<dyn $trait>,
        ) -> Result<&mut Self, DaemonEffectAdapterError> {
            let result = bind(&mut self.adapters.$field, descriptor, effect);
            self.finish_step(result)
        }
    };
}

impl DaemonEffectAdaptersBuilder {
    bind_single_effect!(bind_runtime, runtime, RuntimeControlPort);
    bind_single_effect!(bind_transport, transport, LocalEndpointPort);
    bind_single_effect!(bind_substrate, substrate, HostSubstratePort);
    bind_single_effect!(bind_display, display, DisplayEffectPort);
    bind_single_effect!(bind_network, network, NetworkEffectPort);
    bind_single_effect!(bind_storage, storage, StorageEffectPort);

    pub fn bind_device(
        &mut self,
        descriptor: ProviderDescriptor,
        effects: Arc<dyn DeviceEffectPort>,
        queries: Arc<dyn DeviceQueryPort>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        let adapter = Arc::new(DeviceEffectAdapter { effects, queries });
        let result = bind(&mut self.adapters.device, descriptor, adapter);
        self.finish_step(result)
    }

    pub fn bind_audio(
        &mut self,
        descriptor: ProviderDescriptor,
        effects: Arc<dyn AudioEffectPort>,
        queries: Arc<dyn AudioQueryPort>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        let adapter = Arc::new(AudioEffectAdapter { effects, queries });
        let result = bind(&mut self.adapters.audio, descriptor, adapter);
        self.finish_step(result)
    }

    pub fn bind_observability(
        &mut self,
        descriptor: ProviderDescriptor,
        queries: Arc<dyn ObservabilityQueryPort>,
        exports: Arc<dyn ObservabilityExportPort>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        let adapter = Arc::new(ObservabilityEffectAdapter { queries, exports });
        let result = bind(&mut self.adapters.observability, descriptor, adapter);
        self.finish_step(result)
    }

    fn finish_step(
        &mut self,
        result: Result<(), DaemonEffectAdapterError>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        match result {
            Ok(()) if !self.failed => Ok(self),
            Ok(()) => Err(DaemonEffectAdapterError::TransactionAborted),
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    pub fn finish(self) -> Result<DaemonEffectAdapters, DaemonEffectAdapterError> {
        if self.failed {
            Err(DaemonEffectAdapterError::TransactionAborted)
        } else {
            Ok(self.adapters)
        }
    }
}

fn bind<T: ?Sized>(
    bindings: &mut BTreeMap<ProviderId, ExactEffect<T>>,
    descriptor: ProviderDescriptor,
    effect: Arc<T>,
) -> Result<(), DaemonEffectAdapterError> {
    if bindings.contains_key(&descriptor.provider_id) {
        return Err(DaemonEffectAdapterError::DuplicateBinding);
    }
    bindings.insert(
        descriptor.provider_id.clone(),
        ExactEffect { descriptor, effect },
    );
    Ok(())
}

fn resolve<T: ?Sized>(
    bindings: &BTreeMap<ProviderId, ExactEffect<T>>,
    descriptor: &ProviderDescriptor,
) -> Result<Arc<T>, DaemonEffectAdapterError> {
    let binding = bindings
        .get(&descriptor.provider_id)
        .ok_or(DaemonEffectAdapterError::MappingUnavailable)?;
    if binding.descriptor != *descriptor {
        return Err(DaemonEffectAdapterError::ConfigurationMismatch);
    }
    Ok(Arc::clone(&binding.effect))
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v2_identity::ProviderType;
    use d2b_provider_toolkit::Fixture;

    use super::*;

    #[test]
    fn missing_and_mismatched_mappings_fail_closed() {
        let descriptor = Fixture::new(ProviderType::Runtime, 1)
            .expect("fixture")
            .descriptor;
        let adapters = DaemonEffectAdapters::builder()
            .finish()
            .expect("empty adapter set");
        assert!(matches!(
            adapters.runtime(&descriptor),
            Err(DaemonEffectAdapterError::MappingUnavailable)
        ));

        let mut bindings = BTreeMap::new();
        bind(&mut bindings, descriptor.clone(), Arc::new(1_u8)).expect("bind exact effect");
        let mut mismatched = descriptor;
        mismatched.registry_generation =
            d2b_contracts::v2_provider::Generation::new(2).expect("generation");
        assert!(matches!(
            resolve(&bindings, &mismatched),
            Err(DaemonEffectAdapterError::ConfigurationMismatch)
        ));
    }
}
