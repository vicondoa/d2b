//! Authenticated `d2b.broker.v2` service over ComponentSession.

use std::{
    collections::BTreeMap,
    fmt,
    os::fd::{AsFd, AsRawFd, OwnedFd},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
        AttachmentPolicy, AttachmentPurpose, BoundedVec, EndpointPolicy, EndpointPurpose,
        EndpointRole, IdentityEvidenceRequirement, KernelObjectType, LimitProfile, Locality,
        NoiseProfile, PurposeClass, RequestId, ServicePackage, TransportBinding, TransportClass,
    },
    v2_services::{
        SERVICE_INVENTORY, StrictWireMessage, admit_metadata,
        broker::{
            AllocateRequest, AllocateResponse, SpawnRealmChildrenRequest,
            SpawnRealmChildrenResponse,
        },
        broker_ttrpc::{BrokerService, create_broker_service},
        common::{self, ServiceRequest, ServiceResponse},
        service_schema_fingerprint,
    },
};
use d2b_host::realm_children::{RealmChildBootstrapEndpoint, RealmChildBootstrapEndpoints};
use d2b_session::{
    Cancellation, ComponentSessionDriver, HandshakeCredentials, OwnedAttachment, SessionEngine,
};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, DescriptorPolicy, DescriptorPolicyResolver, ObjectIdentity,
    OwnedUnixAttachment, PeerIdentityPolicy, SeqpacketSocket, UnixAttachmentPayload,
    UnixSeqpacketTransport, UnixSessionError,
};
use futures::stream;
use protobuf::Enum;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use ttrpc::proto::{MESSAGE_HEADER_LENGTH, MessageHeader};

use crate::allocator_service::{AllocatedResourceBackend, AllocatorServiceError, ServiceReply};
use d2b_realm_core::allocator::{AllocatorLease, GrantedHostResource};

const MAX_BROKER_REQUEST_LIFETIME_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerPeerRole {
    LocalRootController,
    RealmController,
}

impl BrokerPeerRole {
    pub fn endpoint_role(self) -> EndpointRole {
        match self {
            Self::LocalRootController => EndpointRole::LocalRootController,
            Self::RealmController => EndpointRole::RealmController,
        }
    }

    fn permits(self, method: BrokerMethod) -> bool {
        self == Self::LocalRootController
            || !matches!(method, BrokerMethod::Allocate | BrokerMethod::Spawn)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerMethod {
    ValidateLease,
    Allocate,
    Delegate,
    Spawn,
    OpenResource,
    Apply,
    Observe,
    RevokeLease,
    ExportAudit,
}

impl BrokerMethod {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ValidateLease => "ValidateLease",
            Self::Allocate => "Allocate",
            Self::Delegate => "Delegate",
            Self::Spawn => "Spawn",
            Self::OpenResource => "OpenResource",
            Self::Apply => "Apply",
            Self::Observe => "Observe",
            Self::RevokeLease => "RevokeLease",
            Self::ExportAudit => "ExportAudit",
        }
    }
}

pub struct BrokerReply<T> {
    pub message: T,
    pub attachments: Vec<OwnedAttachment>,
}

impl<T> BrokerReply<T> {
    pub fn message(message: T) -> Self {
        Self {
            message,
            attachments: Vec::new(),
        }
    }
}

impl<T> fmt::Debug for BrokerReply<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrokerReply")
            .field("message", &"<redacted>")
            .field("attachment_count", &self.attachments.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerServiceFailure {
    InvalidRequest,
    PermissionDenied,
    GenerationMismatch,
    NotFound,
    Conflict,
    DeadlineExceeded,
    Cancelled,
    ResourceExhausted,
    AttachmentMismatch,
    Backend,
}

impl fmt::Display for BrokerServiceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "broker-request-invalid",
            Self::PermissionDenied => "broker-admission-denied",
            Self::GenerationMismatch => "broker-generation-mismatch",
            Self::NotFound => "broker-resource-not-found",
            Self::Conflict => "broker-operation-conflict",
            Self::DeadlineExceeded => "broker-deadline-exceeded",
            Self::Cancelled => "broker-request-cancelled",
            Self::ResourceExhausted => "broker-resource-exhausted",
            Self::AttachmentMismatch => "broker-attachment-mismatch",
            Self::Backend => "broker-operation-failed",
        })
    }
}

impl std::error::Error for BrokerServiceFailure {}

pub struct BrokerCallContext {
    pub peer_role: BrokerPeerRole,
    pub request_id: RequestId,
    pub session_generation: u64,
    pub remaining: Duration,
    pub cancellation: Cancellation,
}

impl fmt::Debug for BrokerCallContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrokerCallContext")
            .field("peer_role", &self.peer_role)
            .field("request_id", &"<redacted>")
            .field("session_generation", &"<redacted>")
            .field("remaining", &self.remaining)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish()
    }
}

#[async_trait]
pub trait BrokerRuntimeDispatch: Send + Sync {
    async fn dispatch(
        &self,
        method: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure>;
}

#[async_trait]
pub trait AllocatorServiceDispatch: Send {
    async fn allocate(
        &mut self,
        request: &AllocateRequest,
    ) -> Result<ServiceReply<AllocateResponse>, AllocatorServiceError>;

