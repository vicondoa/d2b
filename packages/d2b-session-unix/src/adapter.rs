use crate::{
    AncillaryCapacity, CreditBundle, CreditScopeSet, DescriptorPolicy, ObjectIdentity,
    PeerCredentials, ReceivedPacket, SeqpacketSocket, StreamSocket, UnixSessionError,
    descriptor::{ReceivedControl, validate_owned_file, validate_owned_file_identity},
};
use async_trait::async_trait;
use d2b_contracts::v2_component_session::{
    AttachmentAccess, AttachmentDescriptor, AttachmentKind, AttachmentPolicy, AttachmentPolicyKind,
    AttachmentPurpose, KernelObjectType, LimitProfile, Locality, ServicePackage, TransportClass,
};
use d2b_session::{
    AttachmentPayload, AttachmentValidationError, OwnedAttachment, OwnedTransport,
    TransportDescriptor, TransportError, TransportPacket,
};
use rustix::{
    fd::{AsFd, BorrowedFd, OwnedFd},
    process::Uid,
};
use std::{
    any::Any,
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex},
};

pub type DescriptorPolicyResolver =
    Arc<dyn Fn(&AttachmentDescriptor) -> Result<DescriptorPolicy, UnixSessionError> + Send + Sync>;

pub type PathnamePeerVerifier =
    Arc<dyn Fn(&SeqpacketSocket) -> Result<(), UnixSessionError> + Send + Sync>;

/// One entry of a per-connection closed descriptor allowlist.
///
/// Every axis is checked for an exact match against the received
/// [`AttachmentDescriptor`]: the declaring service composition member, the
/// method that is allowed to attach it, its packet index, its declared
/// purpose, its kernel object type, and whether `close-on-exec` is
/// required. There is no wildcard axis — an attachment that does not match
/// every field of some entry is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorAllowlistEntry {
    pub service: ServicePackage,
    pub method_id: u32,
    pub index: u16,
    pub purpose: AttachmentPurpose,
    pub object_type: KernelObjectType,
    pub cloexec_required: bool,
}

impl DescriptorAllowlistEntry {
    fn matches(&self, descriptor: &AttachmentDescriptor) -> bool {
        self.service == descriptor.service
            && self.method_id == descriptor.method_id
            && self.index == descriptor.index
            && self.purpose == descriptor.purpose
            && self.object_type == descriptor.object_type
            && self.cloexec_required == descriptor.cloexec_required
    }
}

/// Builds a fresh, per-connection [`DescriptorPolicyResolver`] closure that
/// is the sole gate for descriptors received over `SCM_RIGHTS` on this one
/// connection.
///
/// The resolver rejects everything that is not an exact match against
/// `allowlist` (undeclared method/index/purpose/object-type/cloexec
/// combinations), everything bound to a different `session_generation`, and
/// closes down to exactly two supported structural policies: a connected
/// `AF_UNIX`/`SOCK_STREAM` socket credentialed to `peer_uid` (the already
/// -authenticated peer of this connection, never a value read from the
/// wire), or a fully-sealed read-only `memfd`. Every other kernel object
/// type declared in `allowlist` is a caller/builder-time misconfiguration
/// (a static policy authoring error, not attacker-controlled input), so
/// building a resolver with an unsupported entry fails eagerly rather than
/// silently accepting the whole allowlist.
///
/// This function captures `peer_uid`/`session_generation`/`allowlist` by
/// value inside the returned closure and touches no process-global or
/// shared mutable state, so two connections built from independent calls
/// are fully isolated from one another.
pub fn negotiated_descriptor_policy_resolver(
    peer_uid: Uid,
    session_generation: u64,
    allowlist: Vec<DescriptorAllowlistEntry>,
) -> Result<DescriptorPolicyResolver, UnixSessionError> {
    for entry in &allowlist {
        match entry.object_type {
            KernelObjectType::UnixStreamSocket | KernelObjectType::Memfd => {}
            _ => return Err(UnixSessionError::DescriptorMismatch),
        }
    }
    Ok(Arc::new(move |descriptor: &AttachmentDescriptor| {
        if descriptor.kind != AttachmentKind::FileDescriptor {
            return Err(UnixSessionError::DescriptorMismatch);
        }
        if descriptor.reconnect_generation != session_generation {
            return Err(UnixSessionError::DescriptorMismatch);
        }
        let matched = allowlist
            .iter()
            .find(|entry| entry.matches(descriptor))
            .ok_or(UnixSessionError::DescriptorMismatch)?;
        match matched.object_type {
            KernelObjectType::UnixStreamSocket => Ok(DescriptorPolicy::ConnectedUnixStreamSocket {
                expected_peer_uid: peer_uid,
            }),
            KernelObjectType::Memfd if descriptor.access == AttachmentAccess::ReadOnly => {
                Ok(DescriptorPolicy::SealedReadOnlyMemfd)
            }
            _ => Err(UnixSessionError::DescriptorMismatch),
        }
    }))
}

