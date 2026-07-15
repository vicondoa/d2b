use std::{
    fmt,
    num::NonZeroU32,
    os::fd::{AsFd, OwnedFd},
    sync::{Arc, LazyLock},
};

use d2b_contracts::{
    v2_component_session::{Locality, TransportClass},
    v2_identity::{ProviderId, ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AuthorizedProviderScope, Fingerprint, Generation, ImplementationId, ProviderFactoryKey,
        TransportBindingId,
    },
};

pub static UNIX_STREAM_IMPLEMENTATION_ID: LazyLock<ImplementationId> =
    LazyLock::new(|| canonical_implementation_id("unix-stream"));
pub static UNIX_SEQPACKET_IMPLEMENTATION_ID: LazyLock<ImplementationId> =
    LazyLock::new(|| canonical_implementation_id("unix-seqpacket"));
pub static NATIVE_VSOCK_IMPLEMENTATION_ID: LazyLock<ImplementationId> =
    LazyLock::new(|| canonical_implementation_id("native-vsock"));
pub static CLOUD_HYPERVISOR_VSOCK_IMPLEMENTATION_ID: LazyLock<ImplementationId> =
    LazyLock::new(|| canonical_implementation_id("cloud-hypervisor-vsock"));

pub static UNIX_STREAM_FACTORY_KEY: LazyLock<ProviderFactoryKey> =
    LazyLock::new(|| transport_factory_key(&UNIX_STREAM_IMPLEMENTATION_ID));
pub static UNIX_SEQPACKET_FACTORY_KEY: LazyLock<ProviderFactoryKey> =
    LazyLock::new(|| transport_factory_key(&UNIX_SEQPACKET_IMPLEMENTATION_ID));
pub static NATIVE_VSOCK_FACTORY_KEY: LazyLock<ProviderFactoryKey> =
    LazyLock::new(|| transport_factory_key(&NATIVE_VSOCK_IMPLEMENTATION_ID));
pub static CLOUD_HYPERVISOR_VSOCK_FACTORY_KEY: LazyLock<ProviderFactoryKey> =
    LazyLock::new(|| transport_factory_key(&CLOUD_HYPERVISOR_VSOCK_IMPLEMENTATION_ID));

/// The four local transports accepted by this implementation crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LocalTransportKind {
    UnixStream,
    UnixSeqpacket,
    NativeVsock,
    CloudHypervisorVsock,
}

impl LocalTransportKind {
    /// Canonical provider implementation ID for this transport.
    pub fn implementation_id(self) -> &'static ImplementationId {
        match self {
            Self::UnixStream => &UNIX_STREAM_IMPLEMENTATION_ID,
            Self::UnixSeqpacket => &UNIX_SEQPACKET_IMPLEMENTATION_ID,
            Self::NativeVsock => &NATIVE_VSOCK_IMPLEMENTATION_ID,
            Self::CloudHypervisorVsock => &CLOUD_HYPERVISOR_VSOCK_IMPLEMENTATION_ID,
        }
    }

    /// Canonical registry key for this transport implementation.
    pub fn factory_key(self) -> ProviderFactoryKey {
        match self {
            Self::UnixStream => (*UNIX_STREAM_FACTORY_KEY).clone(),
            Self::UnixSeqpacket => (*UNIX_SEQPACKET_FACTORY_KEY).clone(),
            Self::NativeVsock => (*NATIVE_VSOCK_FACTORY_KEY).clone(),
            Self::CloudHypervisorVsock => (*CLOUD_HYPERVISOR_VSOCK_FACTORY_KEY).clone(),
        }
    }

    /// Exact transport metadata handed to the session layer.
    pub const fn capability_profile(self) -> TransportCapabilityProfile {
        match self {
            Self::UnixStream => TransportCapabilityProfile {
                transport_class: TransportClass::UnixStream,
                locality: Locality::HostLocal,
                packet_atomic: false,
                attachments: AttachmentCapability::Disabled,
                authentication: AuthenticationOwner::ComponentSession,
            },
            Self::UnixSeqpacket => TransportCapabilityProfile {
                transport_class: TransportClass::UnixSeqpacket,
                locality: Locality::HostLocal,
                packet_atomic: true,
                attachments: AttachmentCapability::ComponentSessionNegotiatedPacketAtomic,
                authentication: AuthenticationOwner::ComponentSession,
            },
            Self::NativeVsock => TransportCapabilityProfile {
                transport_class: TransportClass::NativeVsock,
                locality: Locality::GuestLocal,
                packet_atomic: false,
                attachments: AttachmentCapability::Disabled,
                authentication: AuthenticationOwner::ComponentSession,
            },
            Self::CloudHypervisorVsock => TransportCapabilityProfile {
                transport_class: TransportClass::CloudHypervisorVsock,
                locality: Locality::GuestLocal,
                packet_atomic: false,
                attachments: AttachmentCapability::Disabled,
                authentication: AuthenticationOwner::ComponentSession,
            },
        }
    }
}