    async fn spawn(
        &self,
        request: &SpawnRealmChildrenRequest,
        attachments: Vec<OwnedFd>,
        bootstrap: RealmChildBootstrapEndpoints,
    ) -> Result<
        ServiceReply<
            SpawnRealmChildrenResponse,
            crate::allocator_service::PolicyBoundPidfdAttachments,
        >,
        AllocatorServiceError,
    >;
}

#[derive(Clone)]
struct AuthorizedDescriptor {
    fd: Arc<OwnedFd>,
    policy: DescriptorPolicy,
    object_type: KernelObjectType,
    access: AttachmentAccess,
    purpose: AttachmentPurpose,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SpawnBinding {
    role: i32,
    kind: i32,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PendingAttachment {
    request_id: Vec<u8>,
    generation: u64,
    index: u16,
}

#[derive(Default)]
struct AuthorizedFdState {
    resources: BTreeMap<String, AuthorizedDescriptor>,
    singleton_bindings: BTreeMap<SpawnBinding, AuthorizedDescriptor>,
    bootstrap_endpoints: BTreeMap<i32, RealmChildBootstrapEndpoint>,
    pending: BTreeMap<PendingAttachment, AuthorizedDescriptor>,
}

#[derive(Default)]
pub struct AuthorizedFdRegistry {
    state: Mutex<AuthorizedFdState>,
}

impl fmt::Debug for AuthorizedFdRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizedFdRegistry(REDACTED)")
    }
}

impl AuthorizedFdRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_resource(
        &self,
        resource_id: impl Into<String>,
        fd: OwnedFd,
        object_type: KernelObjectType,
        access: AttachmentAccess,
        purpose: AttachmentPurpose,
    ) -> Result<(), BrokerServiceFailure> {
        let resource_id = resource_id.into();
        if resource_id.is_empty() {
            return Err(BrokerServiceFailure::InvalidRequest);
        }
        let descriptor = authorized_descriptor(fd, object_type, access, purpose)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| BrokerServiceFailure::Backend)?;
        if state.resources.contains_key(&resource_id) {
            return Err(BrokerServiceFailure::AttachmentMismatch);
        }
        state.resources.insert(resource_id, descriptor);
        Ok(())
    }

    pub fn register_spawn_binding(
        &self,
        role: i32,
        kind: i32,
        fd: OwnedFd,
        object_type: KernelObjectType,
        access: AttachmentAccess,
        purpose: AttachmentPurpose,
    ) -> Result<(), BrokerServiceFailure> {
        if kind
            == d2b_contracts::v2_services::broker::RealmChildFdKind::
                REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION
                .value()
        {
            return Err(BrokerServiceFailure::InvalidRequest);
        }
        let binding = SpawnBinding { role, kind };
        let descriptor = authorized_descriptor(fd, object_type, access, purpose)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| BrokerServiceFailure::Backend)?;
        if state.singleton_bindings.contains_key(&binding) {
            return Err(BrokerServiceFailure::AttachmentMismatch);
        }
        state.singleton_bindings.insert(binding, descriptor);
        Ok(())
    }

    pub fn register_spawn_bootstrap_pair(
        &self,
        role: i32,
        child_fd: OwnedFd,
        parent_fd: OwnedFd,
    ) -> Result<(), BrokerServiceFailure> {
        use d2b_contracts::v2_services::broker::{RealmChildFdKind, RealmChildRole};

        if role != RealmChildRole::REALM_CHILD_ROLE_CONTROLLER.value()
            && role != RealmChildRole::REALM_CHILD_ROLE_BROKER.value()
        {
            return Err(BrokerServiceFailure::InvalidRequest);
        }
        let endpoint = RealmChildBootstrapEndpoint::from_parent_prearmed(parent_fd, &child_fd)
            .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?;
        let descriptor = authorized_descriptor(
            child_fd,
            KernelObjectType::UnixSeqpacketSocket,
            AttachmentAccess::ReadWrite,
            AttachmentPurpose::RequestInput,
        )?;
        let binding = SpawnBinding {
            role,
            kind: RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION.value(),
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| BrokerServiceFailure::Backend)?;
        if state.singleton_bindings.contains_key(&binding)
            || state.bootstrap_endpoints.contains_key(&role)
        {
            return Err(BrokerServiceFailure::AttachmentMismatch);
        }
        state.singleton_bindings.insert(binding, descriptor);
        state.bootstrap_endpoints.insert(role, endpoint);
        Ok(())
    }

    fn take_bootstrap_endpoints(
        &self,
    ) -> Result<RealmChildBootstrapEndpoints, BrokerServiceFailure> {
        use d2b_contracts::v2_services::broker::RealmChildRole;

        let controller_role = RealmChildRole::REALM_CHILD_ROLE_CONTROLLER.value();
        let broker_role = RealmChildRole::REALM_CHILD_ROLE_BROKER.value();
        let bootstrap_kind =
            d2b_contracts::v2_services::broker::RealmChildFdKind::
                REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION
                .value();
        let controller_binding = SpawnBinding {
            role: controller_role,
            kind: bootstrap_kind,
        };
        let broker_binding = SpawnBinding {
            role: broker_role,
            kind: bootstrap_kind,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| BrokerServiceFailure::Backend)?;
        if !state.bootstrap_endpoints.contains_key(&controller_role)
            || !state.bootstrap_endpoints.contains_key(&broker_role)
            || !state.singleton_bindings.contains_key(&controller_binding)
            || !state.singleton_bindings.contains_key(&broker_binding)
        {
            return Err(BrokerServiceFailure::PermissionDenied);
        }
        let controller = state
            .bootstrap_endpoints
            .remove(&controller_role)
            .ok_or(BrokerServiceFailure::Backend)?;
        let broker = state
            .bootstrap_endpoints
            .remove(&broker_role)
            .ok_or(BrokerServiceFailure::Backend)?;
        state.singleton_bindings.remove(&controller_binding);
        state.singleton_bindings.remove(&broker_binding);
        Ok(RealmChildBootstrapEndpoints { controller, broker })
    }

    fn remember_allocated_resource(
        &self,
        resource_id: &str,
        fd: &OwnedFd,
        shape: DescriptorShape,
    ) -> Result<(), BrokerServiceFailure> {
        let identity = ObjectIdentity::from_trusted(fd, shape.object_type, shape.access)
            .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?;
        {
            let state = self
                .state
                .lock()
                .map_err(|_| BrokerServiceFailure::Backend)?;
            if let Some(existing) = state.resources.get(resource_id) {
                return match &existing.policy {
                    DescriptorPolicy::File(expected)
                        if expected == &identity
                            && existing.object_type == shape.object_type
                            && existing.access == shape.access =>
                    {
                        Ok(())
                    }
                    _ => Err(BrokerServiceFailure::AttachmentMismatch),
                };
            }
        }
        self.register_resource(
            resource_id,
            fd.try_clone().map_err(|_| BrokerServiceFailure::Backend)?,
            shape.object_type,
            shape.access,
            AttachmentPurpose::DeviceLease,
        )
    }

    fn clone_resource(&self, resource_id: &str) -> Result<Option<OwnedFd>, AllocatorServiceError> {
        let state = self.state.lock().map_err(|_| {
            AllocatorServiceError::Invariant("authorized descriptor registry poisoned")
        })?;
        state
            .resources
            .get(resource_id)
            .map(|descriptor| descriptor.fd.try_clone())
            .transpose()
            .map_err(AllocatorServiceError::Descriptor)
    }

    fn authorize_spawn(
        &self,
        request: &SpawnRealmChildrenRequest,
        context: &BrokerCallContext,
    ) -> Result<(), BrokerServiceFailure> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| BrokerServiceFailure::Backend)?;
        let mut pending = Vec::with_capacity(request.fds.len());
        for binding in &request.fds {
            let authorized = match binding.resource_id.as_deref() {
                Some(resource_id) => state.resources.get(resource_id),
                None => state.singleton_bindings.get(&SpawnBinding {
                    role: binding.role.value(),
                    kind: binding.kind.value(),
                }),
            }
            .cloned()
            .ok_or(BrokerServiceFailure::PermissionDenied)?;
            pending.push((
                PendingAttachment {
                    request_id: context.request_id.as_bytes().to_vec(),
                    generation: context.session_generation,
                    index: binding
                        .attachment_index
                        .try_into()
                        .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?,
                },
                authorized,
            ));
        }
        if pending
            .iter()
            .any(|(key, _)| state.pending.contains_key(key))
        {
            return Err(BrokerServiceFailure::AttachmentMismatch);
        }
        state.pending.extend(pending);
        Ok(())
    }

    fn finish_spawn(&self, context: &BrokerCallContext, request: &SpawnRealmChildrenRequest) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for binding in &request.fds {
            state.pending.remove(&PendingAttachment {
                request_id: context.request_id.as_bytes().to_vec(),
                generation: context.session_generation,
                index: match binding.attachment_index.try_into() {
                    Ok(index) => index,
                    Err(_) => continue,
                },
            });
        }
    }

    pub fn resolver(self: &Arc<Self>) -> DescriptorPolicyResolver {
        let registry = Arc::clone(self);
        Arc::new(move |descriptor: &AttachmentDescriptor| registry.resolve(descriptor))
    }

    fn resolve(
        &self,
        descriptor: &AttachmentDescriptor,
    ) -> Result<DescriptorPolicy, UnixSessionError> {
        let state = self
            .state
            .lock()
            .map_err(|_| UnixSessionError::DescriptorMismatch)?;
        let authorized = state
            .pending
            .get(&PendingAttachment {
                request_id: descriptor.request_id.as_bytes().to_vec(),
                generation: descriptor.reconnect_generation,
                index: descriptor.index,
            })
            .ok_or(UnixSessionError::DescriptorMismatch)?;
        if descriptor.service != ServicePackage::BrokerV2
            || descriptor.method_id != broker_method_id(BrokerMethod::Spawn)
            || descriptor.object_type != authorized.object_type
            || descriptor.access != authorized.access
            || descriptor.purpose != authorized.purpose
            || descriptor.kind != AttachmentKind::FileDescriptor
            || !descriptor.cloexec_required
        {
            return Err(UnixSessionError::DescriptorMismatch);
        }
        let _ = authorized.fd.as_fd();
        Ok(authorized.policy.clone())
    }
}