pub enum PeerIdentityPolicy {
    Pathname { verifier: PathnamePeerVerifier },
    InheritedSocketpair { expected_peer: PeerCredentials },
}

impl fmt::Debug for PeerIdentityPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PeerIdentityPolicy(REDACTED)")
    }
}

impl PeerIdentityPolicy {
    fn transport_class(&self) -> TransportClass {
        match self {
            Self::Pathname { .. } => TransportClass::UnixSeqpacket,
            Self::InheritedSocketpair { .. } => TransportClass::InheritedSocketpair,
        }
    }

    pub fn accepted(expected_peer: PeerCredentials) -> Self {
        Self::Pathname {
            verifier: Arc::new(move |socket| {
                if socket.acceptor_peer_credentials()? == expected_peer {
                    Ok(())
                } else {
                    Err(UnixSessionError::CredentialMismatch)
                }
            }),
        }
    }

    pub fn pathname(verifier: PathnamePeerVerifier) -> Self {
        Self::Pathname { verifier }
    }

    pub fn inherited_socketpair(expected_peer: PeerCredentials) -> Self {
        Self::InheritedSocketpair { expected_peer }
    }
}

#[derive(Clone, Copy)]
enum ActivePeerIdentityPolicy {
    Pathname,
    InheritedSocketpair { expected_peer: PeerCredentials },
}

#[derive(Clone, Copy)]
enum PeerPacketPhase {
    FirstPreface,
    Established,
}

enum UnixAttachmentValue {
    File(OwnedFd),
}

enum UnixAttachmentPolicy {
    Bound {
        descriptor: Box<AttachmentDescriptor>,
        policy: Box<DescriptorPolicy>,
    },
    Received {
        resolver: DescriptorPolicyResolver,
        packet: Arc<ReceivedPacketState>,
    },
}

pub struct UnixAttachmentPayload {
    value: UnixAttachmentValue,
    policy: UnixAttachmentPolicy,
    validation: Mutex<Option<(AttachmentDescriptor, Option<ObjectIdentity>)>>,
}

impl fmt::Debug for UnixAttachmentPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UnixAttachmentPayload(REDACTED)")
    }
}

impl UnixAttachmentPayload {
    pub fn file(&self) -> Option<BorrowedFd<'_>> {
        match &self.value {
            UnixAttachmentValue::File(fd) => Some(fd.as_fd()),
        }
    }

    fn received(
        value: UnixAttachmentValue,
        resolver: DescriptorPolicyResolver,
        packet: Arc<ReceivedPacketState>,
    ) -> Self {
        Self {
            value,
            policy: UnixAttachmentPolicy::Received { resolver, packet },
            validation: Mutex::new(None),
        }
    }

    fn validate(
        &self,
        descriptor: &AttachmentDescriptor,
    ) -> Result<Option<ObjectIdentity>, UnixSessionError> {
        let mut validation = self
            .validation
            .lock()
            .map_err(|_| UnixSessionError::DescriptorMismatch)?;
        if let Some((bound, identity)) = validation.as_ref() {
            return if bound == descriptor {
                Ok(identity.clone())
            } else {
                Err(UnixSessionError::DescriptorMismatch)
            };
        }
        descriptor
            .validate(descriptor.index)
            .map_err(|_| UnixSessionError::DescriptorMismatch)?;
        let identity = match &self.policy {
            UnixAttachmentPolicy::Bound {
                descriptor: expected,
                policy,
            } => {
                if !same_descriptor_binding(expected, descriptor) {
                    return Err(UnixSessionError::DescriptorMismatch);
                }
                validate_value(&self.value, descriptor, policy)?
            }
            UnixAttachmentPolicy::Received { resolver, packet } => {
                let policy = resolver(descriptor)?;
                let identity = validate_value(&self.value, descriptor, &policy)?;
                packet.accept(identity.clone(), descriptor.duplicate_object_allowed)?;
                identity
            }
        };
        *validation = Some((descriptor.clone(), identity.clone()));
        Ok(identity)
    }

    fn into_send_parts(self) -> (UnixAttachmentValue, Option<Arc<ReceivedPacketState>>) {
        let retained = match self.policy {
            UnixAttachmentPolicy::Received { packet, .. } => Some(packet),
            UnixAttachmentPolicy::Bound { .. } => None,
        };
        (self.value, retained)
    }
}

impl AttachmentPayload for UnixAttachmentPayload {
    fn close(self: Box<Self>) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    fn validate_descriptor(
        &self,
        descriptor: &AttachmentDescriptor,
    ) -> Result<(), AttachmentValidationError> {
        self.validate(descriptor)
            .map(|_| ())
            .map_err(map_validation_error)
    }
}

