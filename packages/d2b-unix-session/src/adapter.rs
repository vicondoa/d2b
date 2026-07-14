use crate::{
    AncillaryCapacity, CreditScopeSet, DescriptorPolicy, PeerCredentials, ReceivedPacket,
    SentPacket, SeqpacketSocket, StreamSocket, UnixSessionError, VerifiedPacket,
    descriptor::validate_owned_file,
};
use async_trait::async_trait;
use d2b_contracts::v2_component_session::{
    AttachmentDescriptor, AttachmentKind, AttachmentPacket, AttachmentPolicy, AttachmentPolicyKind,
    LimitProfile, Locality, TransportClass,
};
use d2b_session::{OwnedTransport, TransportDescriptor, TransportError, TransportPacket};
use rustix::fd::OwnedFd;
use std::{collections::VecDeque, fmt, sync::Arc};

enum OwnedUnixAttachmentValue {
    File(OwnedFd),
    Credentials(PeerCredentials),
}

pub struct OwnedUnixAttachment {
    descriptor: AttachmentDescriptor,
    value: OwnedUnixAttachmentValue,
}

impl fmt::Debug for OwnedUnixAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedUnixAttachment")
            .field("kind", &self.descriptor.kind)
            .field("value", &"REDACTED")
            .finish_non_exhaustive()
    }
}

impl OwnedUnixAttachment {
    pub fn file(
        descriptor: AttachmentDescriptor,
        fd: OwnedFd,
        policy: &DescriptorPolicy,
    ) -> Result<Self, UnixSessionError> {
        descriptor
            .validate(descriptor.index)
            .map_err(|_| UnixSessionError::DescriptorMismatch)?;
        validate_owned_file(&fd, &descriptor, policy)?;
        Ok(Self {
            descriptor,
            value: OwnedUnixAttachmentValue::File(fd),
        })
    }

    pub fn credentials(
        descriptor: AttachmentDescriptor,
        credentials: PeerCredentials,
    ) -> Result<Self, UnixSessionError> {
        descriptor
            .validate(descriptor.index)
            .map_err(|_| UnixSessionError::DescriptorMismatch)?;
        if descriptor.kind != AttachmentKind::Credentials {
            return Err(UnixSessionError::DescriptorMismatch);
        }
        Ok(Self {
            descriptor,
            value: OwnedUnixAttachmentValue::Credentials(credentials),
        })
    }

    pub fn descriptor(&self) -> &AttachmentDescriptor {
        &self.descriptor
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
    received: VecDeque<ReceivedPacket>,
    pending_trait_packet: Option<ReceivedPacket>,
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
        class: TransportClass,
        locality: Locality,
        limits: LimitProfile,
        policy: AttachmentPolicy,
        credits: CreditScopeSet,
    ) -> Result<Self, UnixSessionError> {
        if !matches!(
            class,
            TransportClass::UnixSeqpacket | TransportClass::InheritedSocketpair
        ) || policy.kind != AttachmentPolicyKind::PacketAtomic
        {
            return Err(UnixSessionError::InvalidSocket);
        }
        policy
            .validate(class)
            .map_err(|_| UnixSessionError::AncillaryCapacity)?;
        let capacity = AncillaryCapacity::from_policy(policy)?;
        Ok(Self {
            socket,
            class,
            locality,
            limits,
            policy,
            capacity,
            credits,
            received: VecDeque::new(),
            pending_trait_packet: None,
            closed: false,
        })
    }

    pub async fn receive_owned(
        &mut self,
        protected_limit: usize,
        metadata: &AttachmentPacket,
        policies: &[DescriptorPolicy],
    ) -> Result<VerifiedPacket, UnixSessionError> {
        if self.pending_trait_packet.is_some() {
            return Err(UnixSessionError::ControlMismatch);
        }
        let packet = self.receive_raw(protected_limit).await?;
        packet.verify(metadata, self.policy, policies, &self.credits)
    }

    pub fn accept_trait_packet(
        &mut self,
        metadata: &AttachmentPacket,
        policies: &[DescriptorPolicy],
    ) -> Result<VerifiedPacket, UnixSessionError> {
        self.pending_trait_packet
            .take()
            .ok_or(UnixSessionError::ControlMismatch)?
            .verify(metadata, self.policy, policies, &self.credits)
    }