#[derive(Clone)]
pub struct AuthorizedResourceBackend {
    registry: Arc<AuthorizedFdRegistry>,
}

impl AuthorizedResourceBackend {
    pub fn new(registry: Arc<AuthorizedFdRegistry>) -> Self {
        Self { registry }
    }
}

impl AllocatedResourceBackend for AuthorizedResourceBackend {
    fn materialize(
        &mut self,
        _: &AllocatorLease,
        resource: &GrantedHostResource,
    ) -> Result<Option<OwnedFd>, AllocatorServiceError> {
        self.registry.clone_resource(resource.resource_id.as_str())
    }
}

pub(crate) struct DescriptorShape {
    pub(crate) object_type: KernelObjectType,
    pub(crate) access: AttachmentAccess,
}

fn descriptor_shape(fd: &OwnedFd) -> Result<DescriptorShape, BrokerServiceFailure> {
    use nix::{
        fcntl::{FcntlArg, OFlag, fcntl},
        sys::{
            socket::{SockType, getsockopt, sockopt},
            stat::{SFlag, fstat},
        },
    };

    if std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
        .is_some_and(|path| path == "anon_inode:[pidfd]")
    {
        return Err(BrokerServiceFailure::Backend);
    }
    let stat = fstat(fd.as_raw_fd()).map_err(|_| BrokerServiceFailure::Backend)?;
    let flags = OFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL).map_err(|_| BrokerServiceFailure::Backend)?,
    );
    let access = match flags & OFlag::O_ACCMODE {
        OFlag::O_WRONLY => AttachmentAccess::WriteOnly,
        OFlag::O_RDWR => AttachmentAccess::ReadWrite,
        _ => AttachmentAccess::ReadOnly,
    };
    let file_type = SFlag::from_bits_truncate(stat.st_mode);
    let object_type = if file_type.contains(SFlag::S_IFDIR) {
        KernelObjectType::Directory
    } else if file_type.contains(SFlag::S_IFREG) {
        KernelObjectType::RegularFile
    } else if file_type.contains(SFlag::S_IFCHR) || file_type.contains(SFlag::S_IFBLK) {
        KernelObjectType::Device
    } else if file_type.contains(SFlag::S_IFSOCK) {
        match getsockopt(fd, sockopt::SockType).map_err(|_| BrokerServiceFailure::Backend)? {
            SockType::SeqPacket => KernelObjectType::UnixSeqpacketSocket,
            SockType::Stream => KernelObjectType::UnixStreamSocket,
            _ => return Err(BrokerServiceFailure::AttachmentMismatch),
        }
    } else {
        return Err(BrokerServiceFailure::AttachmentMismatch);
    };
    Ok(DescriptorShape {
        object_type,
        access,
    })
}

fn authorized_descriptor(
    fd: OwnedFd,
    object_type: KernelObjectType,
    access: AttachmentAccess,
    purpose: AttachmentPurpose,
) -> Result<AuthorizedDescriptor, BrokerServiceFailure> {
    let identity = ObjectIdentity::from_trusted(&fd, object_type, access)
        .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?;
    Ok(AuthorizedDescriptor {
        fd: Arc::new(fd),
        policy: DescriptorPolicy::File(identity),
        object_type,
        access,
        purpose,
    })
}

fn broker_method_id(method: BrokerMethod) -> u32 {
    SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2")
        .and_then(|service| {
            service
                .methods
                .iter()
                .find(|candidate| candidate.name == method.name())
        })
        .expect("generated broker service method")
        .method_id("d2b.broker.v2", "BrokerService")
}

pub(crate) fn attachment_descriptor(
    method: BrokerMethod,
    context: &BrokerCallContext,
    index: u32,
    shape: DescriptorShape,
    purpose: AttachmentPurpose,
) -> Result<AttachmentDescriptor, BrokerServiceFailure> {
    let index = u16::try_from(index).map_err(|_| BrokerServiceFailure::AttachmentMismatch)?;
    let descriptor = AttachmentDescriptor {
        index,
        kind: AttachmentKind::FileDescriptor,
        object_type: shape.object_type,
        access: shape.access,
        purpose,
        service: ServicePackage::BrokerV2,
        method_id: broker_method_id(method),
        request_id: context.request_id.clone(),
        operation_id: None,
        packet_sequence: 1,
        reconnect_generation: context.session_generation,
        duplicate_object_allowed: false,
        cloexec_required: true,
        credit_classes: BoundedVec::new(vec![
            AttachmentCreditClass::Packet,
            AttachmentCreditClass::Request,
            AttachmentCreditClass::Operation,
            AttachmentCreditClass::Session,
            AttachmentCreditClass::Process,
            AttachmentCreditClass::Host,
        ])
        .map_err(|_| BrokerServiceFailure::Backend)?,
    };
    descriptor
        .validate(index)
        .map_err(|_| BrokerServiceFailure::Backend)?;
    Ok(descriptor)
}

