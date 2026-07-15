use std::{
    fmt,
    os::fd::{AsFd, OwnedFd},
    sync::Arc,
};

use d2b_contracts::{
    v2_component_session::{Locality, TransportClass},
    v2_identity::{ProviderId, RealmId, WorkloadId},
    v2_provider::{AuthorizedProviderScope, Fingerprint, Generation, TransportBindingId},
};

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
    pub const fn implementation_id(self) -> &'static str {
        match self {
            Self::UnixStream => "unix-stream",
            Self::UnixSeqpacket => "unix-seqpacket",
            Self::NativeVsock => "native-vsock",
            Self::CloudHypervisorVsock => "cloud-hypervisor-vsock",
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

/// A validated, already-owned connected endpoint.
///
/// The raw descriptor is never exposed. Endpoint ports can request a
/// close-on-exec duplicate, preserving ownership of the binding copy.
#[derive(Clone)]
pub struct OwnedEndpointDescriptor {
    kind: LocalTransportKind,
    descriptor: Arc<OwnedFd>,
}

impl OwnedEndpointDescriptor {
    /// Wrap a descriptor whose owner has already validated its socket class,
    /// nonblocking mode, close-on-exec flag, identity, and authorization.
    ///
    /// The endpoint port must adapt duplicates through `AsyncFd` or an
    /// equivalent Tokio transport before performing I/O.
    pub fn from_pre_authorized(kind: LocalTransportKind, descriptor: OwnedFd) -> Self {
        Self {
            kind,
            descriptor: Arc::new(descriptor),
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
