//! Transactional first-party provider registry composition.
//!
//! Host composition accepts only exact first-party host descriptors and
//! descriptor-bound daemon effects. Credential-bearing implementations are
//! available only through the separate provider-agent composer.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    provider_registry_v2::{
        ProviderBindingV2ConsumerView, ProviderRegistryEntryV2, ProviderRegistryV2,
        ProviderRegistryV2Error,
    },
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::{
        ProviderId, ProviderType, RealmId, RealmPath as ProviderRealmPath, WorkloadId, WorkloadName,
    },
    v2_provider::{
        AuthorizedProviderScope, CorrelationId, Fingerprint, Generation, IdempotencyKey,
        MAX_PROVIDER_REQUEST_LIFETIME_MS, OperationId, PROVIDER_SCHEMA_VERSION, PrincipalRef,
        ProviderCallContext, ProviderCapability, ProviderDescriptor, ProviderFactoryKey,
        ProviderFailureKind, ProviderMethod, ProviderOperationContext, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlacement, ProviderTarget, RetryClass,
    },
};
use d2b_core::{
    bundle_resolver::{BundleResolver, ResolvedRunnerSource},
    processes::ProcessRole,
};
use d2b_provider::{
    AdmissionOptions, CancellationToken, FactoryError, ProviderFactory, ProviderInstance,
    ProviderRegistry, ProviderRegistryBuilder, RegistryBuildError,
    provider_capabilities_are_dispatchable,
};
use d2b_provider_audio_pipewire_vhost_user::{
    AudioConfiguration, IMPLEMENTATION_ID as AUDIO_IMPLEMENTATION_ID,
    PipewireVhostUserAudioFactory, PipewireVhostUserAudioFactoryEntry,
};
use d2b_provider_device_host_mediated::{
    DeviceSelectorDefinition, HostMediatedDeviceFactory, HostMediatedDeviceFactoryEntry,
    IMPLEMENTATION_ID as DEVICE_IMPLEMENTATION_ID,
};
use d2b_provider_display_wayland::{
    IMPLEMENTATION_ID as DISPLAY_IMPLEMENTATION_ID, WaylandDisplayBinding, WaylandDisplayFactory,
};
use d2b_provider_network_local_realm::{
    IMPLEMENTATION_ID as NETWORK_IMPLEMENTATION_ID, LocalRealmNetworkBinding,
    LocalRealmNetworkFactory,
};
use d2b_provider_observability_local::{
    IMPLEMENTATION_ID as OBSERVABILITY_IMPLEMENTATION_ID, LocalObservabilityFactory,
    LocalObservabilityFactoryEntry, ObservabilityLimits,
};
use d2b_provider_runtime_azure_container_apps::{
    ACA_IMPLEMENTATION_ID, AcaRuntimeProviderBinding, AzureContainerAppsRuntimeProviderFactory,
};
use d2b_provider_runtime_local::{
    CLOUD_HYPERVISOR_IMPLEMENTATION_ID, LocalRuntimeConfiguration, LocalRuntimeKind,
    LocalRuntimeProviderFactory, LocalRuntimeProviderFactoryEntry, QEMU_MEDIA_IMPLEMENTATION_ID,
    RuntimeBundleIntentId, RuntimeIntentBinding, RuntimeRunnerId, SYSTEMD_USER_IMPLEMENTATION_ID,
};
use d2b_provider_storage_local::{
    IMPLEMENTATION_ID as STORAGE_IMPLEMENTATION_ID, LocalStorageBinding, LocalStorageFactory,
};
use d2b_provider_substrate_host::{
    HostSubstrateConfiguration, HostSubstrateFactoryEntry, HostSubstrateKind,
    HostSubstrateProviderFactory, LINUX_IMPLEMENTATION_ID, NIXOS_IMPLEMENTATION_ID,
};
use d2b_provider_transport_azure_relay::{
    AZURE_RELAY_IMPLEMENTATION_ID, AzureRelayConfiguration, AzureRelayFactoryEntry,
    AzureRelayProviderFactory, RelayControlPort,
};
use d2b_provider_transport_local::{LocalTransportFactory, LocalTransportKind, TransportBinding};
use sha2::{Digest, Sha256};

use crate::{
    ServerState, load_bundle_resolver,
    provider_effects::{
        DaemonEffectAdapterError, DaemonEffectAdapters, ProviderLifecycleDispatch,
        ProviderLifecycleInvocationHandle,
    },
};

const AZURE_VM_IMPLEMENTATION_ID: &str = "azure-vm";
const PROVIDER_BUNDLE_VERSION: u32 = 12;
const PROVIDER_BUNDLE_SCHEMA_VERSION: &str = "v2";
static NEXT_LIFECYCLE_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCompositionError {
    ArtifactMissing,
    ArtifactMalformed,
    InvalidDescriptor,
    DuplicateDescriptor,
    DuplicateBinding,
    MissingBinding,
    DescriptorBindingMismatch,
    GenerationMismatch,
    WrongProcessPlacement,
    UnsupportedImplementation,
    UnsupportedBinding,
    AzureVmForbidden,
    NondispatchableCapability,
    ConfigurationMismatch,
    ProviderIdMismatch,
    ConfigurationSchemaFingerprintMismatch,
    ConfiguredScopeDigestMismatch,
    EffectAdapter(DaemonEffectAdapterError),
    Factory(FactoryError),
    Registry(RegistryBuildError),
    StartupProbeFailed,
    BundleContractMismatch,
    ProcessIdentityMismatch,
    DuplicateRuntimeMapping,
    LegacyRunnerForbidden,
    LifecycleBudgetExceeded,
}

impl fmt::Display for ProviderCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ArtifactMissing => "provider-registry-v2 artifact is missing",
            Self::ArtifactMalformed => "provider-registry-v2 artifact is malformed",
            Self::InvalidDescriptor => "provider descriptor is invalid",
            Self::DuplicateDescriptor => "provider composition has a duplicate descriptor",
            Self::DuplicateBinding => "provider composition has a duplicate configuration binding",
            Self::MissingBinding => "provider descriptor has no exact configuration binding",
            Self::DescriptorBindingMismatch => {
                "provider descriptor and configuration binding do not match"
            }
            Self::GenerationMismatch => {
                "provider descriptor generation does not match the registry"
            }
            Self::WrongProcessPlacement => {
                "provider implementation is not permitted in this process"
            }
            Self::UnsupportedImplementation => "provider implementation is not first-party live",
            Self::UnsupportedBinding => {
                "provider binding has no registered daemon consumer adapter"
            }
            Self::AzureVmForbidden => "Azure VM providers are not production implementations",
            Self::NondispatchableCapability => {
                "provider advertises a method without a live dispatcher"
            }
            Self::ConfigurationMismatch => {
                "provider factory rejected its exact configuration binding"
            }
            Self::ProviderIdMismatch => {
                "provider-registry-v2 provider ID does not match descriptor placement and workload binding"
            }
            Self::ConfigurationSchemaFingerprintMismatch => {
                "provider-registry-v2 configuration schema fingerprint does not match the first-party provider contract"
            }
            Self::ConfiguredScopeDigestMismatch => {
                "provider-registry-v2 configured scope digest does not match the closed provider binding"
            }
            Self::EffectAdapter(error) => return error.fmt(formatter),
            Self::Factory(error) => return error.fmt(formatter),
            Self::Registry(error) => return error.fmt(formatter),
            Self::StartupProbeFailed => "provider registry startup health/inspect probe failed",
            Self::BundleContractMismatch => {
                "provider registry requires an integrity-complete bundle v12/v2 contract"
            }
            Self::ProcessIdentityMismatch => {
                "provider runtime binding does not match the explicit process workload identity"
            }
            Self::DuplicateRuntimeMapping => {
                "multiple provider runtime bindings claim the same VM or intent"
            }
            Self::LegacyRunnerForbidden => {
                "provider runtime binding resolves through a legacy runner fallback"
            }
            Self::LifecycleBudgetExceeded => {
                "mapped runtime lifecycle exceeds the provider request lifetime contract"
            }
        })
    }
}

impl Error for ProviderCompositionError {}

fn map_provider_registry_validation_error(
    error: ProviderRegistryV2Error,
) -> ProviderCompositionError {
    match error {
        ProviderRegistryV2Error::InvalidDescriptor => ProviderCompositionError::InvalidDescriptor,
        ProviderRegistryV2Error::GenerationMismatch => ProviderCompositionError::GenerationMismatch,
        ProviderRegistryV2Error::DuplicateProvider => ProviderCompositionError::DuplicateDescriptor,
        ProviderRegistryV2Error::ProviderIdMismatch => ProviderCompositionError::ProviderIdMismatch,
        ProviderRegistryV2Error::ConfigurationSchemaFingerprintMismatch => {
            ProviderCompositionError::ConfigurationSchemaFingerprintMismatch
        }
        ProviderRegistryV2Error::ConfiguredScopeDigestMismatch => {
            ProviderCompositionError::ConfiguredScopeDigestMismatch
        }
        _ => ProviderCompositionError::ArtifactMalformed,
    }
}

