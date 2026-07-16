use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        HandleId, ImplementationId, MAX_PROVIDER_REGISTRY_ENTRIES, ProviderDescriptor,
        ProviderFactoryKey, TransportBindingId,
    },
};
use d2b_provider::{FactoryError, ProviderFactory, ProviderInstance};

use crate::{
    LocalEndpointPort, LocalEndpointResolver, LocalTransportClock, LocalTransportHandoffRegistry,
    LocalTransportKind, LocalTransportLimits, LocalTransportProvider, MAX_ACTIVE_LOCAL_TRANSPORTS,
    MAX_LOCAL_TRANSPORT_BINDINGS, OwnedLocalTransport, SystemTransportClock,
    TokioLocalEndpointPort, TransportBinding, TransportHandoffError,
};

pub const MAX_LOCAL_TRANSPORT_FACTORY_PROVIDERS: usize = MAX_PROVIDER_REGISTRY_ENTRIES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalTransportFactoryError {
    InvalidLimit,
    EmptyBindings,
    ProviderLimit,
    BindingLimit,
    DuplicateBinding,
    BindingKindMismatch,
    BindingDescriptorMismatch,
}

impl fmt::Display for LocalTransportFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimit => "local transport factory limit invalid",
            Self::EmptyBindings => "local transport factory has no bindings",
            Self::ProviderLimit => "local transport factory provider limit exceeded",
            Self::BindingLimit => "local transport factory binding limit exceeded",
            Self::DuplicateBinding => "local transport factory has a duplicate binding",
            Self::BindingKindMismatch => "local transport factory binding kind mismatch",
            Self::BindingDescriptorMismatch => {
                "local transport factory binding descriptor mismatch"
            }
        })
    }
}

impl Error for LocalTransportFactoryError {}

/// One constructed provider paired with its exact descriptor-handoff seam.
///
/// Register `instance` with [`ProviderRegistryBuilder::register_constructed`]
/// and retain `handoffs` in the session layer. Each construction owns a fresh
/// registry, so dropping another provider instance cannot clear this one.
///
/// [`ProviderRegistryBuilder::register_constructed`]: d2b_provider::ProviderRegistryBuilder::register_constructed
pub struct LocalTransportConstruction {
    instance: ProviderInstance,
    handoffs: Arc<LocalTransportHandoffRegistry>,
}

impl LocalTransportConstruction {
    pub fn instance(&self) -> ProviderInstance {
        self.instance.clone()
    }

    pub fn handoff_registry(&self) -> Arc<LocalTransportHandoffRegistry> {
        self.handoffs.clone()
    }

    pub fn into_parts(self) -> (ProviderInstance, Arc<LocalTransportHandoffRegistry>) {
        (self.instance, self.handoffs)
    }
}

impl fmt::Debug for LocalTransportConstruction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalTransportConstruction")
            .field("provider_id", &self.instance.descriptor().provider_id)
            .finish_non_exhaustive()
    }
}

/// Registry factory for one exact local transport implementation.
///
/// A factory owns no endpoint names. It groups pre-authorized bindings by their
/// canonical provider ID and shares only the injected semantic endpoint port
/// with constructed provider instances.
#[derive(Clone)]
pub struct LocalTransportFactory {
    kind: LocalTransportKind,
    endpoint_port: Arc<dyn LocalEndpointPort>,
    bindings: BTreeMap<ProviderId, Vec<TransportBinding>>,
    registered_handoffs: Arc<Mutex<BTreeMap<ProviderId, Weak<LocalTransportHandoffRegistry>>>>,
    clock: Arc<dyn LocalTransportClock>,
    limits: LocalTransportLimits,
}