pub struct OwnedUnixAttachment;

impl OwnedUnixAttachment {
    pub fn file(
        descriptor: AttachmentDescriptor,
        fd: OwnedFd,
        policy: DescriptorPolicy,
    ) -> Result<OwnedAttachment, UnixSessionError> {
        descriptor
            .validate(descriptor.index)
            .map_err(|_| UnixSessionError::DescriptorMismatch)?;
        validate_owned_file(&fd, &descriptor, &policy)?;
        Ok(OwnedAttachment::new(
            descriptor.clone(),
            Box::new(UnixAttachmentPayload {
                value: UnixAttachmentValue::File(fd),
                policy: UnixAttachmentPolicy::Bound {
                    descriptor: Box::new(descriptor),
                    policy: Box::new(policy),
                },
                validation: Mutex::new(None),
            }),
        ))
    }
}

struct ReceivedPacketState {
    scopes: CreditScopeSet,
    count: usize,
    inner: Mutex<ReceivedPacketStateInner>,
}

struct ReceivedPacketStateInner {
    credits: CreditBundle,
    dispatch_acquired: bool,
    identities: Vec<(ObjectIdentity, bool)>,
}

impl ReceivedPacketState {
    fn new(credits: CreditBundle, scopes: CreditScopeSet, count: usize) -> Self {
        Self {
            scopes,
            count,
            inner: Mutex::new(ReceivedPacketStateInner {
                credits,
                dispatch_acquired: false,
                identities: Vec::new(),
            }),
        }
    }

    fn accept(
        &self,
        identity: Option<ObjectIdentity>,
        duplicate_allowed: bool,
    ) -> Result<(), UnixSessionError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| UnixSessionError::CreditExceeded)?;
        if let Some(actual) = identity {
            if inner.identities.iter().any(|(prior, prior_allows)| {
                prior.same_kernel_object(&actual) && (!duplicate_allowed || !prior_allows)
            }) {
                return Err(UnixSessionError::DuplicateObject);
            }
            inner.identities.push((actual, duplicate_allowed));
        }
        if !inner.dispatch_acquired {
            inner
                .credits
                .acquire_dispatch(&self.scopes, self.count)
                .map_err(|_| UnixSessionError::CreditExceeded)?;
            inner.dispatch_acquired = true;
        }
        Ok(())
    }
}

pub struct UnixSeqpacketTransport {
    socket: SeqpacketSocket,
    class: TransportClass,
    locality: Locality,
    limits: LimitProfile,
    policy: AttachmentPolicy,
    capacity: AncillaryCapacity,
    credits: CreditScopeSet,
    resolver: DescriptorPolicyResolver,
    peer_identity: ActivePeerIdentityPolicy,
    received: VecDeque<ReceivedPacket>,
    closed: bool,
}