pub struct ProductionBrokerOperationHandler<R, A> {
    runtime: R,
    allocator: Arc<tokio::sync::Mutex<A>>,
    authorized_fds: Arc<AuthorizedFdRegistry>,
}

impl<R, A> ProductionBrokerOperationHandler<R, A> {
    pub fn new(runtime: R, allocator: A, authorized_fds: Arc<AuthorizedFdRegistry>) -> Self {
        Self {
            runtime,
            allocator: Arc::new(tokio::sync::Mutex::new(allocator)),
            authorized_fds,
        }
    }

    pub fn authorized_fds(&self) -> &Arc<AuthorizedFdRegistry> {
        &self.authorized_fds
    }
}

#[async_trait]
pub trait BrokerOperationHandler: Send + Sync {
    async fn handle(
        &self,
        method: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure>;

    async fn allocate(
        &self,
        request: AllocateRequest,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure>;

    async fn spawn(
        &self,
        request: SpawnRealmChildrenRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure>;

    async fn cancel(
        &self,
        _: common::CancelRequest,
        _: Arc<dyn ComponentSessionDriver>,
    ) -> Result<common::CancelResponse, BrokerServiceFailure> {
        Err(BrokerServiceFailure::Backend)
    }

    fn attachment_policy_resolver(&self) -> Option<DescriptorPolicyResolver> {
        None
    }

    fn prepare_spawn_attachments(
        &self,
        request: &SpawnRealmChildrenRequest,
        _: &BrokerCallContext,
    ) -> Result<(), BrokerServiceFailure> {
        if request.fds.is_empty() {
            Ok(())
        } else {
            Err(BrokerServiceFailure::AttachmentMismatch)
        }
    }

    fn finish_spawn_attachments(&self, _: &SpawnRealmChildrenRequest, _: &BrokerCallContext) {}
}

#[async_trait]
impl<R, A> BrokerOperationHandler for ProductionBrokerOperationHandler<R, A>
where
    R: BrokerRuntimeDispatch,
    A: AllocatorServiceDispatch + Sync + 'static,
{
    async fn handle(
        &self,
        method: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        self.runtime
            .dispatch(method, request, attachments, context)
            .await
    }

    async fn allocate(
        &self,
        request: AllocateRequest,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure> {
        if context.cancellation.is_cancelled() {
            return Err(BrokerServiceFailure::Cancelled);
        }
        let reply = self
            .allocator
            .lock()
            .await
            .allocate(&request)
            .await
            .map_err(|_| BrokerServiceFailure::Backend)?;
        if context.cancellation.is_cancelled() {
            return Err(BrokerServiceFailure::Cancelled);
        }
        let mut attachments = Vec::with_capacity(reply.attachments.len());
        for (index, fd) in reply.attachments.into_iter().enumerate() {
            let resource = reply
                .message
                .resources
                .iter()
                .find(|resource| resource.attachment_index == Some(index as u32))
                .ok_or(BrokerServiceFailure::Backend)?;
            let shape = descriptor_shape(&fd)?;
            self.authorized_fds
                .remember_allocated_resource(&resource.resource_id, &fd, shape)?;
            let shape = descriptor_shape(&fd)?;
            let descriptor = attachment_descriptor(
                BrokerMethod::Allocate,
                context,
                index as u32,
                shape,
                AttachmentPurpose::DeviceLease,
            )?;
            let policy = DescriptorPolicy::File(
                ObjectIdentity::from_trusted(&fd, descriptor.object_type, descriptor.access)
                    .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?,
            );
            attachments.push(
                OwnedUnixAttachment::file(descriptor, fd, policy)
                    .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?,
            );
        }
        Ok(BrokerReply {
            message: reply.message,
            attachments,
        })
    }

    async fn spawn(
        &self,
        request: SpawnRealmChildrenRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure> {
        if context.cancellation.is_cancelled() {
            return Err(BrokerServiceFailure::Cancelled);
        }
        let fds = attachments
            .iter()
            .map(|attachment| {
                attachment
                    .payload()
                    .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
                    .and_then(UnixAttachmentPayload::file)
                    .ok_or(BrokerServiceFailure::AttachmentMismatch)?
                    .try_clone_to_owned()
                    .map_err(|_| BrokerServiceFailure::Backend)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bootstrap = self.authorized_fds.take_bootstrap_endpoints()?;
        let allocator = Arc::clone(&self.allocator);
        let cancellation = context.cancellation.clone();
        let allocator_task = tokio::spawn(async move {
            let result = allocator.lock().await.spawn(&request, fds, bootstrap).await;
            if cancellation.is_cancelled() {
                if let Ok(reply) = &result {
                    for child in [reply.attachments.controller(), reply.attachments.broker()] {
                        crate::sys::pidfd_sys::pidfd_send_signal(child.as_fd(), nix::libc::SIGKILL)
                            .map_err(|_| BrokerServiceFailure::Backend)?;
                    }
                }
                return Err(BrokerServiceFailure::Cancelled);
            }
            result.map_err(|_| BrokerServiceFailure::Backend)
        });
        let reply = allocator_task
            .await
            .map_err(|_| BrokerServiceFailure::Backend)??;
        let mut response_attachments = Vec::with_capacity(2);
        for attachment in reply.attachments.into_transport_order() {
            let index = attachment.attachment_index();
            let descriptor = attachment_descriptor(
                BrokerMethod::Spawn,
                context,
                index,
                DescriptorShape {
                    object_type: KernelObjectType::Pidfd,
                    access: AttachmentAccess::ReadWrite,
                },
                AttachmentPurpose::ProcessIdentity,
            )?;
            let (fd, policy) = attachment.into_transport_parts();
            response_attachments.push(
                OwnedUnixAttachment::file(descriptor, fd, DescriptorPolicy::Pidfd(policy))
                    .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?,
            );
        }
        Ok(BrokerReply {
            message: reply.message,
            attachments: response_attachments,
        })
    }

    fn attachment_policy_resolver(&self) -> Option<DescriptorPolicyResolver> {
        Some(self.authorized_fds.resolver())
    }

    fn prepare_spawn_attachments(
        &self,
        request: &SpawnRealmChildrenRequest,
        context: &BrokerCallContext,
    ) -> Result<(), BrokerServiceFailure> {
        self.authorized_fds.authorize_spawn(request, context)
    }

    fn finish_spawn_attachments(
        &self,
        request: &SpawnRealmChildrenRequest,
        context: &BrokerCallContext,
    ) {
        self.authorized_fds.finish_spawn(context, request);
    }
}

pub struct RejectingBrokerOperations;

#[async_trait]
impl BrokerOperationHandler for RejectingBrokerOperations {
    async fn handle(
        &self,
        _: BrokerMethod,
        _: ServiceRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::Backend)
    }

    async fn allocate(
        &self,
        _: AllocateRequest,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::Backend)
    }

    async fn spawn(
        &self,
        _: SpawnRealmChildrenRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::Backend)
    }
}

pub struct BrokerServiceV2<H> {
    handler: Arc<H>,
    session: Arc<dyn ComponentSessionDriver>,
    peer_role: BrokerPeerRole,
    active: Arc<Mutex<BTreeMap<Vec<u8>, Cancellation>>>,
}

impl<H> BrokerServiceV2<H> {
    pub fn new(
        handler: Arc<H>,
        session: Arc<dyn ComponentSessionDriver>,
        peer_role: BrokerPeerRole,
    ) -> Self {
        Self {
            handler,
            session,
            peer_role,
            active: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl BrokerServiceV2<RejectingBrokerOperations> {
    pub fn fail_closed(
        session: Arc<dyn ComponentSessionDriver>,
        peer_role: BrokerPeerRole,
    ) -> Self {
        Self::new(Arc::new(RejectingBrokerOperations), session, peer_role)
    }
}

impl<H: BrokerOperationHandler> BrokerServiceV2<H> {
    async fn dispatch(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        method: BrokerMethod,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let admitted = self
            .admit(
                ttrpc_context,
                method,
                request.metadata.as_ref(),
                requires_idempotency(method),
            )
            .await?;
        request
            .validate_wire(requires_idempotency(method))
            .map_err(|_| invalid_request())?;
        let attachments = self
            .receive_attachments(
                &request.attachment_indexes,
                &admitted.context.request_id,
                method,
            )
            .await?;
        let result = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => {
                Err(cancelled())
            }
            response = tokio::time::timeout(
                admitted.context.remaining,
                self.handler.handle(method, request, attachments, &admitted.context),
            ) => {
                match response {
                    Ok(Ok(reply)) => self.finish_reply(method, &admitted.context.request_id, reply).await,
                    Ok(Err(error)) => Err(service_error(error)),
                    Err(_) => {
                        let _ = admitted.context.cancellation.cancel();
                        Err(deadline_exceeded())
                    },
                }
            }
        };
        admitted.finish(result).await
    }

    async fn dispatch_allocate(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: AllocateRequest,
    ) -> ttrpc::Result<AllocateResponse> {
        let admitted = self
            .admit(
                context,
                BrokerMethod::Allocate,
                request.metadata.as_ref(),
                true,
            )
            .await?;
        request.validate_wire(true).map_err(|_| invalid_request())?;
        let result = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => Err(cancelled()),
            response = tokio::time::timeout(
                admitted.context.remaining,
                self.handler.allocate(request, &admitted.context),
            ) => match response {
                Ok(Ok(reply)) => self.finish_reply(BrokerMethod::Allocate, &admitted.context.request_id, reply).await,
                Ok(Err(error)) => Err(service_error(error)),
                Err(_) => {
                    let _ = admitted.context.cancellation.cancel();
                    Err(deadline_exceeded())
                },
            }
        };
        admitted.finish(result).await
    }

    async fn dispatch_spawn(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: SpawnRealmChildrenRequest,
    ) -> ttrpc::Result<SpawnRealmChildrenResponse> {
        let admitted = self
            .admit(
                context,
                BrokerMethod::Spawn,
                request.metadata.as_ref(),
                true,
            )
            .await?;
        request.validate_wire(true).map_err(|_| invalid_request())?;
        if let Err(error) = self
            .handler
            .prepare_spawn_attachments(&request, &admitted.context)
        {
            return admitted.finish(Err(service_error(error))).await;
        }
        let indices: Vec<u32> = request.fds.iter().map(|fd| fd.attachment_index).collect();
        let attachments = match self
            .receive_attachments(&indices, &admitted.context.request_id, BrokerMethod::Spawn)
            .await
        {
            Ok(attachments) => attachments,
            Err(error) => {
                self.handler
                    .finish_spawn_attachments(&request, &admitted.context);
                return admitted.finish(Err(error)).await;
            }
        };
        let finish_request = request.clone();
        let result = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => Err(cancelled()),
            response = tokio::time::timeout(
                admitted.context.remaining,
                self.handler.spawn(request, attachments, &admitted.context),
            ) => match response {
                Ok(Ok(reply)) => self.finish_reply(BrokerMethod::Spawn, &admitted.context.request_id, reply).await,
                Ok(Err(error)) => Err(service_error(error)),
                Err(_) => {
                    let _ = admitted.context.cancellation.cancel();
                    Err(deadline_exceeded())
                },
            }
        };
        self.handler
            .finish_spawn_attachments(&finish_request, &admitted.context);
        admitted.finish(result).await
    }

    async fn admit(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        method: BrokerMethod,
        metadata: Option<&common::RequestMetadata>,
        requires_idempotency: bool,
    ) -> ttrpc::Result<AdmittedCall> {
        if !self.peer_role.permits(method) {
            return Err(permission_denied());
        }
        let metadata = metadata.ok_or_else(invalid_request)?;
        if metadata.session_generation != self.session.generation() {
            return Err(permission_denied());
        }
        let remaining_nanos = admit_metadata(
            metadata,
            requires_idempotency,
            now_unix_ms(),
            MAX_BROKER_REQUEST_LIFETIME_MS,
            None,
            peer_timeout(ttrpc_context),
        )
        .map_err(|_| invalid_request())?;
        let request_id =
            RequestId::new(metadata.request_id.clone()).map_err(|_| invalid_request())?;
        let cancellation = self
            .session
            .register_inbound_call(request_id.clone())
            .await
            .map_err(|_| invalid_request())?;
        self.active
            .lock()
            .map_err(|_| response_error())?
            .insert(metadata.request_id.clone(), cancellation.clone());
        Ok(AdmittedCall {
            driver: Arc::clone(&self.session),
            request_id: Some(request_id.clone()),
            request_key: metadata.request_id.clone(),
            active: Arc::clone(&self.active),
            context: BrokerCallContext {
                peer_role: self.peer_role,
                request_id,
                session_generation: self.session.generation(),
                remaining: Duration::from_nanos(remaining_nanos),
                cancellation,
            },
        })
    }

    async fn receive_attachments(
        &self,
        indices: &[u32],
        request_id: &RequestId,
        method: BrokerMethod,
    ) -> ttrpc::Result<Vec<OwnedAttachment>> {
        if indices.is_empty() {
            return Ok(Vec::new());
        }
        let attachments = self
            .session
            .receive_attachments()
            .await
            .map_err(|_| attachment_error())?;
        validate_attachment_table(
            &attachments,
            indices,
            request_id,
            self.session.generation(),
            method,
        )?;
        Ok(attachments)
    }

    async fn finish_reply<T: StrictWireMessage + ResponseAttachments>(
        &self,
        method: BrokerMethod,
        request_id: &RequestId,
        reply: BrokerReply<T>,
    ) -> ttrpc::Result<T> {
        reply
            .message
            .validate_wire(false)
            .map_err(|_| response_error())?;
        let expected = response_attachment_indices(&reply.message);
        validate_attachment_table(
            &reply.attachments,
            &expected,
            request_id,
            self.session.generation(),
            method,
        )?;
        if !reply.attachments.is_empty() {
            self.session
                .send_attachments(reply.attachments)
                .await
                .map_err(|_| attachment_error())?;
        }
        Ok(reply.message)
    }
}

#[async_trait]
impl<H: BrokerOperationHandler + 'static> BrokerService for BrokerServiceV2<H> {
    async fn validate_lease(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, BrokerMethod::ValidateLease, request)
            .await
    }

    async fn allocate(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: AllocateRequest,
    ) -> ttrpc::Result<AllocateResponse> {
        self.dispatch_allocate(context, request).await
    }

    async fn delegate(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, BrokerMethod::Delegate, request)
            .await
    }

    async fn spawn(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: SpawnRealmChildrenRequest,
    ) -> ttrpc::Result<SpawnRealmChildrenResponse> {
        self.dispatch_spawn(context, request).await
    }

    async fn open_resource(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, BrokerMethod::OpenResource, request)
            .await
    }

