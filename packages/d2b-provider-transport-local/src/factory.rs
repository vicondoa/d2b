use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        ImplementationId, MAX_PROVIDER_REGISTRY_ENTRIES, ProviderDescriptor, ProviderFactoryKey,
        TransportBindingId,
    },
};
use d2b_provider::{FactoryError, ProviderFactory, ProviderInstance};

use crate::{
    LocalEndpointPort, LocalTransportClock, LocalTransportKind, LocalTransportLimits,
    LocalTransportProvider, MAX_ACTIVE_LOCAL_TRANSPORTS, MAX_LOCAL_TRANSPORT_BINDINGS,
    SystemTransportClock, TransportBinding,
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
        })
    }
}

impl Error for LocalTransportFactoryError {}

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

    pub fn unix_seqpacket(
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::new(LocalTransportKind::UnixSeqpacket, endpoint_port, bindings)
    }

    pub fn native_vsock(
        endpoint_port: Arc<dyn LocalEndpointPort>,
        bindings: impl IntoIterator<Item = TransportBinding>,
    ) -> Result<Self, LocalTransportFactoryError> {
        Self::new(LocalTransportKind::NativeVsock, endpoint_port, bindings)
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
}

impl ProviderFactory for LocalTransportFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Transport
            || &descriptor.implementation_id != self.kind.implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let bindings = self
            .bindings
            .get(&descriptor.provider_id)
            .ok_or(FactoryError::Rejected)?
            .clone();
        let provider = LocalTransportProvider::with_clock_and_limits(
            descriptor.clone(),
            self.kind,
            bindings,
            self.endpoint_port.clone(),
            self.clock.clone(),
            self.limits,
        )
        .map_err(|_| FactoryError::Rejected)?;
        Ok(ProviderInstance::Transport(Arc::new(provider)))
    }
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