impl fmt::Debug for UnixSeqpacketTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnixSeqpacketTransport")
            .field("class", &self.class)
            .field("locality", &self.locality)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl UnixSeqpacketTransport {
    pub fn new(
        socket: SeqpacketSocket,
        locality: Locality,
        limits: LimitProfile,
        policy: AttachmentPolicy,
        credits: CreditScopeSet,
        resolver: DescriptorPolicyResolver,
        peer_identity: PeerIdentityPolicy,
    ) -> Result<Self, UnixSessionError> {
        let class = peer_identity.transport_class();
        if !matches!(
            policy.kind,
            AttachmentPolicyKind::PacketAtomic | AttachmentPolicyKind::Disabled
        ) {
            return Err(UnixSessionError::InvalidSocket);
        }
        policy
            .validate(class)
            .map_err(|_| UnixSessionError::AncillaryCapacity)?;
        let peer_identity = match peer_identity {
            PeerIdentityPolicy::Pathname { verifier } => {
                verifier(&socket)?;
                ActivePeerIdentityPolicy::Pathname
            }
            PeerIdentityPolicy::InheritedSocketpair { expected_peer } => {
                socket.verify_parent_prearmed()?;
                ActivePeerIdentityPolicy::InheritedSocketpair { expected_peer }
            }
        };
        let mut ancillary_policy = policy;
        if matches!(
            peer_identity,
            ActivePeerIdentityPolicy::InheritedSocketpair { .. }
        ) {
            ancillary_policy.kind = AttachmentPolicyKind::PacketAtomic;
            ancillary_policy.credentials_allowed = true;
        }
        let capacity = AncillaryCapacity::from_policy(ancillary_policy)?;
        Ok(Self {
            socket,
            class,
            locality,
            limits,
            policy,
            capacity,
            credits,
            resolver,
            peer_identity,
            received: VecDeque::new(),
            closed: false,
        })
    }

    async fn receive_raw(
        &mut self,
        protected_limit: usize,
    ) -> Result<ReceivedPacket, UnixSessionError> {
        if self.closed {
            return Err(UnixSessionError::Closed);
        }
        let transport_limit = self
            .limits
            .protected_ciphertext_bytes
            .checked_add(2)
            .ok_or(UnixSessionError::PayloadLimit)?;
        if protected_limit == 0
            || protected_limit
                > usize::try_from(transport_limit).map_err(|_| UnixSessionError::PayloadLimit)?
        {
            return Err(UnixSessionError::PayloadLimit);
        }
        if let Some(packet) = self.received.pop_front() {
            return Ok(packet);
        }
        if matches!(
            self.peer_identity,
            ActivePeerIdentityPolicy::InheritedSocketpair { .. }
        ) {
            self.socket.verify_parent_prearmed()?;
        }
        let mut active_limits = self.limits;
        active_limits.protected_ciphertext_bytes =
            u32::try_from(protected_limit).map_err(|_| UnixSessionError::PayloadLimit)?;
        let burst = self
            .socket
            .recv_burst(active_limits, self.capacity, &self.credits, 64)
            .await?;
        self.received.extend(burst.packets);
        self.received.pop_front().ok_or(UnixSessionError::Closed)
    }

    fn received_transport_packet(
        &self,
        packet: ReceivedPacket,
    ) -> Result<TransportPacket, UnixSessionError> {
        let ReceivedPacket {
            payload,
            mut controls,
            unknown_control: _,
            first_on_socket,
            credits,
        } = packet;
        let phase = if first_on_socket {
            PeerPacketPhase::FirstPreface
        } else {
            PeerPacketPhase::Established
        };
        consume_peer_credentials(self.peer_identity, phase, &mut controls)?;
        let file_count = controls
            .iter()
            .filter(|control| matches!(control, ReceivedControl::File(_)))
            .count();
        if file_count > usize::from(self.policy.max_per_packet) {
            return Err(UnixSessionError::CreditExceeded);
        }
        let packet_state = Arc::new(ReceivedPacketState::new(
            credits,
            self.credits.clone(),
            file_count,
        ));
        let attachments = controls
            .into_iter()
            .filter_map(|control| {
                let ReceivedControl::File(fd) = control else {
                    return None;
                };
                Some(OwnedAttachment::unbound(Box::new(
                    UnixAttachmentPayload::received(
                        UnixAttachmentValue::File(fd),
                        self.resolver.clone(),
                        packet_state.clone(),
                    ),
                )))
            })
            .collect();
        Ok(TransportPacket::with_attachments(payload, attachments))
    }

    async fn send_packet(
        &self,
        bytes: Vec<u8>,
        attachments: Vec<OwnedAttachment>,
    ) -> Result<(), UnixSessionError> {
        if attachments.len() > usize::from(self.policy.max_per_packet) {
            return Err(UnixSessionError::CreditExceeded);
        }
        let mut files = Vec::new();
        let mut identities: Vec<(ObjectIdentity, bool)> = Vec::new();
        let mut retained_receive_credits = Vec::new();
        for attachment in attachments {
            let descriptor = attachment
                .descriptor()
                .cloned()
                .ok_or(UnixSessionError::DescriptorMismatch)?;
            if attachment
                .payload()
                .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
                .is_none()
            {
                return Err(UnixSessionError::DescriptorMismatch);
            }
            let payload = attachment
                .into_any()
                .ok_or(UnixSessionError::DescriptorMismatch)?
                .downcast::<UnixAttachmentPayload>()
                .map_err(|_| UnixSessionError::DescriptorMismatch)?;
            if let Some(identity) = payload.validate(&descriptor)? {
                if identities.iter().any(|(prior, prior_allows)| {
                    prior.same_kernel_object(&identity)
                        && (!descriptor.duplicate_object_allowed || !prior_allows)
                }) {
                    return Err(UnixSessionError::DuplicateObject);
                }
                identities.push((identity, descriptor.duplicate_object_allowed));
            }
            let (value, retained) = payload.into_send_parts();
            if let Some(retained) = retained {
                retained_receive_credits.push(retained);
            }
            match value {
                UnixAttachmentValue::File(fd) => files.push(Arc::new(fd)),
            }
        }
        let mut transport_limits = self.limits;
        transport_limits.protected_ciphertext_bytes = transport_limits
            .protected_ciphertext_bytes
            .checked_add(2)
            .ok_or(UnixSessionError::PayloadLimit)?;
        let outbound = crate::OutboundPacket::new(
            bytes,
            files,
            None,
            transport_limits,
            self.capacity,
            &self.credits,
        )?;
        drop(retained_receive_credits);
        let mut queue = VecDeque::from([outbound]);
        loop {
            let burst = self.socket.send_burst(&mut queue, self.capacity, 1).await?;
            if let Some(sent) = burst.sent.into_iter().next() {
                sent.acknowledge();
                return Ok(());
            }
        }
    }
}

