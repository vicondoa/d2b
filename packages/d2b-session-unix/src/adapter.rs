use crate::{
    AncillaryCapacity, CreditBundle, CreditScopeSet, DescriptorPolicy, ObjectIdentity,
    PeerCredentials, ReceivedPacket, SeqpacketSocket, StreamSocket, UnixSessionError,
    descriptor::{ReceivedControl, validate_owned_file, validate_owned_file_identity},
};
use async_trait::async_trait;
use d2b_contracts::v2_component_session::{
    AttachmentDescriptor, AttachmentKind, AttachmentPolicy, AttachmentPolicyKind, LimitProfile,
    Locality, TransportClass,
};
use d2b_session::{
    AttachmentPayload, AttachmentValidationError, OwnedAttachment, OwnedTransport,
    TransportDescriptor, TransportError, TransportPacket,
};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
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
            DescriptorPolicy::File(_) | DescriptorPolicy::Pidfd(_),
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