    async fn apply(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, BrokerMethod::Apply, request).await
    }

    async fn observe(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, BrokerMethod::Observe, request).await
    }

    async fn revoke_lease(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, BrokerMethod::RevokeLease, request)
            .await
    }

    async fn export_audit(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, BrokerMethod::ExportAudit, request)
            .await
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        request
            .validate_wire(false)
            .map_err(|_| invalid_request())?;
        let outcome = if request.session_generation != self.session.generation() {
            common::CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
        } else {
            match self
                .active
                .lock()
                .map_err(|_| response_error())?
                .get(&request.request_id)
            {
                Some(cancellation) if cancellation.cancel() => {
                    common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
                }
                Some(_) => common::CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL,
                None => common::CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
            }
        };
        let response = common::CancelResponse {
            outcome: outcome.into(),
            ..Default::default()
        };
        response
            .validate_wire(false)
            .map_err(|_| response_error())?;
        Ok(response)
    }
}

pub async fn serve_broker_session<H>(
    driver: Arc<dyn ComponentSessionDriver>,
    peer_role: BrokerPeerRole,
    handler: Arc<H>,
) -> Result<(), BrokerServiceFailure>
where
    H: BrokerOperationHandler + 'static,
{
    let service = Arc::new(BrokerServiceV2::new(
        handler,
        Arc::clone(&driver),
        peer_role,
    ));
    let (server_transport, bridge_transport) = tokio::io::duplex(
        d2b_contracts::v2_component_session::LimitProfile::local_default().logical_ttrpc_bytes
            as usize,
    );
    let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
        Ok::<_, std::io::Error>(server_transport)
    }));
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(listener)
        .register_service(create_broker_service(service));
    server
        .start()
        .await
        .map_err(|_| BrokerServiceFailure::Backend)?;
    let (mut reader, mut writer) = tokio::io::split(bridge_transport);
    let receive_driver = Arc::clone(&driver);
    let receive = async move {
        loop {
            let frame = receive_driver
                .receive_ttrpc()
                .await
                .map_err(|_| BrokerServiceFailure::Backend)?;
            writer
                .write_all(&frame)
                .await
                .map_err(|_| BrokerServiceFailure::Backend)?;
            writer
                .flush()
                .await
                .map_err(|_| BrokerServiceFailure::Backend)?;
        }

        #[allow(unreachable_code)]
        Ok::<(), BrokerServiceFailure>(())
    };
    let send = async move {
        loop {
            let Some(frame) = read_ttrpc_frame(&mut reader).await? else {
                return Ok::<(), BrokerServiceFailure>(());
            };
            driver
                .send_ttrpc(frame)
                .await
                .map_err(|_| BrokerServiceFailure::Backend)?;
        }
    };
    let result = tokio::select! {
        result = receive => result,
        result = send => result,
    };
    server.disconnect().await;
    result
}