fn canonical_implementation_id(value: &str) -> ImplementationId {
    ImplementationId::parse(value)
        .unwrap_or_else(|_| unreachable!("fixed local transport implementation ID is valid"))
}

fn transport_factory_key(implementation_id: &ImplementationId) -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Transport,
        implementation_id: implementation_id.clone(),
    }
}

/// Attachment carriage advertised below `ComponentSession`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentCapability {
    Disabled,
    /// SCM_RIGHTS is available only after `ComponentSession` negotiates limits
    /// and authenticates descriptor metadata.
    ComponentSessionNegotiatedPacketAtomic,
}

/// The layer that authenticates a reachable transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationOwner {
    ComponentSession,
}

/// Exact non-secret metadata preserved from binding through connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportCapabilityProfile {
    pub transport_class: TransportClass,
    pub locality: Locality,
    pub packet_atomic: bool,
    pub attachments: AttachmentCapability,
    pub authentication: AuthenticationOwner,
}

/// Opaque ID of an endpoint declared in a verified bundle.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BundleEndpointId(TransportBindingId);

impl BundleEndpointId {
    pub fn new(id: TransportBindingId) -> Self {
        Self(id)
    }

    pub fn as_binding_id(&self) -> &TransportBindingId {
        &self.0
    }
}

impl fmt::Debug for BundleEndpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BundleEndpointId(<redacted>)")
    }
}

/// Opaque ID of an endpoint held under an allocator or broker lease.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndpointLeaseId(TransportBindingId);

impl EndpointLeaseId {
    pub fn new(id: TransportBindingId) -> Self {
        Self(id)
    }

    pub fn as_binding_id(&self) -> &TransportBindingId {
        &self.0
    }
}

impl fmt::Debug for EndpointLeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EndpointLeaseId(<redacted>)")
    }
}

/// Validation failures for an already-owned endpoint descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedEndpointError {
    DescriptorIo,
}

impl fmt::Display for OwnedEndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DescriptorIo => "owned endpoint descriptor validation failed",
        })
    }
}

impl std::error::Error for OwnedEndpointError {}

/// Nonzero guest port used by the Cloud Hypervisor `CONNECT` prelude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudHypervisorVsockPort(NonZeroU32);

impl CloudHypervisorVsockPort {
    pub const fn new(port: u32) -> Option<Self> {
        match NonZeroU32::new(port) {
            Some(port) => Some(Self(port)),
            None => None,
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// A validated, already-owned connected endpoint.
///
/// The raw descriptor is never exposed. Endpoint ports can request a
/// close-on-exec duplicate, preserving ownership of the binding copy.
#[derive(Clone)]
pub struct OwnedEndpointDescriptor {
    kind: LocalTransportKind,
    descriptor: Arc<OwnedFd>,
    identity: Fingerprint,
    generation: Generation,
    cloud_hypervisor_port: Option<CloudHypervisorVsockPort>,
}

impl OwnedEndpointDescriptor {
    /// Wrap a descriptor whose owner has already validated its socket class,
    /// nonblocking mode, close-on-exec flag, identity, and authorization.
    ///
    /// The endpoint port must adapt duplicates through `AsyncFd` or an
    /// equivalent Tokio transport before performing I/O.
    pub fn from_pre_authorized(
        kind: LocalTransportKind,
        descriptor: OwnedFd,
        identity: Fingerprint,
        generation: Generation,
    ) -> Self {
        Self {
            kind,
            descriptor: Arc::new(descriptor),
            identity,
            generation,
            cloud_hypervisor_port: None,
        }
    }

    pub fn from_pre_authorized_cloud_hypervisor(
        descriptor: OwnedFd,
        port: CloudHypervisorVsockPort,
        identity: Fingerprint,
        generation: Generation,
    ) -> Self {
        Self {
            kind: LocalTransportKind::CloudHypervisorVsock,
            descriptor: Arc::new(descriptor),
            identity,
            generation,
            cloud_hypervisor_port: Some(port),
        }
    }

    pub const fn kind(&self) -> LocalTransportKind {
        self.kind
    }

    pub fn duplicate(&self) -> Result<OwnedFd, OwnedEndpointError> {
        self.descriptor
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| OwnedEndpointError::DescriptorIo)
    }

    pub fn identity(&self) -> &Fingerprint {
        &self.identity
    }

    pub const fn generation(&self) -> Generation {
        self.generation
    }

    pub const fn cloud_hypervisor_port(&self) -> Option<CloudHypervisorVsockPort> {
        self.cloud_hypervisor_port
    }
}

impl fmt::Debug for OwnedEndpointDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedEndpointDescriptor")
            .field("kind", &self.kind)
            .field("descriptor", &"<redacted>")
            .finish()
    }
}

/// Provenance category visible to an endpoint port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointProvenance {
    VerifiedBundle,
    AuthorizedLease,
    OwnedDescriptor,
}