#[async_trait]
impl OwnedTransport for UnixSeqpacketTransport {
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: self.class,
            locality: self.locality,
            packet_atomic: true,
            supports_attachments: true,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        let packet = self
            .receive_raw(protected_limit)
            .await
            .map_err(map_transport_error)?;
        self.received_transport_packet(packet)
            .map_err(map_transport_error)
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        let (bytes, attachments) = packet.into_parts();
        self.send_packet(bytes, attachments)
            .await
            .map_err(map_transport_error)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if !self.closed {
            self.socket.close().map_err(map_transport_error)?;
            self.closed = true;
        }
        self.received.clear();
        Ok(())
    }
}

pub struct UnixStreamTransport {
    socket: StreamSocket,
    locality: Locality,
    limits: LimitProfile,
    closed: bool,
}

impl fmt::Debug for UnixStreamTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnixStreamTransport")
            .field("locality", &self.locality)
            .field(
                "protected_ciphertext_limit",
                &self.limits.protected_ciphertext_bytes,
            )
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl UnixStreamTransport {
    pub fn new(socket: StreamSocket, locality: Locality, limits: LimitProfile) -> Self {
        Self {
            socket,
            locality,
            limits,
            closed: false,
        }
    }

    fn configured_wire_limit(&self) -> Result<usize, TransportError> {
        usize::try_from(self.limits.protected_ciphertext_bytes)
            .map_err(|_| TransportError::LimitExceeded)?
            .checked_add(2)
            .ok_or(TransportError::LimitExceeded)
    }

    async fn read_exact_record_bytes(
        &self,
        output: &mut Vec<u8>,
        target_len: usize,
    ) -> Result<(), TransportError> {
        while output.len() < target_len {
            let remaining = target_len
                .checked_sub(output.len())
                .ok_or(TransportError::LimitExceeded)?;
            let received = self
                .socket
                .read_available(output, remaining, 1)
                .await
                .map_err(map_transport_error)?;
            if received.eof {
                return if output.is_empty() {
                    Err(TransportError::Disconnected)
                } else {
                    Err(TransportError::Truncated)
                };
            }
        }
        Ok(())
    }
}

#[async_trait]
impl OwnedTransport for UnixStreamTransport {
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::UnixStream,
            locality: self.locality,
            packet_atomic: false,
            supports_attachments: false,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        let configured_wire_limit = self.configured_wire_limit()?;
        if protected_limit < 2 || protected_limit > configured_wire_limit {
            return Err(TransportError::LimitExceeded);
        }
        let mut bytes = Vec::with_capacity(2);
        self.read_exact_record_bytes(&mut bytes, 2).await?;
        let declared = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
        let wire_len = declared
            .checked_add(2)
            .ok_or(TransportError::LimitExceeded)?;
        if wire_len > protected_limit || wire_len > configured_wire_limit {
            return Err(TransportError::LimitExceeded);
        }
        bytes
            .try_reserve_exact(declared)
            .map_err(|_| TransportError::LimitExceeded)?;
        self.read_exact_record_bytes(&mut bytes, wire_len).await?;
        Ok(TransportPacket::new(bytes))
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        let (bytes, attachments) = packet.into_parts();
        if !attachments.is_empty() {
            return Err(TransportError::InvalidAttachment);
        }
        let configured_wire_limit = self.configured_wire_limit()?;
        if bytes.len() < 2 {
            return Err(TransportError::Truncated);
        }
        let declared = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
        if declared
            .checked_add(2)
            .ok_or(TransportError::LimitExceeded)?
            != bytes.len()
        {
            return Err(TransportError::Truncated);
        }
        if bytes.len() > configured_wire_limit {
            return Err(TransportError::LimitExceeded);
        }
        self.socket
            .write_all(&bytes)
            .await
            .map_err(map_transport_error)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if !self.closed {
            self.socket.close().map_err(map_transport_error)?;
            self.closed = true;
        }
        Ok(())
    }
}

fn validate_value(
    value: &UnixAttachmentValue,
    descriptor: &AttachmentDescriptor,
    policy: &DescriptorPolicy,
) -> Result<Option<ObjectIdentity>, UnixSessionError> {
    match (value, descriptor.kind, policy) {
        (
            UnixAttachmentValue::File(fd),
            AttachmentKind::FileDescriptor,
            DescriptorPolicy::File(_)
            | DescriptorPolicy::SealedReadOnlyMemfd
            | DescriptorPolicy::Pidfd(_)
            | DescriptorPolicy::ConnectedUnixStreamSocket { .. },
        ) => validate_owned_file_identity(fd, descriptor, policy).map(Some),
        (UnixAttachmentValue::File(_), _, _) => Err(UnixSessionError::DescriptorMismatch),
    }
}