async fn read_ttrpc_frame(
    reader: &mut tokio::io::ReadHalf<tokio::io::DuplexStream>,
) -> Result<Option<Vec<u8>>, BrokerServiceFailure> {
    let mut header = [0_u8; MESSAGE_HEADER_LENGTH];
    let first = reader
        .read(&mut header[..1])
        .await
        .map_err(|_| BrokerServiceFailure::Backend)?;
    if first == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut header[1..])
        .await
        .map_err(|_| BrokerServiceFailure::Backend)?;
    let parsed = MessageHeader::from(&header[..]);
    let body_len = usize::try_from(parsed.length).map_err(|_| BrokerServiceFailure::Backend)?;
    if body_len
        > d2b_contracts::v2_component_session::LimitProfile::local_default().logical_ttrpc_bytes
            as usize
    {
        return Err(BrokerServiceFailure::Backend);
    }
    let mut frame = Vec::with_capacity(MESSAGE_HEADER_LENGTH + body_len);
    frame.extend_from_slice(&header);
    let mut body = vec![0_u8; body_len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|_| BrokerServiceFailure::Backend)?;
    frame.extend_from_slice(&body);
    Ok(Some(frame))
}

pub async fn serve_accepted_broker_socket<H>(
    fd: OwnedFd,
    expected_uid: u32,
    expected_gid: u32,
    peer_role: BrokerPeerRole,
    responder_role: EndpointRole,
    generation: u64,
    handler: Arc<H>,
) -> Result<(), BrokerServiceFailure>
where
    H: BrokerOperationHandler + 'static,
{
    let socket = SeqpacketSocket::from_owned(fd).map_err(|_| BrokerServiceFailure::Backend)?;
    let credentials = socket
        .acceptor_peer_credentials()
        .map_err(|_| BrokerServiceFailure::PermissionDenied)?;
    if credentials.uid().as_raw() != expected_uid || credentials.gid().as_raw() != expected_gid {
        return Err(BrokerServiceFailure::PermissionDenied);
    }
    let verified_credentials = credentials;
    let verifier = Arc::new(move |accepted: &SeqpacketSocket| {
        if accepted.acceptor_peer_credentials()? == verified_credentials {
            Ok(())
        } else {
            Err(UnixSessionError::CredentialMismatch)
        }
    });
    let policy = broker_endpoint_policy(
        peer_role,
        responder_role,
        generation,
        broker_channel_binding(expected_uid, expected_gid, responder_role),
    )?;
    let attachment_resolver = handler.attachment_policy_resolver().unwrap_or_else(|| {
        Arc::new(
            |_: &d2b_contracts::v2_component_session::AttachmentDescriptor| {
                Err(UnixSessionError::DescriptorMismatch)
            },
        )
    });
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        broker_credit_scopes(policy.attachment_policy.max_per_session),
        attachment_resolver,
        PeerIdentityPolicy::pathname(verifier),
    )
    .map_err(|_| BrokerServiceFailure::PermissionDenied)?;
    let engine = SessionEngine::establish_responder(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .map_err(|_| BrokerServiceFailure::PermissionDenied)?;
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
    serve_broker_session(driver, peer_role, handler).await
}