    pub async fn send_owned(
        &mut self,
        packet: TransportPacket,
        attachments: Vec<OwnedUnixAttachment>,
    ) -> Result<SentPacket, UnixSessionError> {
        if self.closed {
            return Err(UnixSessionError::Closed);
        }
        if attachments.len() > usize::from(self.policy.max_per_packet) {
            return Err(UnixSessionError::CreditExceeded);
        }
        let mut files = Vec::new();
        let mut credentials = None;
        for (index, attachment) in attachments.into_iter().enumerate() {
            attachment
                .descriptor
                .validate(u16::try_from(index).map_err(|_| UnixSessionError::DescriptorMismatch)?)
                .map_err(|_| UnixSessionError::DescriptorMismatch)?;
            match attachment.value {
                OwnedUnixAttachmentValue::File(fd) if credentials.is_none() => {
                    files.push(Arc::new(fd));
                }
                OwnedUnixAttachmentValue::Credentials(value) if credentials.is_none() => {
                    credentials = Some(value);
                }
                OwnedUnixAttachmentValue::File(_) | OwnedUnixAttachmentValue::Credentials(_) => {
                    return Err(UnixSessionError::ControlMismatch);
                }
            }
        }
        let outbound = crate::OutboundPacket::new(
            packet.into_bytes(),
            files,
            credentials,
            self.limits,
            self.capacity,
            &self.credits,
        )?;
        let mut queue = VecDeque::from([outbound]);
        loop {
            let mut burst = self.socket.send_burst(&mut queue, self.capacity, 1).await?;
            if let Some(sent) = burst.sent.pop() {
                return Ok(sent);
            }
        }
    }

    async fn receive_raw(
        &mut self,
        protected_limit: usize,
    ) -> Result<ReceivedPacket, UnixSessionError> {
        if self.closed {
            return Err(UnixSessionError::Closed);
        }
        if protected_limit == 0
            || protected_limit
                > usize::try_from(self.limits.protected_ciphertext_bytes)
                    .map_err(|_| UnixSessionError::PayloadLimit)?
        {
            return Err(UnixSessionError::PayloadLimit);
        }
        if let Some(packet) = self.received.pop_front() {
            return Ok(packet);
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
        if self.pending_trait_packet.is_some() {
            return Err(TransportError::InvalidAttachment);
        }
        let packet = self
            .receive_raw(protected_limit)
            .await
            .map_err(map_transport_error)?;
        let bytes = packet.payload().to_vec();
        if packet.control_count() == 0 {
            drop(packet);
        } else {
            self.pending_trait_packet = Some(packet);
        }
        Ok(TransportPacket::new(bytes))
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        self.send_owned(packet, Vec::new())
            .await
            .map(SentPacket::acknowledge)
            .map_err(map_transport_error)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if !self.closed {
            self.socket.close().map_err(map_transport_error)?;
            self.closed = true;
        }
        self.received.clear();
        self.pending_trait_packet = None;
        Ok(())
    }
}

pub struct UnixStreamTransport {
    socket: StreamSocket,
    locality: Locality,
    closed: bool,
}

impl fmt::Debug for UnixStreamTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnixStreamTransport")
            .field("locality", &self.locality)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl UnixStreamTransport {
    pub fn new(socket: StreamSocket, locality: Locality) -> Self {
        Self {
            socket,
            locality,
            closed: false,
        }
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
        if protected_limit == 0 {
            return Err(TransportError::LimitExceeded);
        }
        let mut bytes = Vec::new();
        let received = self
            .socket
            .read_available(&mut bytes, protected_limit, 64)
            .await
            .map_err(map_transport_error)?;
        if received.eof && bytes.is_empty() {
            return Err(TransportError::Disconnected);
        }
        Ok(TransportPacket::new(bytes))
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        self.socket
            .write_all(packet.as_bytes())
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
        UnixSessionError::Io
        | UnixSessionError::InvalidSocket
        | UnixSessionError::BlockingSocket
        | UnixSessionError::PasscredNotPrearmed
        | UnixSessionError::EmptyPacket
        | UnixSessionError::PacketNotAtomic
        | UnixSessionError::FairnessBudget => TransportError::Other,
    }
}