/// Closed endpoint source. There is intentionally no path, URL, CID/port pair,
/// or free-form endpoint string variant.
#[derive(Clone)]
pub enum EndpointSource {
    Bundle {
        kind: LocalTransportKind,
        endpoint_id: BundleEndpointId,
    },
    Lease {
        kind: LocalTransportKind,
        lease_id: EndpointLeaseId,
    },
    Owned(OwnedEndpointDescriptor),
}

impl EndpointSource {
    pub fn bundle(kind: LocalTransportKind, endpoint_id: BundleEndpointId) -> Self {
        Self::Bundle { kind, endpoint_id }
    }

    pub fn lease(kind: LocalTransportKind, lease_id: EndpointLeaseId) -> Self {
        Self::Lease { kind, lease_id }
    }

    pub fn owned(descriptor: OwnedEndpointDescriptor) -> Self {
        Self::Owned(descriptor)
    }

    pub const fn kind(&self) -> LocalTransportKind {
        match self {
            Self::Bundle { kind, .. } | Self::Lease { kind, .. } => *kind,
            Self::Owned(descriptor) => descriptor.kind(),
        }
    }

    pub const fn provenance(&self) -> EndpointProvenance {
        match self {
            Self::Bundle { .. } => EndpointProvenance::VerifiedBundle,
            Self::Lease { .. } => EndpointProvenance::AuthorizedLease,
            Self::Owned(_) => EndpointProvenance::OwnedDescriptor,
        }
    }

    pub fn bundle_id(&self) -> Option<&BundleEndpointId> {
        match self {
            Self::Bundle { endpoint_id, .. } => Some(endpoint_id),
            Self::Lease { .. } | Self::Owned(_) => None,
        }
    }

    pub fn lease_id(&self) -> Option<&EndpointLeaseId> {
        match self {
            Self::Lease { lease_id, .. } => Some(lease_id),
            Self::Bundle { .. } | Self::Owned(_) => None,
        }
    }

    pub fn owned_descriptor(&self) -> Option<&OwnedEndpointDescriptor> {
        match self {
            Self::Owned(descriptor) => Some(descriptor),
            Self::Bundle { .. } | Self::Lease { .. } => None,
        }
    }
}

impl fmt::Debug for EndpointSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointSource")
            .field("kind", &self.kind())
            .field("provenance", &self.provenance())
            .finish_non_exhaustive()
    }
}

/// One pre-authorized local endpoint binding.
#[derive(Clone)]
pub struct TransportBinding {
    binding_id: TransportBindingId,
    provider_id: ProviderId,
    provider_generation: Generation,
    configuration_fingerprint: Fingerprint,
    configured_scope_digest: Fingerprint,
    scope: AuthorizedProviderScope,
    endpoint_identity: Fingerprint,
    endpoint_generation: Generation,
    endpoint: EndpointSource,
    capabilities: TransportCapabilityProfile,
}

impl TransportBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        binding_id: TransportBindingId,
        provider_id: ProviderId,
        provider_generation: Generation,
        configuration_fingerprint: Fingerprint,
        configured_scope_digest: Fingerprint,
        scope: AuthorizedProviderScope,
        endpoint_identity: Fingerprint,
        endpoint_generation: Generation,
        endpoint: EndpointSource,
    ) -> Self {
        let capabilities = endpoint.kind().capability_profile();
        Self {
            binding_id,
            provider_id,
            provider_generation,
            configuration_fingerprint,
            configured_scope_digest,
            scope,
            endpoint_identity,
            endpoint_generation,
            endpoint,
            capabilities,
        }
    }

    pub fn binding_id(&self) -> &TransportBindingId {
        &self.binding_id
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub const fn provider_generation(&self) -> Generation {
        self.provider_generation
    }

    pub fn configuration_fingerprint(&self) -> &Fingerprint {
        &self.configuration_fingerprint
    }

    pub fn configured_scope_digest(&self) -> &Fingerprint {
        &self.configured_scope_digest
    }

    pub fn scope(&self) -> &AuthorizedProviderScope {
        &self.scope
    }

    pub fn endpoint_identity(&self) -> &Fingerprint {
        &self.endpoint_identity
    }

    pub const fn endpoint_generation(&self) -> Generation {
        self.endpoint_generation
    }

    pub fn endpoint(&self) -> &EndpointSource {
        &self.endpoint
    }

    pub const fn kind(&self) -> LocalTransportKind {
        self.endpoint.kind()
    }

    pub const fn capabilities(&self) -> TransportCapabilityProfile {
        self.capabilities
    }

    pub fn realm_id(&self) -> &RealmId {
        self.scope.realm_id()
    }

    pub fn workload_id(&self) -> Option<&WorkloadId> {
        self.scope.workload_id()
    }
}

impl fmt::Debug for TransportBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportBinding")
            .field("provider_generation", &self.provider_generation)
            .field("endpoint_generation", &self.endpoint_generation)
            .field("kind", &self.kind())
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}