pub struct RealmBrokerSessionBinding {
    realm_id: String,
    controller_uid: u32,
    controller_gid: u32,
    generation: u64,
}

impl fmt::Debug for RealmBrokerSessionBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RealmBrokerSessionBinding")
            .field("realm_id", &"<closed-id>")
            .field("controller_uid", &self.controller_uid)
            .field("controller_gid", &self.controller_gid)
            .field("generation", &"<redacted>")
            .finish()
    }
}

impl RealmBrokerSessionBinding {
    pub fn new(
        realm_id: String,
        controller_uid: u32,
        controller_gid: u32,
        generation: u64,
    ) -> Result<Self, BrokerServiceFailure> {
        d2b_host::realm_children::validate_realm_id(&realm_id)
            .map_err(|_| BrokerServiceFailure::InvalidRequest)?;
        if controller_uid == 0 || controller_gid == 0 || generation == 0 {
            return Err(BrokerServiceFailure::InvalidRequest);
        }
        Ok(Self {
            realm_id,
            controller_uid,
            controller_gid,
            generation,
        })
    }

    pub fn new_namespace_mapped(
        realm_id: String,
        controller_uid: u32,
        controller_gid: u32,
        generation: u64,
    ) -> Result<Self, BrokerServiceFailure> {
        d2b_host::realm_children::validate_realm_id(&realm_id)
            .map_err(|_| BrokerServiceFailure::InvalidRequest)?;
        if generation == 0 {
            return Err(BrokerServiceFailure::InvalidRequest);
        }
        Ok(Self {
            realm_id,
            controller_uid,
            controller_gid,
            generation,
        })
    }

    pub fn realm_id(&self) -> &str {
        &self.realm_id
    }

    pub fn endpoint_policy(&self) -> Result<EndpointPolicy, BrokerServiceFailure> {
        broker_endpoint_policy(
            BrokerPeerRole::RealmController,
            EndpointRole::RealmBroker,
            self.generation,
            broker_channel_binding(
                self.controller_uid,
                self.controller_gid,
                EndpointRole::RealmBroker,
            ),
        )
    }

    pub async fn serve<H>(&self, fd: OwnedFd, handler: Arc<H>) -> Result<(), BrokerServiceFailure>
    where
        H: BrokerOperationHandler + 'static,
    {
        serve_accepted_broker_socket(
            fd,
            self.controller_uid,
            self.controller_gid,
            BrokerPeerRole::RealmController,
            EndpointRole::RealmBroker,
            self.generation,
            handler,
        )
        .await
    }
}

pub fn broker_channel_binding(
    controller_uid: u32,
    controller_gid: u32,
    responder: EndpointRole,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b.broker.v2\0unix-seqpacket\0");
    digest.update(controller_uid.to_be_bytes());
    digest.update(controller_gid.to_be_bytes());
    digest.update(responder.as_str().as_bytes());
    digest.finalize().into()
}

fn broker_credit_scopes(limit: u16) -> CreditScopeSet {
    let limit = usize::from(limit.max(1));
    CreditScopeSet::new(
        CreditPool::new(limit).expect("positive broker packet credit"),
        CreditPool::new(limit).expect("positive broker request credit"),
        CreditPool::new(limit).expect("positive broker operation credit"),
        CreditPool::new(limit).expect("positive broker session credit"),
        CreditPool::new(limit).expect("positive broker process credit"),
        CreditPool::new(limit).expect("positive broker host credit"),
    )
}

pub fn broker_session_contract(
    peer: BrokerPeerRole,
    responder: EndpointRole,
) -> Result<(), BrokerServiceFailure> {
    let responder_valid = matches!(
        responder,
        EndpointRole::LocalRootBroker | EndpointRole::RealmBroker
    );
    if !responder_valid
        || peer.endpoint_role()
            != match peer {
                BrokerPeerRole::LocalRootController => EndpointRole::LocalRootController,
                BrokerPeerRole::RealmController => EndpointRole::RealmController,
            }
    {
        return Err(BrokerServiceFailure::PermissionDenied);
    }
    let _ = (
        EndpointPurpose::PrivilegedBroker,
        ServicePackage::BrokerV2,
        NoiseProfile::Nn25519ChaChaPolySha256,
        TransportClass::UnixSeqpacket,
        Locality::HostLocal,
    );
    Ok(())
}

pub fn broker_endpoint_policy(
    peer: BrokerPeerRole,
    responder: EndpointRole,
    generation: u64,
    channel_binding: [u8; 32],
) -> Result<EndpointPolicy, BrokerServiceFailure> {
    broker_session_contract(peer, responder)?;
    let service = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2" && service.service == "BrokerService")
        .ok_or(BrokerServiceFailure::Backend)?;
    Ok(EndpointPolicy {
        purpose: EndpointPurpose::PrivilegedBroker,
        purpose_class: PurposeClass::Local,
        initiator_role: peer.endpoint_role(),
        responder_role: responder,
        service: ServicePackage::BrokerV2,
        schema_fingerprint: service_schema_fingerprint(service),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding,
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation,
        attachment_policy: AttachmentPolicy {
            kind: d2b_contracts::v2_component_session::AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 16,
            max_per_request: 16,
            max_per_operation: 32,
            max_per_session: 128,
            credentials_allowed: false,
        },
    })
}