impl From<DaemonEffectAdapterError> for ProviderCompositionError {
    fn from(value: DaemonEffectAdapterError) -> Self {
        Self::EffectAdapter(value)
    }
}

impl From<FactoryError> for ProviderCompositionError {
    fn from(value: FactoryError) -> Self {
        Self::Factory(value)
    }
}

impl From<RegistryBuildError> for ProviderCompositionError {
    fn from(value: RegistryBuildError) -> Self {
        Self::Registry(value)
    }
}

#[derive(Debug, Clone)]
pub enum StartupProviderRegistry {
    Empty,
    Live {
        registry: ProviderRegistry,
        runtime_routes: Arc<BTreeMap<String, ProviderRegistryEntryV2>>,
        lifecycle_dispatch: Arc<ProviderLifecycleDispatch>,
    },
}

impl StartupProviderRegistry {
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn registry(&self) -> Option<&ProviderRegistry> {
        match self {
            Self::Empty => None,
            Self::Live { registry, .. } => Some(registry),
        }
    }

    pub fn runtime_route(&self, vm: &str) -> Option<&ProviderRegistryEntryV2> {
        match self {
            Self::Empty => None,
            Self::Live { runtime_routes, .. } => runtime_routes.get(vm),
        }
    }

    pub(crate) fn lifecycle_dispatch(&self) -> &Arc<ProviderLifecycleDispatch> {
        match self {
            Self::Empty => {
                static EMPTY: std::sync::OnceLock<Arc<ProviderLifecycleDispatch>> =
                    std::sync::OnceLock::new();
                EMPTY.get_or_init(|| Arc::new(ProviderLifecycleDispatch::default()))
            }
            Self::Live {
                lifecycle_dispatch, ..
            } => lifecycle_dispatch,
        }
    }

    pub(crate) fn begin_lifecycle_invocation(
        &self,
        operation_id: &OperationId,
        request: d2b_contracts::public_wire::VmLifecycleRequest,
        caller_role: d2b_contracts::broker_wire::BrokerCallerRole,
    ) -> Result<ProviderLifecycleInvocationHandle, crate::TypedError> {
        self.lifecycle_dispatch()
            .begin(operation_id, request, caller_role)
    }

    fn with_runtime_routes(
        self,
        runtime_routes: BTreeMap<String, ProviderRegistryEntryV2>,
    ) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Live {
                registry,
                lifecycle_dispatch,
                ..
            } => Self::Live {
                registry,
                runtime_routes: Arc::new(runtime_routes),
                lifecycle_dispatch,
            },
        }
    }
}

#[derive(Clone)]
pub enum HostProviderBinding {
    LocalRuntime {
        descriptor: ProviderDescriptor,
        configuration: LocalRuntimeConfiguration,
    },
    LocalTransport {
        descriptor: ProviderDescriptor,
        kind: LocalTransportKind,
        bindings: Vec<TransportBinding>,
    },
    HostSubstrate {
        descriptor: ProviderDescriptor,
        configuration: HostSubstrateConfiguration,
    },
    WaylandDisplay {
        descriptor: ProviderDescriptor,
        binding: WaylandDisplayBinding,
    },
    LocalRealmNetwork {
        descriptor: ProviderDescriptor,
        binding: LocalRealmNetworkBinding,
    },
    LocalStorage {
        descriptor: ProviderDescriptor,
        binding: LocalStorageBinding,
    },
    HostMediatedDevice {
        descriptor: ProviderDescriptor,
        selectors: Vec<DeviceSelectorDefinition>,
    },
    PipewireVhostUserAudio {
        descriptor: ProviderDescriptor,
        configuration: AudioConfiguration,
    },
    LocalObservability {
        descriptor: ProviderDescriptor,
        limits: ObservabilityLimits,
    },
}

impl fmt::Debug for HostProviderBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostProviderBinding")
            .field("descriptor", self.descriptor())
            .finish_non_exhaustive()
    }
}

impl HostProviderBinding {
    pub fn descriptor(&self) -> &ProviderDescriptor {
        match self {
            Self::LocalRuntime { descriptor, .. }
            | Self::LocalTransport { descriptor, .. }
            | Self::HostSubstrate { descriptor, .. }
            | Self::WaylandDisplay { descriptor, .. }
            | Self::LocalRealmNetwork { descriptor, .. }
            | Self::LocalStorage { descriptor, .. }
            | Self::HostMediatedDevice { descriptor, .. }
            | Self::PipewireVhostUserAudio { descriptor, .. }
            | Self::LocalObservability { descriptor, .. } => descriptor,
        }
    }