fn consume_peer_credentials(
    policy: ActivePeerIdentityPolicy,
    phase: PeerPacketPhase,
    controls: &mut Vec<ReceivedControl>,
) -> Result<(), UnixSessionError> {
    let mut observed = None;
    let mut duplicate = false;
    controls.retain(|control| match control {
        ReceivedControl::File(_) => true,
        ReceivedControl::Credentials(credentials) => {
            if observed.replace(*credentials).is_some() {
                duplicate = true;
            }
            false
        }
    });
    if duplicate {
        return Err(UnixSessionError::ControlMismatch);
    }
    match (policy, phase, observed) {
        (ActivePeerIdentityPolicy::Pathname, _, None) => Ok(()),
        (
            ActivePeerIdentityPolicy::InheritedSocketpair { expected_peer },
            PeerPacketPhase::FirstPreface | PeerPacketPhase::Established,
            Some(actual),
        ) if actual == expected_peer => Ok(()),
        (ActivePeerIdentityPolicy::InheritedSocketpair { .. }, _, Some(_)) => {
            Err(UnixSessionError::CredentialMismatch)
        }
        (ActivePeerIdentityPolicy::InheritedSocketpair { .. }, _, None)
        | (ActivePeerIdentityPolicy::Pathname, _, Some(_)) => {
            Err(UnixSessionError::ControlMismatch)
        }
    }
}

fn same_descriptor_binding(expected: &AttachmentDescriptor, actual: &AttachmentDescriptor) -> bool {
    let mut expected = expected.clone();
    expected.index = actual.index;
    expected.packet_sequence = actual.packet_sequence;
    expected == *actual
}

fn map_validation_error(error: UnixSessionError) -> AttachmentValidationError {
    match error {
        UnixSessionError::MissingCloexec => AttachmentValidationError::CloseOnExec,
        UnixSessionError::DescriptorMismatch => AttachmentValidationError::Other,
        UnixSessionError::CredentialMismatch
        | UnixSessionError::DuplicateObject
        | UnixSessionError::PidfdEvidenceUnavailable
        | UnixSessionError::PidfdIdentityMismatch
        | UnixSessionError::CreditExceeded => AttachmentValidationError::Other,
        UnixSessionError::Io { .. }
        | UnixSessionError::Closed
        | UnixSessionError::InvalidSocket
        | UnixSessionError::BlockingSocket
        | UnixSessionError::PasscredNotPrearmed
        | UnixSessionError::EmptyPacket
        | UnixSessionError::PayloadLimit
        | UnixSessionError::AncillaryCapacity
        | UnixSessionError::MessageTruncated
        | UnixSessionError::ControlTruncated
        | UnixSessionError::UnknownControl
        | UnixSessionError::ControlMismatch
        | UnixSessionError::PacketNotAtomic
        | UnixSessionError::FairnessBudget => AttachmentValidationError::Other,
    }
}