struct AdmittedCall {
    driver: Arc<dyn ComponentSessionDriver>,
    request_id: Option<RequestId>,
    request_key: Vec<u8>,
    active: Arc<Mutex<BTreeMap<Vec<u8>, Cancellation>>>,
    context: BrokerCallContext,
}

impl AdmittedCall {
    async fn finish<T>(mut self, result: ttrpc::Result<T>) -> ttrpc::Result<T> {
        self.active
            .lock()
            .map_err(|_| response_error())?
            .remove(&self.request_key);
        let request_id = self.request_id.take().ok_or_else(response_error)?;
        let completed = if result.is_ok() {
            self.driver.complete_inbound_call(request_id).await
        } else {
            self.driver.remove_inbound_call(request_id).await
        }
        .map_err(|_| response_error())?;
        if !completed {
            return Err(response_error());
        }
        result
    }
}

impl Drop for AdmittedCall {
    fn drop(&mut self) {
        let Some(request_id) = self.request_id.take() else {
            return;
        };
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.request_key);
        }
        let driver = Arc::clone(&self.driver);
        tokio::spawn(async move {
            let _ = driver.remove_inbound_call(request_id).await;
        });
    }
}

fn validate_attachment_table(
    attachments: &[OwnedAttachment],
    indices: &[u32],
    request_id: &RequestId,
    generation: u64,
    method: BrokerMethod,
) -> ttrpc::Result<()> {
    if attachments.len() != indices.len() {
        return Err(attachment_error());
    }
    let method_id = method_id(method)?;
    for (position, (attachment, expected_index)) in
        attachments.iter().zip(indices.iter()).enumerate()
    {
        let descriptor = attachment.descriptor().ok_or_else(attachment_error)?;
        if usize::try_from(*expected_index).ok() != Some(position)
            || descriptor.index != u16::try_from(position).map_err(|_| attachment_error())?
            || &descriptor.request_id != request_id
            || descriptor.reconnect_generation != generation
            || descriptor.service != ServicePackage::BrokerV2
            || descriptor.method_id != method_id
            || !descriptor.cloexec_required
        {
            return Err(attachment_error());
        }
    }
    Ok(())
}

trait ResponseAttachments {
    fn attachment_indices(&self) -> Vec<u32>;
}

impl ResponseAttachments for ServiceResponse {
    fn attachment_indices(&self) -> Vec<u32> {
        self.attachment_indexes.clone()
    }
}

impl ResponseAttachments for AllocateResponse {
    fn attachment_indices(&self) -> Vec<u32> {
        self.resources
            .iter()
            .filter_map(|resource| resource.attachment_index)
            .collect()
    }
}

impl ResponseAttachments for SpawnRealmChildrenResponse {
    fn attachment_indices(&self) -> Vec<u32> {
        self.children
            .iter()
            .map(|child| child.pidfd_attachment_index)
            .collect()
    }
}

fn response_attachment_indices<T: ResponseAttachments>(response: &T) -> Vec<u32> {
    response.attachment_indices()
}

fn requires_idempotency(method: BrokerMethod) -> bool {
    SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2")
        .and_then(|service| {
            service
                .methods
                .iter()
                .find(|candidate| candidate.name == method.name())
        })
        .is_none_or(|method| method.requires_idempotency)
}

fn method_id(method: BrokerMethod) -> ttrpc::Result<u32> {
    SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2")
        .and_then(|service| {
            service
                .methods
                .iter()
                .find(|candidate| candidate.name == method.name())
        })
        .map(|method| method.method_id("d2b.broker.v2", "BrokerService"))
        .ok_or_else(response_error)
}

fn peer_timeout(context: &ttrpc::r#async::TtrpcContext) -> Option<u64> {
    u64::try_from(context.timeout_nano)
        .ok()
        .filter(|timeout| *timeout != 0)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn rpc_error(code: ttrpc::Code, message: &'static str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

fn invalid_request() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INVALID_ARGUMENT, "broker-request-invalid")
}

fn permission_denied() -> ttrpc::Error {
    rpc_error(ttrpc::Code::PERMISSION_DENIED, "broker-admission-denied")
}

fn cancelled() -> ttrpc::Error {
    rpc_error(ttrpc::Code::CANCELLED, "broker-request-cancelled")
}

fn deadline_exceeded() -> ttrpc::Error {
    rpc_error(ttrpc::Code::DEADLINE_EXCEEDED, "broker-deadline-exceeded")
}

fn attachment_error() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INVALID_ARGUMENT, "broker-attachment-mismatch")
}

fn response_error() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INTERNAL, "broker-response-contract-invalid")
}

fn service_error(error: BrokerServiceFailure) -> ttrpc::Error {
    match error {
        BrokerServiceFailure::InvalidRequest => invalid_request(),
        BrokerServiceFailure::PermissionDenied => permission_denied(),
        BrokerServiceFailure::GenerationMismatch => rpc_error(
            ttrpc::Code::FAILED_PRECONDITION,
            "broker-generation-mismatch",
        ),
        BrokerServiceFailure::NotFound => {
            rpc_error(ttrpc::Code::NOT_FOUND, "broker-resource-not-found")
        }
        BrokerServiceFailure::Conflict => {
            rpc_error(ttrpc::Code::ALREADY_EXISTS, "broker-operation-conflict")
        }
        BrokerServiceFailure::DeadlineExceeded => deadline_exceeded(),
        BrokerServiceFailure::Cancelled => cancelled(),
        BrokerServiceFailure::ResourceExhausted => {
            rpc_error(ttrpc::Code::RESOURCE_EXHAUSTED, "broker-resource-exhausted")
        }
        BrokerServiceFailure::AttachmentMismatch => attachment_error(),
        BrokerServiceFailure::Backend => {
            rpc_error(ttrpc::Code::FAILED_PRECONDITION, "broker-operation-failed")
        }
    }
}

#[cfg(test)]
mod ttrpc_bridge_tests {
    use super::*;

    #[tokio::test]
    async fn fragmented_stream_frame_is_reassembled_before_session_send() {
        let body = b"response-body";
        let mut frame = Vec::from(MessageHeader::new_response(1, body.len() as u32));
        frame.extend_from_slice(body);
        let (mut writer, reader) = tokio::io::duplex(128);
        let (mut reader, _) = tokio::io::split(reader);
        let expected = frame.clone();
        let send = tokio::spawn(async move {
            writer.write_all(&frame[..3]).await.unwrap();
            tokio::task::yield_now().await;
            writer.write_all(&frame[3..]).await.unwrap();
        });
        let decoded = read_ttrpc_frame(&mut reader).await.unwrap().unwrap();
        send.await.unwrap();
        assert_eq!(decoded, expected);
    }
}
