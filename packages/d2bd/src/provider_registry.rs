//! Transactional first-party provider registry composition.
//!
//! Host composition accepts only exact first-party host descriptors and
//! descriptor-bound daemon effects. Credential-bearing implementations are
//! available only through the separate provider-agent composer.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    provider_registry_v2::{ProviderBindingV2, ProviderRegistryV2},
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        AuthorizedProviderScope, CorrelationId, Fingerprint, Generation, IdempotencyKey,
        OperationId, PROVIDER_SCHEMA_VERSION, PrincipalRef, ProviderCallContext,
        ProviderCapability, ProviderDescriptor, ProviderFactoryKey, ProviderMethod,
        ProviderOperationContext, ProviderOperationInput, ProviderOperationRequest,
        ProviderPlacement, ProviderTarget,
    },
};
use d2b_provider::{
    FactoryError, ProviderFactory, ProviderInstance, ProviderRegistry, ProviderRegistryBuilder,
    RegistryBuildError, provider_capabilities_are_dispatchable,
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

use crate::{
    ServerState, load_bundle_resolver,
    provider_effects::{DaemonEffectAdapterError, DaemonEffectAdapters},
};

const AZURE_VM_IMPLEMENTATION_ID: &str = "azure-vm";

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
    AzureVmForbidden,
    NondispatchableCapability,
    ConfigurationMismatch,
    EffectAdapter(DaemonEffectAdapterError),
    Factory(FactoryError),
    Registry(RegistryBuildError),
    StartupProbeFailed,
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
            Self::AzureVmForbidden => "Azure VM providers are not production implementations",
            Self::NondispatchableCapability => {
                "provider advertises a method without a live dispatcher"
            }
            Self::ConfigurationMismatch => {
                "provider factory rejected its exact configuration binding"
            }
            Self::EffectAdapter(error) => return error.fmt(formatter),
            Self::Factory(error) => return error.fmt(formatter),
            Self::Registry(error) => return error.fmt(formatter),
            Self::StartupProbeFailed => "provider registry startup health/inspect probe failed",
        })
    }
}

impl Error for ProviderCompositionError {}

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
    Live(ProviderRegistry),
}

impl StartupProviderRegistry {
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn registry(&self) -> Option<&ProviderRegistry> {
        match self {
            Self::Empty => None,
            Self::Live(registry) => Some(registry),
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
    .map(StartupProviderRegistry::Live)
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
    .map(StartupProviderRegistry::Live)
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

pub(crate) fn load_provider_registry_v2(
    state: &ServerState,
) -> Result<ProviderRegistryV2, ProviderCompositionError> {
    let resolver =
        load_bundle_resolver(state).map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    let bytes = resolver
        .provider_registry_v2_bytes()
        .ok_or(ProviderCompositionError::ArtifactMissing)?;
    let artifact: ProviderRegistryV2 =
        serde_json::from_slice(bytes).map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    artifact
        .validate()
        .map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
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
    let bytes = resolver
        .provider_registry_v2_bytes()
        .ok_or(ProviderCompositionError::ArtifactMissing)?;
    let artifact: ProviderRegistryV2 =
        serde_json::from_slice(bytes).map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    artifact
        .validate()
        .map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    Ok(artifact)
}

pub(crate) fn compose_startup_registry(
    state: &Arc<ServerState>,
    artifact: &ProviderRegistryV2,
) -> Result<StartupProviderRegistry, ProviderCompositionError> {
    artifact
        .validate()
        .map_err(|_| ProviderCompositionError::ArtifactMalformed)?;
    let effects =
        DaemonEffectAdapters::for_server_state(Arc::downgrade(state), &artifact.providers)?;
    let bindings = artifact
        .providers
        .iter()
        .map(|entry| match &entry.binding {
            ProviderBindingV2::LocalRuntime(binding) => {
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
                    QEMU_MEDIA_IMPLEMENTATION_ID => LocalRuntimeConfiguration::QemuMedia(intent),
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
        let ProviderBindingV2::LocalRuntime(binding) = &entry.binding;
        let method = ProviderMethod::RuntimeInspect;
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
            scope: AuthorizedProviderScope::Workload {
                realm_id: binding.realm_id.clone(),
                workload_id: binding.workload_id.clone(),
            },
            principal: PrincipalRef::parse("daemon-startup")
                .map_err(|_| ProviderCompositionError::StartupProbeFailed)?,
            provider_id: entry.descriptor.provider_id.clone(),
            provider_type: ProviderType::Runtime,
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
            peer_role: EndpointRole::RealmController,
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
            target: ProviderTarget::Workload {
                realm_id: binding.realm_id.clone(),
                workload_id: binding.workload_id.clone(),
            },
            expected_configuration_fingerprint: entry
                .descriptor
                .configuration_schema_fingerprint
                .clone(),
            input: ProviderOperationInput::NoInput,
        };
        let ProviderInstance::Runtime(runtime) = instance else {
            return Err(ProviderCompositionError::StartupProbeFailed);
        };
        runtime
            .inspect(&call, &request)
            .await
            .map_err(|_| ProviderCompositionError::StartupProbeFailed)?
            .validate()
            .map_err(|_| ProviderCompositionError::StartupProbeFailed)?;
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