fn map_transport_error(error: UnixSessionError) -> TransportError {
    match error {
        UnixSessionError::Closed => TransportError::Disconnected,
        UnixSessionError::MessageTruncated | UnixSessionError::ControlTruncated => {
            TransportError::Truncated
        }
        UnixSessionError::PayloadLimit
        | UnixSessionError::AncillaryCapacity
        | UnixSessionError::CreditExceeded => TransportError::LimitExceeded,
        UnixSessionError::UnknownControl
        | UnixSessionError::ControlMismatch
        | UnixSessionError::CredentialMismatch
        | UnixSessionError::DescriptorMismatch
        | UnixSessionError::DuplicateObject
        | UnixSessionError::MissingCloexec
        | UnixSessionError::PidfdEvidenceUnavailable
        | UnixSessionError::PidfdIdentityMismatch => TransportError::InvalidAttachment,
        UnixSessionError::Io { .. }
        | UnixSessionError::InvalidSocket
        | UnixSessionError::BlockingSocket
        | UnixSessionError::PasscredNotPrearmed
        | UnixSessionError::EmptyPacket
        | UnixSessionError::PacketNotAtomic
        | UnixSessionError::FairnessBudget => TransportError::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::{
        net::UCred,
        process::{getgid, getpid, getppid, getuid},
    };

    fn current_credentials() -> PeerCredentials {
        PeerCredentials::from_ucred(UCred {
            pid: getpid(),
            uid: getuid(),
            gid: getgid(),
        })
    }

    #[test]
    fn inherited_first_packet_requires_credentials() {
        let expected = current_credentials();
        let mut controls = Vec::new();
        assert_eq!(
            consume_peer_credentials(
                ActivePeerIdentityPolicy::InheritedSocketpair {
                    expected_peer: expected,
                },
                PeerPacketPhase::FirstPreface,
                &mut controls,
            ),
            Err(UnixSessionError::ControlMismatch)
        );
    }

    #[test]
    fn inherited_first_packet_rejects_wrong_credentials() {
        let expected = current_credentials();
        let wrong_pid = getppid().expect("test process has a parent");
        let wrong = PeerCredentials::from_ucred(UCred {
            pid: wrong_pid,
            uid: expected.uid(),
            gid: expected.gid(),
        });
        let mut controls = vec![ReceivedControl::Credentials(wrong)];
        assert_eq!(
            consume_peer_credentials(
                ActivePeerIdentityPolicy::InheritedSocketpair {
                    expected_peer: expected,
                },
                PeerPacketPhase::FirstPreface,
                &mut controls,
            ),
            Err(UnixSessionError::CredentialMismatch)
        );
        assert!(controls.is_empty());
    }
}

#[cfg(test)]
mod negotiated_descriptor_policy_resolver_tests {
    use super::*;
    use crate::credit::{CreditPool, CreditScopeSet};
    use d2b_contracts::v2_component_session::{AttachmentCreditClass, BoundedVec, RequestId};
    use rustix::{
        io::fcntl_getfd,
        net::{AddressFamily, SocketFlags, SocketType, socketpair},
        process::getuid,
    };

    fn scopes(limit: usize) -> CreditScopeSet {
        CreditScopeSet::new(
            CreditPool::new(limit).unwrap(),
            CreditPool::new(limit).unwrap(),
            CreditPool::new(limit).unwrap(),
            CreditPool::new(limit).unwrap(),
            CreditPool::new(limit).unwrap(),
            CreditPool::new(limit).unwrap(),
        )
    }

    fn seqpacket_pair() -> (SeqpacketSocket, SeqpacketSocket) {
        let (left, right) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        (
            SeqpacketSocket::from_owned(left).unwrap(),
            SeqpacketSocket::from_owned(right).unwrap(),
        )
    }

    /// Returns a connected `AF_UNIX`/`SOCK_STREAM` fd. The other end of the
    /// pair is intentionally dropped: Linux caches `SO_PEERCRED` for a
    /// socketpair at connect time, independent of the peer fd's lifetime.
    fn connected_stream_fd() -> OwnedFd {
        let (left, _right) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        left
    }

    fn allowlist_entry(
        service: ServicePackage,
        method_id: u32,
        index: u16,
        purpose: AttachmentPurpose,
        object_type: KernelObjectType,
        cloexec_required: bool,
    ) -> DescriptorAllowlistEntry {
        DescriptorAllowlistEntry {
            service,
            method_id,
            index,
            purpose,
            object_type,
            cloexec_required,
        }
    }

    fn descriptor_for(
        service: ServicePackage,
        method_id: u32,
        index: u16,
        purpose: AttachmentPurpose,
        object_type: KernelObjectType,
        reconnect_generation: u64,
    ) -> AttachmentDescriptor {
        AttachmentDescriptor {
            index,
            kind: AttachmentKind::FileDescriptor,
            object_type,
            access: AttachmentAccess::ReadWrite,
            purpose,
            service,
            method_id,
            request_id: RequestId::new([9_u8; 16]).unwrap(),
            operation_id: None,
            packet_sequence: 1,
            reconnect_generation,
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
            .unwrap(),
        }
    }

    fn base_entry() -> DescriptorAllowlistEntry {
        allowlist_entry(
            ServicePackage::ShellV2,
            42,
            0,
            AttachmentPurpose::RuntimeHandle,
            KernelObjectType::UnixStreamSocket,
            true,
        )
    }

    fn base_descriptor() -> AttachmentDescriptor {
        descriptor_for(
            ServicePackage::ShellV2,
            42,
            0,
            AttachmentPurpose::RuntimeHandle,
            KernelObjectType::UnixStreamSocket,
            7,
        )
    }

    #[test]
    fn builder_rejects_an_unsupported_object_type_eagerly() {
        let unsupported = allowlist_entry(
            ServicePackage::ShellV2,
            1,
            0,
            AttachmentPurpose::RuntimeHandle,
            KernelObjectType::Pidfd,
            true,
        );
        assert!(matches!(
            negotiated_descriptor_policy_resolver(getuid(), 1, vec![unsupported]),
            Err(UnixSessionError::DescriptorMismatch)
        ));
    }

    #[test]
    fn exact_allowlisted_descriptor_resolves_to_connected_unix_stream_policy() {
        let resolver =
            negotiated_descriptor_policy_resolver(getuid(), 7, vec![base_entry()]).unwrap();
        let descriptor = base_descriptor();
        let policy = resolver(&descriptor).unwrap();
        assert!(matches!(
            policy,
            DescriptorPolicy::ConnectedUnixStreamSocket { expected_peer_uid } if expected_peer_uid == getuid()
        ));
    }

    #[test]
    fn every_mismatch_axis_is_rejected() {
        let resolver =
            negotiated_descriptor_policy_resolver(getuid(), 7, vec![base_entry()]).unwrap();

        let mut wrong_service = base_descriptor();
        wrong_service.service = ServicePackage::TtyV2;
        assert!(matches!(
            resolver(&wrong_service),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        let mut wrong_method = base_descriptor();
        wrong_method.method_id = 43;
        assert!(matches!(
            resolver(&wrong_method),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        let mut wrong_index = base_descriptor();
        wrong_index.index = 1;
        assert!(matches!(
            resolver(&wrong_index),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        let mut wrong_purpose = base_descriptor();
        wrong_purpose.purpose = AttachmentPurpose::Terminal;
        assert!(matches!(
            resolver(&wrong_purpose),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        let mut wrong_object_type = base_descriptor();
        wrong_object_type.object_type = KernelObjectType::Memfd;
        assert!(matches!(
            resolver(&wrong_object_type),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        let mut wrong_cloexec = base_descriptor();
        wrong_cloexec.cloexec_required = false;
        assert!(matches!(
            resolver(&wrong_cloexec),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        let mut wrong_generation = base_descriptor();
        wrong_generation.reconnect_generation = 8;
        assert!(matches!(
            resolver(&wrong_generation),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        let mut wrong_kind = base_descriptor();
        wrong_kind.kind = AttachmentKind::Credentials;
        assert!(matches!(
            resolver(&wrong_kind),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        assert!(resolver(&base_descriptor()).is_ok());
    }

    #[test]
    fn two_concurrent_connection_resolvers_are_isolated() {
        let resolver_a =
            negotiated_descriptor_policy_resolver(getuid(), 7, vec![base_entry()]).unwrap();
        let mut entry_b = base_entry();
        entry_b.method_id = 99;
        let resolver_b = negotiated_descriptor_policy_resolver(getuid(), 7, vec![entry_b]).unwrap();

        let descriptor = base_descriptor();
        assert!(resolver_a(&descriptor).is_ok());
        assert!(matches!(
            resolver_b(&descriptor),
            Err(UnixSessionError::DescriptorMismatch)
        ));

        // Building/using resolver_b did not mutate or replace resolver_a's
        // independently captured allowlist/generation/peer_uid state.
        assert!(resolver_a(&descriptor).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn real_scm_rights_transfer_through_the_resolver_accepts_exactly_one_and_retains_cloexec()
    {
        let (sender_socket, receiver_socket) = seqpacket_pair();
        let sender_peer = sender_socket.acceptor_peer_credentials().unwrap();
        let receiver_peer = receiver_socket.acceptor_peer_credentials().unwrap();
        let policy = AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 8,
            credentials_allowed: false,
        };
        let sender_scopes = scopes(8);
        let receiver_scopes = scopes(8);

        let fd = connected_stream_fd();
        let descriptor = base_descriptor();
        let sender_policy = DescriptorPolicy::ConnectedUnixStreamSocket {
            expected_peer_uid: getuid(),
        };
        let attachment = OwnedUnixAttachment::file(descriptor.clone(), fd, sender_policy).unwrap();

        let receiver_resolver =
            negotiated_descriptor_policy_resolver(getuid(), 7, vec![base_entry()]).unwrap();
        // The sender side never consults a resolver (its policy is bound
        // at attachment-construction time), so an empty allowlist is fine.
        let sender_resolver = negotiated_descriptor_policy_resolver(getuid(), 7, vec![]).unwrap();

        let mut sender = UnixSeqpacketTransport::new(
            sender_socket,
            Locality::HostLocal,
            LimitProfile::local_default(),
            policy,
            sender_scopes,
            sender_resolver,
            PeerIdentityPolicy::accepted(sender_peer),
        )
        .unwrap();
        let mut receiver = UnixSeqpacketTransport::new(
            receiver_socket,
            Locality::HostLocal,
            LimitProfile::local_default(),
            policy,
            receiver_scopes,
            receiver_resolver,
            PeerIdentityPolicy::accepted(receiver_peer),
        )
        .unwrap();

        sender
            .send(TransportPacket::with_attachments(
                b"resolver-e2e".to_vec(),
                vec![attachment],
            ))
            .await
            .unwrap();
        let received = receiver
            .receive(LimitProfile::local_default().protected_ciphertext_bytes as usize)
            .await
            .unwrap();
        let (payload, attachments) = received.into_parts();
        assert_eq!(payload, b"resolver-e2e");
        assert_eq!(attachments.len(), 1);
        let unix_payload = attachments[0]
            .payload()
            .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
            .unwrap();
        AttachmentPayload::validate_descriptor(unix_payload, &descriptor).unwrap();
        let received_fd = unix_payload.file().unwrap();
        assert!(
            fcntl_getfd(received_fd)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
    }
}