    fn construct(
        &self,
        effects: &DaemonEffectAdapters,
    ) -> Result<(ProviderFactoryKey, ProviderInstance), ProviderCompositionError> {
        let descriptor = self.descriptor();
        match self {
            Self::LocalRuntime { configuration, .. } => {
                let kind = configuration.kind();
                if !matches!(
                    kind,
                    LocalRuntimeKind::CloudHypervisor | LocalRuntimeKind::QemuMedia
                ) {
                    return Err(ProviderCompositionError::WrongProcessPlacement);
                }
                let entry = LocalRuntimeProviderFactoryEntry::new(
                    descriptor.clone(),
                    configuration.clone(),
                    effects.runtime(descriptor)?,
                );
                let factory = match kind {
                    LocalRuntimeKind::CloudHypervisor => {
                        LocalRuntimeProviderFactory::cloud_hypervisor(vec![entry])
                    }
                    LocalRuntimeKind::QemuMedia => {
                        LocalRuntimeProviderFactory::qemu_media(vec![entry])
                    }
                    LocalRuntimeKind::SystemdUser => unreachable!("placement checked above"),
                }
                .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(factory.key(), &factory, descriptor)
            }
            Self::LocalTransport { kind, bindings, .. } => {
                if bindings.iter().any(|binding| {
                    binding.provider_id() != &descriptor.provider_id
                        || binding.provider_generation() != descriptor.registry_generation
                        || binding.configuration_fingerprint()
                            != &descriptor.configuration_schema_fingerprint
                        || binding.configured_scope_digest() != &descriptor.configured_scope_digest
                        || binding.realm_id() != descriptor.placement.realm_id()
                }) {
                    return Err(ProviderCompositionError::ConfigurationMismatch);
                }
                let factory = LocalTransportFactory::new(
                    *kind,
                    effects.transport(descriptor)?,
                    bindings.clone(),
                )
                .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(factory.key(), &factory, descriptor)
            }
            Self::HostSubstrate { configuration, .. } => {
                let entry = HostSubstrateFactoryEntry::new(
                    descriptor.clone(),
                    *configuration,
                    effects.substrate(descriptor)?,
                )
                .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                let factory = match configuration.substrate() {
                    HostSubstrateKind::NixOs => HostSubstrateProviderFactory::nixos([entry]),
                    HostSubstrateKind::GenericLinux => HostSubstrateProviderFactory::linux([entry]),
                }
                .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(factory.key(), &factory, descriptor)
            }
            Self::WaylandDisplay { binding, .. } => {
                let factory =
                    WaylandDisplayFactory::new(binding.clone(), effects.display(descriptor)?);
                construct_exact(factory_key(descriptor), &factory, descriptor)
            }
            Self::LocalRealmNetwork { binding, .. } => {
                let factory =
                    LocalRealmNetworkFactory::new(binding.clone(), effects.network(descriptor)?);
                construct_exact(
                    d2b_provider_network_local_realm::provider_factory_key(),
                    &factory,
                    descriptor,
                )
            }
            Self::LocalStorage { binding, .. } => {
                let factory =
                    LocalStorageFactory::new(binding.clone(), effects.storage(descriptor)?);
                construct_exact(
                    d2b_provider_storage_local::provider_factory_key(),
                    &factory,
                    descriptor,
                )
            }
            Self::HostMediatedDevice { selectors, .. } => {
                let adapter = effects.device(descriptor)?;
                let entry = HostMediatedDeviceFactoryEntry::new(
                    descriptor,
                    selectors.clone(),
                    adapter.effects(),
                    adapter.queries(),
                )
                .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                let factory = HostMediatedDeviceFactory::new(vec![entry])
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(
                    d2b_provider_device_host_mediated::factory_key(),
                    &factory,
                    descriptor,
                )
            }
            Self::PipewireVhostUserAudio { configuration, .. } => {
                let adapter = effects.audio(descriptor)?;
                let entry = PipewireVhostUserAudioFactoryEntry::new(
                    descriptor,
                    configuration.clone(),
                    adapter.effects(),
                    adapter.queries(),
                )
                .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                let factory = PipewireVhostUserAudioFactory::new(vec![entry])
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(
                    d2b_provider_audio_pipewire_vhost_user::factory_key(),
                    &factory,
                    descriptor,
                )
            }
            Self::LocalObservability { limits, .. } => {
                let adapter = effects.observability(descriptor)?;
                let entry = LocalObservabilityFactoryEntry::new(
                    descriptor,
                    *limits,
                    adapter.queries(),
                    adapter.exports(),
                )
                .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                let factory = LocalObservabilityFactory::new(vec![entry])
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(
                    d2b_provider_observability_local::factory_key(),
                    &factory,
                    descriptor,
                )
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostProviderComposition {
    pub generation: Generation,
    pub configuration_fingerprint: Fingerprint,
    pub published_at_unix_ms: u64,
    pub descriptors: Vec<ProviderDescriptor>,
    pub bindings: Vec<HostProviderBinding>,
}

pub fn compose_host_provider_registry(
    composition: HostProviderComposition,
    effects: &DaemonEffectAdapters,
) -> Result<StartupProviderRegistry, ProviderCompositionError> {
    if composition.descriptors.is_empty() && composition.bindings.is_empty() {
        return Ok(StartupProviderRegistry::Empty);
    }
    validate_exact_bindings(
        composition.generation,
        &composition.descriptors,
        composition
            .bindings
            .iter()
            .map(HostProviderBinding::descriptor),
        validate_host_descriptor,
    )?;

    let mut constructed = Vec::with_capacity(composition.bindings.len());
    for binding in &composition.bindings {
        constructed.push(binding.construct(effects)?);
    }
    assemble_registry(
        composition.generation,
        composition.configuration_fingerprint,
        composition.published_at_unix_ms,
        constructed,
    )
    .map(|registry| StartupProviderRegistry::Live {
        registry,
        runtime_routes: Arc::new(BTreeMap::new()),
        lifecycle_dispatch: Arc::new(ProviderLifecycleDispatch::default()),
    })
}

#[derive(Clone)]
pub enum AgentProviderBinding {
    SystemdUser {
        descriptor: ProviderDescriptor,
        configuration: LocalRuntimeConfiguration,
        control: Arc<dyn d2b_provider_runtime_local::RuntimeControlPort>,
    },
    AzureContainerApps {
        binding: AcaRuntimeProviderBinding,
    },
    AzureRelay {
        descriptor: ProviderDescriptor,
        configuration: AzureRelayConfiguration,
        control: Arc<dyn RelayControlPort>,
    },
}

impl fmt::Debug for AgentProviderBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderBinding")
            .field("descriptor", self.descriptor())
            .finish_non_exhaustive()
    }
}

impl AgentProviderBinding {
    pub fn descriptor(&self) -> &ProviderDescriptor {
        match self {
            Self::SystemdUser { descriptor, .. } | Self::AzureRelay { descriptor, .. } => {
                descriptor
            }
            Self::AzureContainerApps { binding } => binding.descriptor(),
        }
    }

    fn construct(
        &self,
    ) -> Result<(ProviderFactoryKey, ProviderInstance), ProviderCompositionError> {
        let descriptor = self.descriptor();
        match self {
            Self::SystemdUser {
                configuration,
                control,
                ..
            } => {
                if configuration.kind() != LocalRuntimeKind::SystemdUser {
                    return Err(ProviderCompositionError::ConfigurationMismatch);
                }
                let entry = LocalRuntimeProviderFactoryEntry::new(
                    descriptor.clone(),
                    configuration.clone(),
                    Arc::clone(control),
                );
                let factory = LocalRuntimeProviderFactory::systemd_user(vec![entry])
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(factory.key(), &factory, descriptor)
            }
            Self::AzureContainerApps { binding } => {
                let factory = AzureContainerAppsRuntimeProviderFactory::new(vec![binding.clone()])
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                let key = AzureContainerAppsRuntimeProviderFactory::key()
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(key, &factory, descriptor)
            }
            Self::AzureRelay {
                configuration,
                control,
                ..
            } => {
                let entry =
                    AzureRelayFactoryEntry::for_descriptor(descriptor, configuration.clone());
                let factory = AzureRelayProviderFactory::new(Arc::clone(control), [entry])
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                construct_exact(AzureRelayProviderFactory::key(), &factory, descriptor)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentProviderComposition {
    pub generation: Generation,
    pub configuration_fingerprint: Fingerprint,
    pub published_at_unix_ms: u64,
    pub descriptors: Vec<ProviderDescriptor>,
    pub bindings: Vec<AgentProviderBinding>,
}

pub fn compose_agent_provider_registry(
    composition: AgentProviderComposition,
) -> Result<StartupProviderRegistry, ProviderCompositionError> {
    if composition.descriptors.is_empty() && composition.bindings.is_empty() {
        return Ok(StartupProviderRegistry::Empty);
    }
    validate_exact_bindings(
        composition.generation,
        &composition.descriptors,
        composition
            .bindings
            .iter()
            .map(AgentProviderBinding::descriptor),
        validate_agent_descriptor,
    )?;

    let mut constructed = Vec::with_capacity(composition.bindings.len());
    for binding in &composition.bindings {
        constructed.push(binding.construct()?);
    }
    assemble_registry(
        composition.generation,
        composition.configuration_fingerprint,
        composition.published_at_unix_ms,
        constructed,
    )
    .map(|registry| StartupProviderRegistry::Live {
        registry,
        runtime_routes: Arc::new(BTreeMap::new()),
        lifecycle_dispatch: Arc::new(ProviderLifecycleDispatch::default()),
    })
}

fn validate_exact_bindings<'a>(
    generation: Generation,
    descriptors: &[ProviderDescriptor],
    bindings: impl Iterator<Item = &'a ProviderDescriptor>,
    placement_validator: fn(&ProviderDescriptor) -> Result<(), ProviderCompositionError>,
) -> Result<(), ProviderCompositionError> {
    let mut accepted = BTreeMap::new();
    for descriptor in descriptors {
        descriptor
            .validate()
            .map_err(|_| ProviderCompositionError::InvalidDescriptor)?;
        if descriptor.registry_generation != generation {
            return Err(ProviderCompositionError::GenerationMismatch);
        }
        if is_azure_vm(descriptor) {
            return Err(ProviderCompositionError::AzureVmForbidden);
        }
        if !provider_capabilities_are_dispatchable(&descriptor.capabilities) {
            return Err(ProviderCompositionError::NondispatchableCapability);
        }
        placement_validator(descriptor)?;
        if accepted
            .insert(descriptor.provider_id.clone(), descriptor)
            .is_some()
        {
            return Err(ProviderCompositionError::DuplicateDescriptor);
        }
    }

    let mut configured = BTreeSet::new();
    for binding in bindings {
        if !configured.insert(binding.provider_id.clone()) {
            return Err(ProviderCompositionError::DuplicateBinding);
        }
        if accepted.get(&binding.provider_id).copied() != Some(binding) {
            return Err(ProviderCompositionError::DescriptorBindingMismatch);
        }
    }
    if configured.len() != accepted.len() {
        return Err(ProviderCompositionError::MissingBinding);
    }
    Ok(())
}

fn validate_host_descriptor(
    descriptor: &ProviderDescriptor,
) -> Result<(), ProviderCompositionError> {
    if !matches!(
        descriptor.placement,
        ProviderPlacement::TrustedFirstPartyInProcess {
            controller_role: EndpointRole::LocalRootController | EndpointRole::RealmController,
            ..
        }
    ) {
        return Err(ProviderCompositionError::WrongProcessPlacement);
    }
    let implementation = descriptor.implementation_id.as_str();
    let supported = match descriptor.provider_type() {
        ProviderType::Runtime => {
            implementation == CLOUD_HYPERVISOR_IMPLEMENTATION_ID
                || implementation == QEMU_MEDIA_IMPLEMENTATION_ID
        }
        ProviderType::Transport => [
            LocalTransportKind::UnixStream,
            LocalTransportKind::UnixSeqpacket,
            LocalTransportKind::NativeVsock,
            LocalTransportKind::CloudHypervisorVsock,
        ]
        .into_iter()
        .any(|kind| kind.implementation_id() == &descriptor.implementation_id),
        ProviderType::Substrate => {
            implementation == NIXOS_IMPLEMENTATION_ID || implementation == LINUX_IMPLEMENTATION_ID
        }
        ProviderType::Display => implementation == DISPLAY_IMPLEMENTATION_ID,
        ProviderType::Network => implementation == NETWORK_IMPLEMENTATION_ID,
        ProviderType::Storage => implementation == STORAGE_IMPLEMENTATION_ID,
        ProviderType::Observability => implementation == OBSERVABILITY_IMPLEMENTATION_ID,
        ProviderType::Device => implementation == DEVICE_IMPLEMENTATION_ID,
        ProviderType::Audio => implementation == AUDIO_IMPLEMENTATION_ID,
        ProviderType::Infrastructure | ProviderType::Credential => false,
    };
    if supported {
        Ok(())
    } else if is_agent_implementation(descriptor)
        || descriptor.provider_type() == ProviderType::Credential
    {
        Err(ProviderCompositionError::WrongProcessPlacement)
    } else {
        Err(ProviderCompositionError::UnsupportedImplementation)
    }
}

fn validate_agent_descriptor(
    descriptor: &ProviderDescriptor,
) -> Result<(), ProviderCompositionError> {
    let implementation = (
        descriptor.provider_type(),
        descriptor.implementation_id.as_str(),
    );
    let placement_matches = match implementation {
        (ProviderType::Runtime, SYSTEMD_USER_IMPLEMENTATION_ID) => {
            matches!(descriptor.placement, ProviderPlacement::UserAgent { .. })
        }
        (ProviderType::Runtime, ACA_IMPLEMENTATION_ID)
        | (ProviderType::Transport, AZURE_RELAY_IMPLEMENTATION_ID) => {
            matches!(
                descriptor.placement,
                ProviderPlacement::ProviderAgent { .. }
            )
        }
        _ => return Err(ProviderCompositionError::UnsupportedImplementation),
    };
    if placement_matches {
        Ok(())
    } else {
        Err(ProviderCompositionError::WrongProcessPlacement)
    }
}

fn is_agent_implementation(descriptor: &ProviderDescriptor) -> bool {
    matches!(
        (
            descriptor.provider_type(),
            descriptor.implementation_id.as_str()
        ),
        (
            ProviderType::Runtime,
            SYSTEMD_USER_IMPLEMENTATION_ID | ACA_IMPLEMENTATION_ID
        ) | (ProviderType::Transport, AZURE_RELAY_IMPLEMENTATION_ID)
    )
}

fn is_azure_vm(descriptor: &ProviderDescriptor) -> bool {
    descriptor.implementation_id.as_str() == AZURE_VM_IMPLEMENTATION_ID
        && matches!(
            descriptor.provider_type(),
            ProviderType::Runtime | ProviderType::Infrastructure
        )
}

fn factory_key(descriptor: &ProviderDescriptor) -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: descriptor.provider_type(),
        implementation_id: descriptor.implementation_id.clone(),
    }
}

fn construct_exact(
    key: ProviderFactoryKey,
    factory: &dyn ProviderFactory,
    descriptor: &ProviderDescriptor,
) -> Result<(ProviderFactoryKey, ProviderInstance), ProviderCompositionError> {
    if key != factory_key(descriptor) {
        return Err(ProviderCompositionError::DescriptorBindingMismatch);
    }
    let instance = factory.construct(descriptor)?;
    if instance.descriptor() != *descriptor || instance.capabilities() != descriptor.capabilities {
        return Err(ProviderCompositionError::DescriptorBindingMismatch);
    }
    Ok((key, instance))
}

fn assemble_registry(
    generation: Generation,
    configuration_fingerprint: Fingerprint,
    published_at_unix_ms: u64,
    constructed: Vec<(ProviderFactoryKey, ProviderInstance)>,
) -> Result<ProviderRegistry, ProviderCompositionError> {
    let mut grouped = BTreeMap::<ProviderFactoryKey, Vec<ProviderInstance>>::new();
    for (key, instance) in constructed {
        grouped.entry(key).or_default().push(instance);
    }

    let mut builder =
        ProviderRegistryBuilder::new(generation, configuration_fingerprint, published_at_unix_ms);
    for (key, instances) in &grouped {
        builder.register_factory(
            key.clone(),
            Arc::new(ExactConstructedFactory::new(instances)),
        )?;
    }
    for (key, instances) in grouped {
        for instance in instances {
            builder.register_constructed(key.clone(), instance)?;
        }
    }
    builder.finish().map_err(ProviderCompositionError::from)
}

#[derive(Clone)]
struct ExactConstructedFactory {
    instances: BTreeMap<ProviderId, ProviderInstance>,
}

impl ExactConstructedFactory {
    fn new(instances: &[ProviderInstance]) -> Self {
        Self {
            instances: instances
                .iter()
                .map(|instance| (instance.descriptor().provider_id, instance.clone()))
                .collect(),
        }
    }
}

impl ProviderFactory for ExactConstructedFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        self.instances
            .get(&descriptor.provider_id)
            .filter(|instance| instance.descriptor() == *descriptor)
            .cloned()
            .ok_or(FactoryError::Rejected)
    }
}

fn validate_provider_bundle_contract(
    resolver: &BundleResolver,
) -> Result<(), ProviderCompositionError> {
    let bundle = &resolver.bundle;
    let provider_path = bundle
        .provider_registry_v2_path
        .as_ref()
        .ok_or(ProviderCompositionError::BundleContractMismatch)?;
    let artifact_hashes = bundle
        .artifact_hashes
        .as_ref()
        .ok_or(ProviderCompositionError::BundleContractMismatch)?;
    if bundle.bundle_version != PROVIDER_BUNDLE_VERSION
        || bundle.schema_version != PROVIDER_BUNDLE_SCHEMA_VERSION
        || bundle.bundle_hash.is_none()
        || !artifact_hashes.contains_key(provider_path)
    {
        return Err(ProviderCompositionError::BundleContractMismatch);
    }
    Ok(())
}

fn validate_runtime_routes(
    resolver: &BundleResolver,
    artifact: &ProviderRegistryV2,
) -> Result<BTreeMap<String, ProviderRegistryEntryV2>, ProviderCompositionError> {
    validate_provider_bundle_contract(resolver)?;
    let current: ProviderRegistryV2 = serde_json::from_slice(
        resolver
            .provider_registry_v2_bytes()
            .ok_or(ProviderCompositionError::ArtifactMissing)?,
    )
    .map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    if &current != artifact {
        return Err(ProviderCompositionError::ArtifactMalformed);
    }

    let mut routes = BTreeMap::new();
    let mut vm_start_intents = BTreeSet::new();
    let mut runner_intents = BTreeSet::new();
    for entry in &artifact.providers {
        let binding = match entry
            .binding
            .consumer_view()
            .map_err(|_| ProviderCompositionError::UnsupportedBinding)?
        {
            ProviderBindingV2ConsumerView::LocalRuntime(binding) => binding,
            ProviderBindingV2ConsumerView::LocalObservability(_) => continue,
            _ => return Err(ProviderCompositionError::UnsupportedBinding),
        };
        let realm_id = entry.descriptor.placement.realm_id();
        let vm_start = resolver
            .find_vm_start_intent(binding.vm_start_intent_id.as_str())
            .ok_or(ProviderCompositionError::ProcessIdentityMismatch)?;
        let runner = resolver
            .find_runner_intent(binding.runner_intent_id.as_str())
            .ok_or(ProviderCompositionError::ProcessIdentityMismatch)?;
        if runner.source != ResolvedRunnerSource::ExplicitProcessNode {
            return Err(ProviderCompositionError::LegacyRunnerForbidden);
        }
        if vm_start.vm_name != runner.vm_name
            || vm_start.role_id != runner.role_id
            || vm_start.role != runner.role
        {
            return Err(ProviderCompositionError::ProcessIdentityMismatch);
        }

        let (expected_role, expected_runtime_kind, expected_legacy_provider) =
            match entry.descriptor.implementation_id.as_str() {
                CLOUD_HYPERVISOR_IMPLEMENTATION_ID => (
                    ProcessRole::CloudHypervisorRunner,
                    "nixos",
                    "local-cloud-hypervisor",
                ),
                QEMU_MEDIA_IMPLEMENTATION_ID => (
                    ProcessRole::QemuMediaRunner,
                    "qemu-media",
                    "local-qemu-media",
                ),
                AZURE_VM_IMPLEMENTATION_ID => {
                    return Err(ProviderCompositionError::AzureVmForbidden);
                }
                _ => return Err(ProviderCompositionError::UnsupportedImplementation),
            };
        if runner.role != expected_role {
            return Err(ProviderCompositionError::ProcessIdentityMismatch);
        }

        let dag = resolver
            .find_process_vm(&runner.vm_name)
            .ok_or(ProviderCompositionError::ProcessIdentityMismatch)?;
        let identity = dag
            .workload_identity
            .as_ref()
            .ok_or(ProviderCompositionError::ProcessIdentityMismatch)?;
        let provider_realm_path =
            ProviderRealmPath::parse(format!("{}.local-root", identity.realm_path.target_form()))
                .map_err(|_| ProviderCompositionError::ProcessIdentityMismatch)?;
        let expected_realm_id = RealmId::derive(&provider_realm_path);
        let workload_name = WorkloadName::parse(identity.workload_id.as_str())
            .map_err(|_| ProviderCompositionError::ProcessIdentityMismatch)?;
        let expected_workload_id = WorkloadId::derive(&expected_realm_id, &workload_name);
        if realm_id != &expected_realm_id
            || binding.workload_id != expected_workload_id
            || identity.legacy_vm_name.as_ref().map(|value| value.as_str())
                != Some(runner.vm_name.as_str())
            || identity.runtime_kind.as_ref().map(|value| value.as_str())
                != Some(expected_runtime_kind)
            || identity.provider_id.as_ref().map(|value| value.as_str())
                != Some(expected_legacy_provider)
        {
            return Err(ProviderCompositionError::ProcessIdentityMismatch);
        }

        if !vm_start_intents.insert(binding.vm_start_intent_id.clone())
            || !runner_intents.insert(binding.runner_intent_id.clone())
            || routes
                .insert(runner.vm_name.clone(), entry.clone())
                .is_some()
        {
            return Err(ProviderCompositionError::DuplicateRuntimeMapping);
        }
    }
    Ok(routes)
}

pub(crate) fn resolve_current_runtime_route(
    state: &ServerState,
    expected: &ProviderRegistryEntryV2,
) -> Result<(String, String), ProviderCompositionError> {
    let resolver =
        load_bundle_resolver(state).map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    let artifact: ProviderRegistryV2 = serde_json::from_slice(
        resolver
            .provider_registry_v2_bytes()
            .ok_or(ProviderCompositionError::ArtifactMissing)?,
    )
    .map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    artifact
        .validate()
        .map_err(map_provider_registry_validation_error)?;
    let routes = validate_runtime_routes(&resolver, &artifact)?;
    let binding = match expected
        .binding
        .consumer_view()
        .map_err(|_| ProviderCompositionError::UnsupportedBinding)?
    {
        ProviderBindingV2ConsumerView::LocalRuntime(binding) => binding,
        ProviderBindingV2ConsumerView::LocalObservability(_) => {
            return Err(ProviderCompositionError::ProcessIdentityMismatch);
        }
        _ => return Err(ProviderCompositionError::UnsupportedBinding),
    };
    let runner = resolver
        .find_runner_intent(binding.runner_intent_id.as_str())
        .ok_or(ProviderCompositionError::ProcessIdentityMismatch)?;
    if routes.get(&runner.vm_name) != Some(expected) {
        return Err(ProviderCompositionError::ProcessIdentityMismatch);
    }
    let registration_role = match runner.role {
        ProcessRole::CloudHypervisorRunner => crate::VM_RUNNER_ROLE_ID.to_owned(),
        ProcessRole::QemuMediaRunner => runner.role_id.clone(),
        _ => return Err(ProviderCompositionError::ProcessIdentityMismatch),
    };
    Ok((runner.vm_name.clone(), registration_role))
}

pub(crate) fn load_provider_registry_v2(
    state: &ServerState,
) -> Result<ProviderRegistryV2, ProviderCompositionError> {
    let resolver =
        load_bundle_resolver(state).map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    validate_provider_bundle_contract(&resolver)?;
    let bytes = resolver
        .provider_registry_v2_bytes()
        .ok_or(ProviderCompositionError::ArtifactMissing)?;
    let artifact: ProviderRegistryV2 =
        serde_json::from_slice(bytes).map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    artifact
        .validate()
        .map_err(map_provider_registry_validation_error)?;
    Ok(artifact)
}

pub(crate) fn load_provider_registry_v2_with_policy(
    state: &ServerState,
    policy: &d2b_core::bundle_resolver::BundleVerifyPolicy,
) -> Result<ProviderRegistryV2, ProviderCompositionError> {
    let resolver = d2b_core::bundle_resolver::BundleResolver::load_with_policy(
        &state.config.artifacts.bundle_path,
        policy,
    )
    .map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    validate_provider_bundle_contract(&resolver)?;
    let bytes = resolver
        .provider_registry_v2_bytes()
        .ok_or(ProviderCompositionError::ArtifactMissing)?;
    let artifact: ProviderRegistryV2 =
        serde_json::from_slice(bytes).map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    artifact
        .validate()
        .map_err(map_provider_registry_validation_error)?;
    Ok(artifact)
}

pub(crate) fn compose_startup_registry_with_policy(
    state: &Arc<ServerState>,
    artifact: &ProviderRegistryV2,
    policy: Option<&d2b_core::bundle_resolver::BundleVerifyPolicy>,
) -> Result<StartupProviderRegistry, ProviderCompositionError> {
    artifact
        .validate()
        .map_err(map_provider_registry_validation_error)?;
    let resolver = match policy {
        Some(policy) => {
            BundleResolver::load_with_policy(&state.config.artifacts.bundle_path, policy)
                .map_err(|_| ProviderCompositionError::ArtifactMalformed)?
        }
        None => {
            load_bundle_resolver(state).map_err(|_| ProviderCompositionError::ArtifactMalformed)?
        }
    };
    let runtime_routes = validate_runtime_routes(&resolver, artifact)?;
    validate_runtime_lifecycle_budgets(state, &runtime_routes)?;
    let effects =
        DaemonEffectAdapters::for_server_state(Arc::downgrade(state), &artifact.providers)?;
    let bindings = artifact
        .providers
        .iter()
        .map(|entry| {
            let binding = entry
                .binding
                .consumer_view()
                .map_err(|_| ProviderCompositionError::UnsupportedBinding)?;
            match binding {
                ProviderBindingV2ConsumerView::LocalRuntime(binding) => {
                    let intent = RuntimeIntentBinding::new(
                        RuntimeBundleIntentId::parse(binding.vm_start_intent_id.as_str())
                            .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?,
                        RuntimeRunnerId::parse(binding.runner_intent_id.as_str())
                            .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?,
                    );
                    let configuration = match entry.descriptor.implementation_id.as_str() {
                        CLOUD_HYPERVISOR_IMPLEMENTATION_ID => {
                            LocalRuntimeConfiguration::CloudHypervisor(intent)
                        }
                        QEMU_MEDIA_IMPLEMENTATION_ID => {
                            LocalRuntimeConfiguration::QemuMedia(intent)
                        }
                        AZURE_VM_IMPLEMENTATION_ID => {
                            return Err(ProviderCompositionError::AzureVmForbidden);
                        }
                        _ => return Err(ProviderCompositionError::UnsupportedImplementation),
                    };
                    Ok(HostProviderBinding::LocalRuntime {
                        descriptor: entry.descriptor.clone(),
                        configuration,
                    })
                }
                ProviderBindingV2ConsumerView::LocalObservability(binding) => {
                    let limits = ObservabilityLimits::new(
                        binding.max_records,
                        binding.max_bytes,
                        binding.max_time_window_ms,
                    )
                    .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
                    Ok(HostProviderBinding::LocalObservability {
                        descriptor: entry.descriptor.clone(),
                        limits,
                    })
                }
                _ => Err(ProviderCompositionError::UnsupportedBinding),
            }
        })
        .collect::<Result<Vec<_>, ProviderCompositionError>>()?;
    compose_host_provider_registry(
        HostProviderComposition {
            generation: artifact.registry_generation,
            configuration_fingerprint: artifact.configuration_fingerprint.clone(),
            published_at_unix_ms: artifact.published_at_unix_ms,
            descriptors: artifact
                .providers
                .iter()
                .map(|entry| entry.descriptor.clone())
                .collect(),
            bindings,
        },
        &effects,
    )
    .map(|registry| registry.with_runtime_routes(runtime_routes))
}

pub(crate) enum RuntimeLifecycleInvocation {
    Unmapped,
    Direct(serde_json::Value),
    Converged,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderLifecycleDeadline {
    pub duration: Duration,
    pub milliseconds: u32,
}

fn lifecycle_budget_within_provider_contract(duration: Duration) -> bool {
    !duration.is_zero() && duration.as_millis() <= u128::from(MAX_PROVIDER_REQUEST_LIFETIME_MS)
}

fn validate_runtime_lifecycle_budgets(
    state: &ServerState,
    routes: &BTreeMap<String, ProviderRegistryEntryV2>,
) -> Result<(), ProviderCompositionError> {
    for vm in routes.keys() {
        let request = d2b_contracts::public_wire::VmLifecycleRequest {
            vm: vm.clone(),
            flags: d2b_contracts::public_wire::MutationFlags {
                dry_run: false,
                apply: true,
                json: true,
            },
            force: false,
            no_wait_api: false,
        };
        let budgets = crate::mapped_runtime_lifecycle_budgets(state, &request)
            .map_err(|_| ProviderCompositionError::ConfigurationMismatch)?;
        if !lifecycle_budget_within_provider_contract(budgets.restart) {
            return Err(ProviderCompositionError::LifecycleBudgetExceeded);
        }
    }
    Ok(())
}

pub(crate) fn ensure_runtime_restart_budget(
    state: &ServerState,
    request: &d2b_contracts::public_wire::VmLifecycleRequest,
) -> Result<(), crate::TypedError> {
    let budgets = crate::mapped_runtime_lifecycle_budgets(state, request)?;
    if lifecycle_budget_within_provider_contract(budgets.restart) {
        Ok(())
    } else {
        Err(crate::TypedError::InternalConfig {
            detail: "mapped runtime restart exceeds provider contract lifetime".to_owned(),
        })
    }
}

pub(crate) fn provider_lifecycle_deadline(
    state: &ServerState,
    request: &d2b_contracts::public_wire::VmLifecycleRequest,
    method: ProviderMethod,
) -> Result<ProviderLifecycleDeadline, crate::TypedError> {
    let actual = crate::mapped_runtime_lifecycle_budget(state, request, method)?;
    if !lifecycle_budget_within_provider_contract(actual) {
        return Err(crate::TypedError::InternalConfig {
            detail: "mapped runtime lifecycle exceeds provider contract lifetime".to_owned(),
        });
    }
    let milliseconds = actual.as_millis();
    let milliseconds =
        u32::try_from(milliseconds).map_err(|_| crate::TypedError::InternalConfig {
            detail: "provider lifecycle deadline exceeds contract bounds".to_owned(),
        })?;
    Ok(ProviderLifecycleDeadline {
        duration: Duration::from_millis(u64::from(milliseconds)),
        milliseconds,
    })
}

pub(crate) async fn invoke_runtime_lifecycle(
    state: &ServerState,
    request: d2b_contracts::public_wire::VmLifecycleRequest,
    caller_role: d2b_contracts::broker_wire::BrokerCallerRole,
    method: ProviderMethod,
) -> Result<RuntimeLifecycleInvocation, crate::TypedError> {
    if !matches!(
        method,
        ProviderMethod::RuntimeStart | ProviderMethod::RuntimeStop
    ) {
        return Err(crate::TypedError::InternalConfig {
            detail: "unsupported mapped runtime lifecycle method".to_owned(),
        });
    }
    let startup = state.provider_registry()?;
    let Some(entry) = startup.runtime_route(&request.vm) else {
        return Ok(RuntimeLifecycleInvocation::Unmapped);
    };
    let binding = match entry.binding.consumer_view() {
        Ok(ProviderBindingV2ConsumerView::LocalRuntime(binding)) => binding,
        Ok(ProviderBindingV2ConsumerView::LocalObservability(_)) => {
            return Err(crate::TypedError::InternalConfig {
                detail: "runtime route resolved to a non-runtime provider binding".to_owned(),
            });
        }
        Ok(_) | Err(_) => {
            return Err(crate::TypedError::InternalConfig {
                detail: "runtime route resolved to an unsupported provider binding".to_owned(),
            });
        }
    };
    let realm_id = entry.descriptor.placement.realm_id();
    let registry = startup
        .registry()
        .ok_or_else(|| crate::TypedError::InternalConfig {
            detail: "mapped runtime provider registry is empty".to_owned(),
        })?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| crate::TypedError::InternalConfig {
            detail: "system time is unavailable for provider dispatch".to_owned(),
        })?
        .as_millis()
        .try_into()
        .map_err(|_| crate::TypedError::InternalConfig {
            detail: "system time exceeds provider contract bounds".to_owned(),
        })?;
    let deadline = provider_lifecycle_deadline(state, &request, method)?;
    let sequence = NEXT_LIFECYCLE_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
    let operation_id = OperationId::parse(format!("lifecycle-{sequence}")).map_err(|_| {
        crate::TypedError::InternalConfig {
            detail: "provider lifecycle operation id is invalid".to_owned(),
        }
    })?;
    let idempotency_key = IdempotencyKey::parse(format!("lifecycle-{sequence}")).map_err(|_| {
        crate::TypedError::InternalConfig {
            detail: "provider lifecycle idempotency key is invalid".to_owned(),
        }
    })?;
    let request_material = format!(
        "{}:{}:{}:{}:{}:{}",
        method.as_str(),
        entry.descriptor.provider_id.as_str(),
        realm_id.as_str(),
        binding.workload_id.as_str(),
        request.force,
        request.no_wait_api,
    );
    let request_digest =
        Fingerprint::parse(format!("{:x}", Sha256::digest(request_material.as_bytes()))).map_err(
            |_| crate::TypedError::InternalConfig {
                detail: "provider lifecycle request digest is invalid".to_owned(),
            },
        )?;
    let operation = ProviderOperationContext {
        schema_version: PROVIDER_SCHEMA_VERSION,
        operation_id,
        idempotency_key,
        request_digest: request_digest.clone(),
        scope: AuthorizedProviderScope::Workload {
            realm_id: realm_id.clone(),
            workload_id: binding.workload_id.clone(),
        },
        principal: PrincipalRef::parse("daemon-lifecycle").map_err(|_| {
            crate::TypedError::InternalConfig {
                detail: "provider lifecycle principal is invalid".to_owned(),
            }
        })?,
        provider_id: entry.descriptor.provider_id.clone(),
        provider_type: ProviderType::Runtime,
        provider_generation: entry.descriptor.registry_generation,
        capability: ProviderCapability(method),
        method,
        policy_epoch: Generation::new(1).map_err(|_| crate::TypedError::InternalConfig {
            detail: "provider lifecycle policy generation is invalid".to_owned(),
        })?,
        authorization_decision_digest: request_digest,
        issued_at_unix_ms: now,
        expires_at_unix_ms: now
            .checked_add(u64::from(deadline.milliseconds))
            .ok_or_else(|| crate::TypedError::InternalConfig {
                detail: "provider lifecycle deadline overflow".to_owned(),
            })?,
        correlation_id: CorrelationId::parse(format!("lifecycle-{sequence}")).map_err(|_| {
            crate::TypedError::InternalConfig {
                detail: "provider lifecycle correlation id is invalid".to_owned(),
            }
        })?,
        trace_id: Fingerprint::parse(format!(
            "{:x}",
            Sha256::digest(format!("trace-{sequence}").as_bytes())
        ))
        .map_err(|_| crate::TypedError::InternalConfig {
            detail: "provider lifecycle trace id is invalid".to_owned(),
        })?,
    };
    let provider_request = ProviderOperationRequest {
        context: operation.clone(),
        target: ProviderTarget::Workload {
            realm_id: realm_id.clone(),
            workload_id: binding.workload_id.clone(),
        },
        expected_configuration_fingerprint: entry
            .descriptor
            .configuration_schema_fingerprint
            .clone(),
        input: ProviderOperationInput::NoInput,
    };
    let admitted = registry
        .admit(
            operation.clone(),
            AdmissionOptions {
                expected_method: method,
                peer_role: EndpointRole::RealmController,
                service: ServicePackage::ProviderV2,
                deadline_after: deadline.duration,
                caller_cancellation: CancellationToken::new(),
                now_unix_ms: now,
            },
        )
        .map_err(|_| crate::TypedError::InternalConfig {
            detail: "mapped runtime provider admission failed".to_owned(),
        })?;
    let call = admitted
        .context
        .call_context()
        .map_err(|_| crate::TypedError::InternalConfig {
            detail: "mapped runtime provider call context is unavailable".to_owned(),
        })?;
    let ProviderInstance::Runtime(runtime) = &admitted.instance else {
        return Err(crate::TypedError::InternalConfig {
            detail: "mapped runtime provider has the wrong axis".to_owned(),
        });
    };
    let invocation =
        startup.begin_lifecycle_invocation(&operation.operation_id, request, caller_role)?;
    let provider_result = match method {
        ProviderMethod::RuntimeStart => runtime.start(&call, &provider_request).await,
        ProviderMethod::RuntimeStop => runtime.stop(&call, &provider_request).await,
        _ => unreachable!("method checked above"),
    };
    let direct_result = invocation.finish();
    if let Some(result) = direct_result {
        let value = result?;
        if value.get("outcome").and_then(serde_json::Value::as_str) == Some("applied")
            && let Err(failure) = provider_result
            && (failure.kind != ProviderFailureKind::AmbiguousMutation
                || failure.retry != RetryClass::AfterObservation)
        {
            return Err(crate::TypedError::InternalConfig {
                detail: "mapped runtime provider rejected applied lifecycle dispatch".to_owned(),
            });
        }
        return Ok(RuntimeLifecycleInvocation::Direct(value));
    }
    let observation = match provider_result {
        Ok(observation) => observation,
        Err(failure)
            if failure.kind == ProviderFailureKind::AmbiguousMutation
                && failure.retry == RetryClass::AfterObservation =>
        {
            return Ok(RuntimeLifecycleInvocation::Ambiguous);
        }
        Err(_) => {
            return Err(crate::TypedError::InternalConfig {
                detail: "mapped runtime provider rejected lifecycle dispatch".to_owned(),
            });
        }
    };
    observation
        .validate()
        .map_err(|_| crate::TypedError::InternalConfig {
            detail: "mapped runtime provider returned an invalid observation".to_owned(),
        })?;
    Ok(RuntimeLifecycleInvocation::Converged)
}

pub(crate) async fn probe_startup_registry(
    registry: &StartupProviderRegistry,
    artifact: &ProviderRegistryV2,
) -> Result<(), ProviderCompositionError> {
    let Some(registry) = registry.registry() else {
        return if artifact.providers.is_empty() {
            Ok(())
        } else {
            Err(ProviderCompositionError::StartupProbeFailed)
        };
    };
    if artifact.providers.is_empty() {
        return Err(ProviderCompositionError::StartupProbeFailed);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProviderCompositionError::StartupProbeFailed)?
        .as_millis()
        .try_into()
        .map_err(|_| ProviderCompositionError::StartupProbeFailed)?;
    for entry in &artifact.providers {
        let realm_id = entry.descriptor.placement.realm_id();
        let binding = entry
            .binding
            .consumer_view()
            .map_err(|_| ProviderCompositionError::UnsupportedBinding)?;
        let (method, scope, target) = match binding {
            ProviderBindingV2ConsumerView::LocalRuntime(binding) => (
                ProviderMethod::RuntimeInspect,
                AuthorizedProviderScope::Workload {
                    realm_id: realm_id.clone(),
                    workload_id: binding.workload_id.clone(),
                },
                ProviderTarget::Workload {
                    realm_id: realm_id.clone(),
                    workload_id: binding.workload_id.clone(),
                },
            ),
            ProviderBindingV2ConsumerView::LocalObservability(_) => (
                ProviderMethod::ObservabilityStatus,
                AuthorizedProviderScope::Realm {
                    realm_id: realm_id.clone(),
                },
                ProviderTarget::Realm {
                    realm_id: realm_id.clone(),
                },
            ),
            _ => return Err(ProviderCompositionError::UnsupportedBinding),
        };
        let ProviderPlacement::TrustedFirstPartyInProcess {
            controller_role, ..
        } = &entry.descriptor.placement
        else {
            return Err(ProviderCompositionError::StartupProbeFailed);
        };
        let controller_role = *controller_role;
        let operation = ProviderOperationContext {
            schema_version: PROVIDER_SCHEMA_VERSION,
            operation_id: OperationId::parse(format!(
                "startup-{}",
                entry.descriptor.provider_id.as_str()
            ))
            .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            idempotency_key: IdempotencyKey::parse(format!(
                "startup-{}",
                entry.descriptor.provider_id.as_str()
            ))
            .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            request_digest: entry.descriptor.configured_scope_digest.clone(),
            scope,
            principal: PrincipalRef::parse("daemon-startup")
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            provider_id: entry.descriptor.provider_id.clone(),
            provider_type: entry.descriptor.provider_type(),
            provider_generation: entry.descriptor.registry_generation,
            capability: ProviderCapability(method),
            method,
            policy_epoch: Generation::new(1)
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            authorization_decision_digest: entry.descriptor.configured_scope_digest.clone(),
            issued_at_unix_ms: now,
            expires_at_unix_ms: now
                .checked_add(30_000)
                .ok_or(ProviderCompositionError::StartupProbeFailed)?,
            correlation_id: CorrelationId::parse(format!(
                "startup-{}",
                entry.descriptor.provider_id.as_str()
            ))
            .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            trace_id: Fingerprint::parse("0".repeat(64))
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
        };
        let call = ProviderCallContext {
            operation: &operation,
            peer_role: controller_role,
            service: ServicePackage::ProviderV2,
            monotonic_deadline_remaining_ms: 30_000,
            cancelled: false,
        };
        let instance = registry
            .instance(&entry.descriptor.provider_id)
            .ok_or(ProviderCompositionError::StartupProbeFailed)?;
        let health = instance
            .provider()
            .health(&call)
            .await
            .map_err(|_| ProviderCompositionError::StartupProbeFailed)?;
        health
            .validate()
            .map_err(|_| ProviderCompositionError::StartupProbeFailed)?;
        if !health.admits_operations() {
            return Err(ProviderCompositionError::StartupProbeFailed);
        }
        let request = ProviderOperationRequest {
            context: operation.clone(),
            target,
            expected_configuration_fingerprint: entry
                .descriptor
                .configuration_schema_fingerprint
                .clone(),
            input: ProviderOperationInput::NoInput,
        };
        match (instance, binding) {
            (
                ProviderInstance::Runtime(runtime),
                ProviderBindingV2ConsumerView::LocalRuntime(_),
            ) => runtime
                .inspect(&call, &request)
                .await
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?
                .validate()
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            (
                ProviderInstance::Observability(observability),
                ProviderBindingV2ConsumerView::LocalObservability(_),
            ) => observability
                .status(&call, &request)
                .await
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?
                .validate()
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            (
                _,
                ProviderBindingV2ConsumerView::LocalRuntime(_)
                | ProviderBindingV2ConsumerView::LocalObservability(_),
            ) => return Err(ProviderCompositionError::StartupProbeFailed),
            _ => return Err(ProviderCompositionError::UnsupportedBinding),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use d2b_contracts::v2_provider::{
        ImplementationId, ProviderAuthority, ProviderCapability, ProviderCapabilitySet,
        ProviderMethod,
    };
    use d2b_provider_runtime_local::{
        RuntimeAdoptionControl, RuntimeAdoptionOutcome, RuntimeBundleIntentId,
        RuntimeConfiguredItemControl, RuntimeControlContext, RuntimeControlError,
        RuntimeControlPort, RuntimeEnsureControl, RuntimeHealth, RuntimeMutationOutcome,
        RuntimeObservedState, RuntimeOperationControl, RuntimePlanDecision, RuntimeRunnerId,
        live_runtime_capabilities,
    };
    use d2b_provider_toolkit::Fixture;

    use super::*;

    struct UnavailableRuntimeControl;

    #[async_trait]
    impl RuntimeControlPort for UnavailableRuntimeControl {
        async fn health(
            &self,
            _context: RuntimeControlContext,
        ) -> Result<RuntimeHealth, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }

        async fn plan(
            &self,
            _request: RuntimeOperationControl,
        ) -> Result<RuntimePlanDecision, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }

        async fn ensure(
            &self,
            _request: RuntimeEnsureControl,
        ) -> Result<d2b_provider_runtime_local::RuntimeResourceIdentity, RuntimeControlError>
        {
            Err(RuntimeControlError::Unavailable)
        }

        async fn start(
            &self,
            _request: RuntimeOperationControl,
        ) -> Result<RuntimeObservedState, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }

        async fn stop(
            &self,
            _request: RuntimeOperationControl,
        ) -> Result<RuntimeObservedState, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }

        async fn inspect(
            &self,
            _request: RuntimeOperationControl,
        ) -> Result<RuntimeObservedState, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }

        async fn adopt(
            &self,
            _request: RuntimeAdoptionControl,
        ) -> Result<RuntimeAdoptionOutcome, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }

        async fn destroy(
            &self,
            _request: RuntimeOperationControl,
        ) -> Result<RuntimeMutationOutcome, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }

        async fn execute_configured_item(
            &self,
            _request: RuntimeConfiguredItemControl,
        ) -> Result<RuntimeObservedState, RuntimeControlError> {
            Err(RuntimeControlError::Unavailable)
        }
    }

    fn runtime_descriptor(kind: LocalRuntimeKind) -> (ProviderDescriptor, u64) {
        let fixture = Fixture::new(ProviderType::Runtime, 1).expect("fixture");
        let mut descriptor = fixture.descriptor;
        descriptor.implementation_id =
            ImplementationId::parse(kind.implementation_id()).expect("implementation");
        descriptor.authority = ProviderAuthority::Runtime {
            posture: kind.authority_posture(),
        };
        descriptor.capabilities = live_runtime_capabilities().expect("capabilities");
        descriptor.placement = ProviderPlacement::TrustedFirstPartyInProcess {
            realm_id: descriptor.placement.realm_id().clone(),
            controller_role: EndpointRole::RealmController,
        };
        descriptor.validate().expect("descriptor");
        (descriptor, fixture.now_unix_ms)
    }

    fn runtime_configuration(kind: LocalRuntimeKind) -> LocalRuntimeConfiguration {
        let intent = RuntimeBundleIntentId::parse("runtime-intent").expect("intent");
        let runner = RuntimeRunnerId::parse("runtime-runner").expect("runner");
        match kind {
            LocalRuntimeKind::CloudHypervisor => {
                LocalRuntimeConfiguration::cloud_hypervisor(intent, runner)
            }
            LocalRuntimeKind::QemuMedia => LocalRuntimeConfiguration::qemu_media(intent, runner),
            LocalRuntimeKind::SystemdUser => {
                LocalRuntimeConfiguration::systemd_user(intent, runner)
            }
        }
    }

    fn composition(
        descriptor: ProviderDescriptor,
        published_at_unix_ms: u64,
        bindings: Vec<HostProviderBinding>,
    ) -> HostProviderComposition {
        HostProviderComposition {
            generation: descriptor.registry_generation,
            configuration_fingerprint: descriptor.configuration_schema_fingerprint.clone(),
            published_at_unix_ms,
            descriptors: vec![descriptor],
            bindings,
        }
    }

    fn runtime_effects(descriptor: &ProviderDescriptor) -> DaemonEffectAdapters {
        let mut builder = DaemonEffectAdapters::builder();
        builder
            .bind_runtime(descriptor.clone(), Arc::new(UnavailableRuntimeControl))
            .expect("bind runtime");
        builder.finish().expect("effects")
    }

    #[test]
    fn lifecycle_contract_accepts_exact_900_seconds_and_rejects_901() {
        assert_eq!(MAX_PROVIDER_REQUEST_LIFETIME_MS, 900_000);
        assert!(lifecycle_budget_within_provider_contract(
            Duration::from_secs(900)
        ));
        assert!(!lifecycle_budget_within_provider_contract(
            Duration::from_secs(901)
        ));
    }

    #[test]
    fn exact_live_runtime_factories_register_once_with_matching_capabilities() {
        for kind in [
            LocalRuntimeKind::CloudHypervisor,
            LocalRuntimeKind::QemuMedia,
        ] {
            let (descriptor, now) = runtime_descriptor(kind);
            let effects = runtime_effects(&descriptor);
            let registry = compose_host_provider_registry(
                composition(
                    descriptor.clone(),
                    now,
                    vec![HostProviderBinding::LocalRuntime {
                        descriptor: descriptor.clone(),
                        configuration: runtime_configuration(kind),
                    }],
                ),
                &effects,
            )
            .expect("registry");
            let live = registry.registry().expect("live registry");
            let instance = live
                .instance(&descriptor.provider_id)
                .expect("runtime instance");
            assert_eq!(instance.descriptor(), descriptor);
            assert_eq!(instance.capabilities(), descriptor.capabilities);
            assert_eq!(live.snapshot().factories.as_slice().len(), 1);
        }
    }

    #[test]
    fn explicit_zero_row_artifact_composes_an_empty_registry() {
        let artifact = ProviderRegistryV2 {
            schema_version:
                d2b_contracts::provider_registry_v2::PROVIDER_REGISTRY_V2_SCHEMA_VERSION.to_owned(),
            registry_generation: Generation::new(1).expect("generation"),
            configuration_fingerprint: Fingerprint::parse("0".repeat(64)).expect("fingerprint"),
            published_at_unix_ms: 0,
            providers: Vec::new(),
        };
        artifact.validate().expect("valid explicit empty artifact");
        let registry = compose_host_provider_registry(
            HostProviderComposition {
                generation: artifact.registry_generation,
                configuration_fingerprint: artifact.configuration_fingerprint,
                published_at_unix_ms: artifact.published_at_unix_ms,
                descriptors: Vec::new(),
                bindings: Vec::new(),
            },
            &DaemonEffectAdapters::default(),
        )
        .expect("explicit empty registry");
        assert!(registry.is_empty());
    }

    #[test]
    fn factory_configuration_mismatch_aborts_the_transaction() {
        let (descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        let effects = runtime_effects(&descriptor);
        let result = compose_host_provider_registry(
            composition(
                descriptor.clone(),
                now,
                vec![HostProviderBinding::LocalRuntime {
                    descriptor: descriptor.clone(),
                    configuration: runtime_configuration(LocalRuntimeKind::QemuMedia),
                }],
            ),
            &effects,
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::ConfigurationMismatch)
        ));
    }

    #[test]
    fn registry_identity_mismatches_remain_exact_and_actionable() {
        let cases = [
            (
                ProviderRegistryV2Error::ProviderIdMismatch,
                ProviderCompositionError::ProviderIdMismatch,
                "provider-registry-v2 provider ID does not match descriptor placement and workload binding",
            ),
            (
                ProviderRegistryV2Error::ConfigurationSchemaFingerprintMismatch,
                ProviderCompositionError::ConfigurationSchemaFingerprintMismatch,
                "provider-registry-v2 configuration schema fingerprint does not match the first-party provider contract",
            ),
            (
                ProviderRegistryV2Error::ConfiguredScopeDigestMismatch,
                ProviderCompositionError::ConfiguredScopeDigestMismatch,
                "provider-registry-v2 configured scope digest does not match the closed provider binding",
            ),
        ];
        for (contract, composition, message) in cases {
            let mapped = map_provider_registry_validation_error(contract);
            assert_eq!(mapped, composition);
            assert_eq!(mapped.to_string(), message);
        }
    }

    #[test]
    fn unsupported_binding_error_is_explicit_and_redacted() {
        assert_eq!(
            ProviderCompositionError::UnsupportedBinding.to_string(),
            "provider binding has no registered daemon consumer adapter"
        );
    }

    #[test]
    fn unavailable_effect_mapping_does_not_register_a_descriptor() {
        let (descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        let result = compose_host_provider_registry(
            composition(
                descriptor.clone(),
                now,
                vec![HostProviderBinding::LocalRuntime {
                    descriptor,
                    configuration: runtime_configuration(LocalRuntimeKind::CloudHypervisor),
                }],
            ),
            &DaemonEffectAdapters::default(),
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::EffectAdapter(
                DaemonEffectAdapterError::MappingUnavailable
            ))
        ));
    }

    #[test]
    fn declared_provider_without_an_exact_binding_aborts_the_transaction() {
        let (descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        let result = compose_host_provider_registry(
            composition(descriptor, now, Vec::new()),
            &DaemonEffectAdapters::default(),
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::MissingBinding)
        ));
    }

    #[test]
    fn duplicate_descriptor_aborts_before_factory_construction() {
        let (descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        let effects = runtime_effects(&descriptor);
        let binding = HostProviderBinding::LocalRuntime {
            descriptor: descriptor.clone(),
            configuration: runtime_configuration(LocalRuntimeKind::CloudHypervisor),
        };
        let result = compose_host_provider_registry(
            HostProviderComposition {
                generation: descriptor.registry_generation,
                configuration_fingerprint: descriptor.configuration_schema_fingerprint.clone(),
                published_at_unix_ms: now,
                descriptors: vec![descriptor.clone(), descriptor],
                bindings: vec![binding],
            },
            &effects,
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::DuplicateDescriptor)
        ));
    }

    #[test]
    fn duplicate_configuration_binding_aborts_before_factory_construction() {
        let (descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        let effects = runtime_effects(&descriptor);
        let binding = HostProviderBinding::LocalRuntime {
            descriptor: descriptor.clone(),
            configuration: runtime_configuration(LocalRuntimeKind::CloudHypervisor),
        };
        let result = compose_host_provider_registry(
            composition(descriptor, now, vec![binding.clone(), binding]),
            &effects,
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::DuplicateBinding)
        ));
    }

    #[test]
    fn stale_descriptor_generation_aborts_before_factory_construction() {
        let (descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        let effects = runtime_effects(&descriptor);
        let result = compose_host_provider_registry(
            HostProviderComposition {
                generation: descriptor
                    .registry_generation
                    .next()
                    .expect("newer generation"),
                configuration_fingerprint: descriptor.configuration_schema_fingerprint.clone(),
                published_at_unix_ms: now,
                descriptors: vec![descriptor.clone()],
                bindings: vec![HostProviderBinding::LocalRuntime {
                    descriptor,
                    configuration: runtime_configuration(LocalRuntimeKind::CloudHypervisor),
                }],
            },
            &effects,
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::GenerationMismatch)
        ));
    }

    #[test]
    fn azure_vm_is_rejected_before_binding_resolution() {
        let (mut descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        descriptor.implementation_id =
            ImplementationId::parse(AZURE_VM_IMPLEMENTATION_ID).expect("implementation");
        let result = compose_host_provider_registry(
            composition(descriptor, now, Vec::new()),
            &DaemonEffectAdapters::default(),
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::AzureVmForbidden)
        ));
    }

    #[test]
    fn runtime_execute_capability_is_rejected_before_binding_resolution() {
        let (mut descriptor, now) = runtime_descriptor(LocalRuntimeKind::CloudHypervisor);
        let mut capabilities = descriptor.capabilities.as_slice().to_vec();
        capabilities.push(ProviderCapability(ProviderMethod::RuntimeExecute));
        descriptor.capabilities = ProviderCapabilitySet::new(capabilities).expect("capabilities");
        let result = compose_host_provider_registry(
            composition(descriptor, now, Vec::new()),
            &DaemonEffectAdapters::default(),
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::NondispatchableCapability)
        ));
    }

    #[test]
    fn host_registry_rejects_credential_descriptors() {
        let fixture = Fixture::new(ProviderType::Credential, 2).expect("fixture");
        let mut descriptor = fixture.descriptor;
        descriptor.placement = ProviderPlacement::TrustedFirstPartyInProcess {
            realm_id: descriptor.placement.realm_id().clone(),
            controller_role: EndpointRole::LocalRootController,
        };
        let result = compose_host_provider_registry(
            composition(descriptor, fixture.now_unix_ms, Vec::new()),
            &DaemonEffectAdapters::default(),
        );
        assert!(matches!(
            result,
            Err(ProviderCompositionError::WrongProcessPlacement)
        ));
    }
}