impl fmt::Debug for LocalTransportFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalTransportFactory")
            .field("kind", &self.kind)
            .field("provider_count", &self.bindings.len())
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl LocalTransportFactory {
    pub fn new(
        kind: LocalTransportKind,
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::with_clock_and_limits(
            kind,
            endpoint_port,
            bindings,
            Arc::new(SystemTransportClock),
            LocalTransportLimits::default(),
        )
    }

    pub fn unix_stream(
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::new(LocalTransportKind::UnixStream, endpoint_port, bindings)
    }

    pub fn unix_stream_with_resolver(
        resolver: Arc<dyn LocalEndpointResolver>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::with_resolver(LocalTransportKind::UnixStream, resolver, bindings)
    }

    pub fn unix_seqpacket(
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::new(LocalTransportKind::UnixSeqpacket, endpoint_port, bindings)
    }

    pub fn unix_seqpacket_with_resolver(
        resolver: Arc<dyn LocalEndpointResolver>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::with_resolver(LocalTransportKind::UnixSeqpacket, resolver, bindings)
    }

    pub fn native_vsock(
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::new(LocalTransportKind::NativeVsock, endpoint_port, bindings)
    }

    pub fn native_vsock_with_resolver(
        resolver: Arc<dyn LocalEndpointResolver>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::with_resolver(LocalTransportKind::NativeVsock, resolver, bindings)
    }

    pub fn cloud_hypervisor_vsock(
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::new(
            LocalTransportKind::CloudHypervisorVsock,
            endpoint_port,
            bindings,
        )
    }

    pub fn cloud_hypervisor_vsock_with_resolver(
        resolver: Arc<dyn LocalEndpointResolver>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::with_resolver(LocalTransportKind::CloudHypervisorVsock, resolver, bindings)
    }

    pub fn with_resolver(
        kind: LocalTransportKind,
        resolver: Arc<dyn LocalEndpointResolver>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::new(
            kind,
            Arc::new(TokioLocalEndpointPort::new(resolver)),
            bindings,
        )
    }

    pub fn with_clock_and_limits(
        kind: LocalTransportKind,
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
        clock: Arc<dyn LocalTransportClock>,
        limits: LocalTransportLimits,
    ) -> Result<Self, LocalTransportFactoryError> {
        validate_limits(limits)?;
        let mut grouped =
            BTreeMap::<ProviderId, BTreeMap<TransportBindingId, TransportBinding>>::new();
        for binding in bindings {
            if binding.kind() != kind {
                return Err(LocalTransportFactoryError::BindingKindMismatch);
            }
            let provider_id = binding.provider_id().clone();
            if !grouped.contains_key(&provider_id)
                && grouped.len() >= MAX_LOCAL_TRANSPORT_FACTORY_PROVIDERS
            {
                return Err(LocalTransportFactoryError::ProviderLimit);
            }
            let provider_bindings = grouped.entry(provider_id).or_default();
            if provider_bindings.values().next().is_some_and(|configured| {
                configured.provider_generation() != binding.provider_generation()
                    || configured.configuration_fingerprint() != binding.configuration_fingerprint()
                    || configured.configured_scope_digest() != binding.configured_scope_digest()
            }) {
                return Err(LocalTransportFactoryError::BindingDescriptorMismatch);
            }
            if provider_bindings.contains_key(binding.binding_id()) {
                return Err(LocalTransportFactoryError::DuplicateBinding);
            }
            if provider_bindings.len() >= limits.max_bindings {
                return Err(LocalTransportFactoryError::BindingLimit);
            }
            provider_bindings.insert(binding.binding_id().clone(), binding);
        }
        if grouped.is_empty() {
            return Err(LocalTransportFactoryError::EmptyBindings);
        }
        Ok(Self {
            kind,
            endpoint_port,
            bindings: grouped
                .into_iter()
                .map(|(provider_id, bindings)| (provider_id, bindings.into_values().collect()))
                .collect(),
            registered_handoffs: Arc::new(Mutex::new(BTreeMap::new())),
            clock,
            limits,
        })
    }

    pub const fn kind(&self) -> LocalTransportKind {
        self.kind
    }

    pub fn implementation_id(&self) -> &'static ImplementationId {
        self.kind.implementation_id()
    }

    pub fn key(&self) -> ProviderFactoryKey {
        self.kind.factory_key()
    }

    pub fn registered_provider_count(&self) -> usize {
        self.bindings.len()
    }

    pub fn handoff_registry(
        &self,
        provider_id: &ProviderId,
    ) -> Option<Arc<LocalTransportHandoffRegistry>> {
        lock(&self.registered_handoffs)
            .get(provider_id)
            .and_then(Weak::upgrade)
    }

    pub fn take_transport(
        &self,
        provider_id: &ProviderId,
        handle_id: &HandleId,
    ) -> Result<OwnedLocalTransport, TransportHandoffError> {
        self.handoff_registry(provider_id)
            .ok_or(TransportHandoffError::UnknownHandle)?
            .take_transport(handle_id)
    }

    pub fn construct_with_handoff(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<LocalTransportConstruction, FactoryError> {
        let handoffs = Arc::new(LocalTransportHandoffRegistry::new());
        let instance = self.construct_isolated(descriptor, handoffs.clone())?;
        Ok(LocalTransportConstruction { instance, handoffs })
    }

    fn construct_isolated(
        &self,
        descriptor: &ProviderDescriptor,
        handoffs: Arc<LocalTransportHandoffRegistry>,
    ) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Transport
            || &descriptor.implementation_id != self.kind.implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let bindings = self
            .bindings
            .get(&descriptor.provider_id)
            .filter(|bindings| {
                bindings.iter().all(|binding| {
                    binding.provider_id() == &descriptor.provider_id
                        && binding.provider_generation() == descriptor.registry_generation
                        && binding.configuration_fingerprint()
                            == &descriptor.configuration_schema_fingerprint
                        && binding.configured_scope_digest() == &descriptor.configured_scope_digest
                        && binding.kind() == self.kind
                })
            })
            .ok_or(FactoryError::Rejected)?
            .clone();
        let provider = LocalTransportProvider::with_handoff_registry(
            descriptor.clone(),
            self.kind,
            bindings,
            self.endpoint_port.clone(),
            self.clock.clone(),
            self.limits,
            handoffs,
        )
        .map_err(|_| FactoryError::Rejected)?;
        Ok(ProviderInstance::Transport(Arc::new(provider)))
    }
}

impl ProviderFactory for LocalTransportFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        // The erased factory interface cannot return the instance handoff.
        // Keep exactly one weakly registered implicit construction so callers
        // can retrieve its seam from `handoff_registry` without sharing it.
        {
            let registered = lock(&self.registered_handoffs);
            if registered
                .get(&descriptor.provider_id)
                .and_then(Weak::upgrade)
                .is_some()
            {
                return Err(FactoryError::Rejected);
            }
        }
        let construction = self.construct_with_handoff(descriptor)?;
        let (instance, handoffs) = construction.into_parts();
        let mut registered = lock(&self.registered_handoffs);
        if registered
            .get(&descriptor.provider_id)
            .and_then(Weak::upgrade)
            .is_some()
        {
            return Err(FactoryError::Rejected);
        }
        registered.insert(descriptor.provider_id.clone(), Arc::downgrade(&handoffs));
        Ok(instance)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn validate_limits(limits: LocalTransportLimits) -> Result<(), LocalTransportFactoryError> {
    if limits.max_bindings == 0
        || limits.max_bindings > MAX_LOCAL_TRANSPORT_BINDINGS
        || limits.max_active == 0
        || limits.max_active > MAX_ACTIVE_LOCAL_TRANSPORTS
    {
        Err(LocalTransportFactoryError::InvalidLimit)
    } else {
        Ok(())
    }
}
