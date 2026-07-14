use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};

use d2b_contracts::v2_component_session::{
    AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
    AttachmentPacket, AttachmentPolicyKind, BoundedVec, CancelAck, CancelRequest, CancelResult,
    ChannelId, CloseReason, CloseRecord, EndpointPolicy, FRAGMENT_HEADER_LEN, FragmentHeader,
    HandshakeOffer, KeepaliveRecord, KernelObjectType, LimitProfile, MAX_PACKET_ATTACHMENTS,
    OperationId, PREFACE_LEN, RecordHeader, RecordKind, Remediation, RequestId, ServicePackage,
    SessionErrorCode,
};

use crate::{
    Cancellation, FairScheduler, Fragment, Fragmenter, HandshakeCredentials, HandshakeRole,
    KeepaliveAction, NamedStreamMux, NoiseHandshake, OutboundFrame, OwnedAttachment,
    OwnedTransport, QueueClass, Reassembler, RecordProtector, Result, SessionError,
    SessionLifecycle, StreamEvent, StreamId, TransportPacket, encode_offer, negotiate_offer,
};

const ATTACHMENT_BATCH: u8 = 1;
const ATTACHMENT_ACK: u8 = 2;
const STREAM_CLOSE: u8 = 1;
const STREAM_CREDIT: u8 = 2;
const STREAM_RESET: u8 = 3;
const ATTACHMENT_DESCRIPTOR_BYTES: usize = 62;

pub enum SessionEvent {
    Ttrpc(Vec<u8>),
    NamedStream(StreamEvent),
    Attachments(Vec<OwnedAttachment>),
    CancelRequest(CancelAck),
    CancelAck(CancelAck),
    AttachmentAcknowledged { count: u16 },
    Close(CloseRecord),
    ControlProcessed,
}

impl fmt::Debug for SessionEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ttrpc(bytes) => formatter
                .debug_tuple("SessionEvent::Ttrpc")
                .field(&format_args!("<redacted:{} bytes>", bytes.len()))
                .finish(),
            Self::NamedStream(event) => event.fmt(formatter),
            Self::Attachments(attachments) => formatter
                .debug_struct("SessionEvent::Attachments")
                .field("count", &attachments.len())
                .finish(),
            Self::CancelRequest(ack) => formatter
                .debug_struct("SessionEvent::CancelRequest")
                .field("result", &ack.result.as_str())
                .field("request", &"<redacted>")
                .finish(),
            Self::CancelAck(ack) => formatter
                .debug_struct("SessionEvent::CancelAck")
                .field("result", &ack.result.as_str())
                .field("request", &"<redacted>")
                .finish(),
            Self::AttachmentAcknowledged { count } => formatter
                .debug_struct("SessionEvent::AttachmentAcknowledged")
                .field("count", count)
                .finish(),
            Self::Close(record) => formatter
                .debug_tuple("SessionEvent::Close")
                .field(&record.reason.as_str())
                .finish(),
            Self::ControlProcessed => formatter.write_str("SessionEvent::ControlProcessed"),
        }
    }
}

pub struct SessionEngine<T: OwnedTransport> {
    transport: T,
    offer: HandshakeOffer,
    protector: RecordProtector,
    lifecycle: SessionLifecycle,
    scheduler: FairScheduler,
    streams: NamedStreamMux,
    outbound_requests: crate::RequestRegistry,
    inbound_requests: crate::RequestRegistry,
    reassemblers: BTreeMap<(RecordKind, ChannelId), Reassembler>,
    next_message_id: u64,
    next_record_sequence: u64,
    pending_attachment_credits: BTreeMap<u64, u16>,
    outstanding_attachment_credits: u16,
}

impl<T: OwnedTransport> SessionEngine<T> {
    pub async fn establish_initiator(
        transport: T,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
        now: Instant,
    ) -> Result<Self> {
        let timeout = Duration::from_millis(u64::from(policy.limits.handshake_deadline_ms));
        tokio::time::timeout(
            timeout,
            Self::establish_initiator_inner(transport, policy, credentials, now),
        )
        .await
        .map_err(|_| SessionError::new(SessionErrorCode::HandshakeTimeout))?
    }

    async fn establish_initiator_inner(
        mut transport: T,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
        now: Instant,
    ) -> Result<Self> {
        validate_transport(&transport, &policy)?;
        let (preface, offer_bytes) = encode_offer(&policy)?;
        let negotiated = negotiate_offer(&preface, &offer_bytes, &policy)?;
        let mut first = Vec::with_capacity(PREFACE_LEN + offer_bytes.len());
        first.extend_from_slice(&preface);
        first.extend_from_slice(&offer_bytes);
        transport.send(TransportPacket::new(first)).await?;

        let mut noise = NoiseHandshake::new(HandshakeRole::Initiator, &negotiated, credentials)?;
        transport
            .send(TransportPacket::new(noise.write_next()?))
            .await?;
        let response =
            receive_clean(&mut transport, policy.limits.protected_ciphertext_bytes).await?;
        noise.read_next(&response)?;
        let established = noise.finish()?;
        Self::from_established(transport, negotiated.offer().clone(), established, now)
    }

    pub async fn establish_responder(
        transport: T,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
        now: Instant,
    ) -> Result<Self> {
        let timeout = Duration::from_millis(u64::from(policy.limits.handshake_deadline_ms));
        tokio::time::timeout(
            timeout,
            Self::establish_responder_inner(transport, policy, credentials, now),
        )
        .await
        .map_err(|_| SessionError::new(SessionErrorCode::HandshakeTimeout))?
    }

    async fn establish_responder_inner(
        mut transport: T,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
        now: Instant,
    ) -> Result<Self> {
        validate_transport(&transport, &policy)?;
        let first = receive_clean(
            &mut transport,
            (PREFACE_LEN + d2b_contracts::v2_component_session::MAX_HANDSHAKE_OFFER_BYTES) as u32,
        )
        .await?;
        if first.len() < PREFACE_LEN {
            return Err(SessionError::new(SessionErrorCode::MalformedPreface));
        }
        let negotiated = negotiate_offer(&first[..PREFACE_LEN], &first[PREFACE_LEN..], &policy)?;
        let mut noise = NoiseHandshake::new(HandshakeRole::Responder, &negotiated, credentials)?;
        let request =
            receive_clean(&mut transport, policy.limits.protected_ciphertext_bytes).await?;
        noise.read_next(&request)?;
        transport
            .send(TransportPacket::new(noise.write_next()?))
            .await?;
        let established = noise.finish()?;
        Self::from_established(transport, negotiated.offer().clone(), established, now)
    }

    fn from_established(
        transport: T,
        offer: HandshakeOffer,
        established: crate::EstablishedHandshake,
        now: Instant,
    ) -> Result<Self> {
        let generation = offer.reconnect_generation;
        Ok(Self {
            transport,
            offer: offer.clone(),
            protector: RecordProtector::from_handshake(established),
            lifecycle: SessionLifecycle::new(generation, offer.limits, now)?,
            scheduler: FairScheduler::new(offer.limits)?,
            streams: NamedStreamMux::new(offer.limits)?,
            outbound_requests: crate::RequestRegistry::new(generation)?,
            inbound_requests: crate::RequestRegistry::new(generation)?,
            reassemblers: BTreeMap::new(),
            next_message_id: 1,
            next_record_sequence: 0,
            pending_attachment_credits: BTreeMap::new(),
            outstanding_attachment_credits: 0,
        })
    }

    pub fn generation(&self) -> u64 {
        self.offer.reconnect_generation
    }

    pub fn outstanding_attachment_credits(&self) -> u16 {
        self.outstanding_attachment_credits
    }

    pub async fn reconnect_initiator(
        mut self,
        transport: T,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
        now: Instant,
    ) -> Result<Self> {
        self.lifecycle.disconnect(now);
        let generation = self.lifecycle.begin_reconnect(now)?;
        if policy.reconnect_generation != generation {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        self.outbound_requests.cancel_all();
        self.inbound_requests.cancel_all();
        self.pending_attachment_credits.clear();
        self.outstanding_attachment_credits = 0;
        self.transport.close().await?;
        Self::establish_initiator(transport, policy, credentials, now).await
    }

    pub async fn reconnect_responder(
        mut self,
        transport: T,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
        now: Instant,
    ) -> Result<Self> {
        self.lifecycle.disconnect(now);
        let generation = self.lifecycle.begin_reconnect(now)?;
        if policy.reconnect_generation != generation {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        self.outbound_requests.cancel_all();
        self.inbound_requests.cancel_all();
        self.pending_attachment_credits.clear();
        self.outstanding_attachment_credits = 0;
        self.transport.close().await?;
        Self::establish_responder(transport, policy, credentials, now).await
    }

    pub async fn call(&mut self, request_id: RequestId, frame: Vec<u8>) -> Result<Cancellation> {
        let cancellation = self.outbound_requests.register(request_id.clone())?;
        if let Err(error) = self.send_ttrpc(frame).await {
            self.outbound_requests.complete(&request_id);
            return Err(error);
        }
        self.outbound_requests.mark_dispatched(&request_id)?;
        Ok(cancellation)
    }

    pub fn complete_call(&mut self, request_id: &RequestId) -> bool {
        self.outbound_requests.complete(request_id)
    }

    pub fn register_inbound_call(&mut self, request_id: RequestId) -> Result<Cancellation> {
        self.inbound_requests.register(request_id)
    }

    pub async fn cancel_call(&mut self, request_id: &RequestId) -> Result<()> {
        let request = CancelRequest {
            reconnect_generation: self.generation(),
            request_id: request_id.clone(),
        };
        self.send_logical(
            RecordKind::CancelRequest,
            ChannelId::SESSION_CONTROL,
            encode_cancel_request(&request),
            Vec::new(),
        )
        .await
    }

    pub async fn send_ttrpc(&mut self, frame: Vec<u8>) -> Result<()> {
        if frame.is_empty() || frame.len() > self.offer.limits.logical_ttrpc_bytes as usize {
            return Err(SessionError::new(SessionErrorCode::ReassemblyLimitExceeded));
        }
        self.scheduler
            .enqueue(OutboundFrame::control(QueueClass::TtrpcControl, frame)?)?;
        self.flush().await
    }

    pub fn open_named_stream(
        &mut self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<()> {
        self.streams.open(stream, send_credit, receive_credit)?;
        if let Err(error) = self.scheduler.register_stream(stream, send_credit) {
            self.streams.reset(stream)?;
            self.streams.remove_terminal(stream);
            return Err(error);
        }
        Ok(())
    }

    pub async fn send_named_stream(&mut self, stream: StreamId, bytes: Vec<u8>) -> Result<()> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        self.streams.reserve_send(stream, bytes.len())?;
        if let Err(error) = self.scheduler.enqueue(OutboundFrame::named(stream, bytes)?) {
            self.streams.refund_send_credit(stream, len)?;
            return Err(error);
        }
        self.flush().await
    }

    pub async fn grant_named_stream_credit(&mut self, stream: StreamId, bytes: u32) -> Result<()> {
        self.streams.release_receive_credit(stream, bytes)?;
        self.send_logical(
            RecordKind::SessionControl,
            ChannelId::SESSION_CONTROL,
            encode_stream_control(STREAM_CREDIT, stream, bytes),
            Vec::new(),
        )
        .await
    }

    pub async fn close_named_stream(&mut self, stream: StreamId) -> Result<()> {
        self.streams.close_local(stream)?;
        self.send_logical(
            RecordKind::SessionControl,
            ChannelId::SESSION_CONTROL,
            encode_stream_control(STREAM_CLOSE, stream, 0),
            Vec::new(),
        )
        .await
    }

    pub async fn reset_named_stream(&mut self, stream: StreamId) -> Result<()> {
        self.streams.reset(stream)?;
        self.scheduler.remove_stream(stream);
        self.send_logical(
            RecordKind::SessionControl,
            ChannelId::SESSION_CONTROL,
            encode_stream_control(STREAM_RESET, stream, 0),
            Vec::new(),
        )
        .await
    }

    pub async fn send_attachments(&mut self, mut attachments: Vec<OwnedAttachment>) -> Result<()> {
        let policy = self.offer.attachment_policy;
        let count = u16::try_from(attachments.len())
            .map_err(|_| SessionError::new(SessionErrorCode::AttachmentCreditExceeded))?;
        if policy.kind != AttachmentPolicyKind::PacketAtomic
            || count == 0
            || count > policy.max_per_packet
            || count > MAX_PACKET_ATTACHMENTS
        {
            return Err(SessionError::new(
                SessionErrorCode::AttachmentCreditExceeded,
            ));
        }
        let outstanding = self
            .outstanding_attachment_credits
            .checked_add(count)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if outstanding > policy.max_per_session {
            return Err(SessionError::new(
                SessionErrorCode::AttachmentCreditExceeded,
            ));
        }
        let sequence = self.next_record_sequence;
        for (index, attachment) in attachments.iter_mut().enumerate() {
            attachment.bind(index as u16, sequence, self.generation());
            if attachment.descriptor().service != self.offer.service {
                return Err(SessionError::new(
                    SessionErrorCode::AttachmentDescriptorMismatch,
                ));
            }
            attachment.descriptor().validate(index as u16)?;
        }
        let descriptors = attachments
            .iter()
            .map(|attachment| attachment.descriptor().clone())
            .collect();
        let packet = AttachmentPacket {
            declared_count: count,
            descriptors: BoundedVec::new(descriptors)?,
        };
        packet
            .validate(policy, attachments.len(), false, false, false)
            .map_err(|_| SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch))?;
        let payload = encode_attachment_batch(&packet)?;
        self.pending_attachment_credits.insert(sequence, count);
        self.outstanding_attachment_credits = outstanding;
        if let Err(error) = self
            .send_logical(
                RecordKind::Attachment,
                ChannelId::ATTACHMENT_CONTROL,
                payload,
                attachments,
            )
            .await
        {
            self.pending_attachment_credits.remove(&sequence);
            self.outstanding_attachment_credits =
                self.outstanding_attachment_credits.saturating_sub(count);
            return Err(error);
        }
        Ok(())
    }

    pub async fn drive_keepalive(&mut self, now: Instant) -> Result<()> {
        match self.lifecycle.poll_keepalive(now) {
            KeepaliveAction::None => Ok(()),
            KeepaliveAction::SendPing(ping) => {
                self.send_logical(
                    RecordKind::KeepalivePing,
                    ChannelId::SESSION_CONTROL,
                    encode_keepalive(ping),
                    Vec::new(),
                )
                .await
            }
            KeepaliveAction::Close(record) => {
                self.send_close_record(record).await?;
                self.transport.close().await.map_err(SessionError::from)
            }
        }
    }

    pub async fn receive(&mut self) -> Result<SessionEvent> {
        let result = self.receive_inner().await;
        if result.is_err() {
            self.fail_closed().await;
        }
        result
    }

    async fn receive_inner(&mut self) -> Result<SessionEvent> {
        let packet = self
            .transport
            .receive(self.offer.limits.protected_ciphertext_bytes as usize + 2)
            .await?;
        let (wire, attachments) = packet.into_parts();
        let (header, protected_payload) = self.protector.unprotect(&wire)?;
        self.lifecycle.on_activity(Instant::now());
        if !attachments.is_empty() && header.kind != RecordKind::Attachment {
            return Err(SessionError::new(
                SessionErrorCode::AttachmentDescriptorMismatch,
            ));
        }
        if header.kind == RecordKind::Attachment {
            return self
                .receive_attachment(header, protected_payload, attachments)
                .await;
        }
        let payload = self.reassemble(header, protected_payload)?;
        let Some(payload) = payload else {
            return Ok(SessionEvent::ControlProcessed);
        };
        match header.kind {
            RecordKind::Ttrpc => Ok(SessionEvent::Ttrpc(payload)),
            RecordKind::NamedStream => {
                let stream = StreamId::new(header.channel.value())?;
                Ok(SessionEvent::NamedStream(
                    self.streams.receive_data(stream, payload)?,
                ))
            }
            RecordKind::KeepalivePing => {
                let ping = decode_keepalive(&payload)?;
                if ping.reconnect_generation != self.generation() {
                    return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
                }
                self.send_logical(
                    RecordKind::KeepalivePong,
                    ChannelId::SESSION_CONTROL,
                    encode_keepalive(ping),
                    Vec::new(),
                )
                .await?;
                Ok(SessionEvent::ControlProcessed)
            }
            RecordKind::KeepalivePong => {
                self.lifecycle
                    .receive_pong(decode_keepalive(&payload)?, Instant::now())?;
                Ok(SessionEvent::ControlProcessed)
            }
            RecordKind::Close => {
                let close = decode_close(&payload)?;
                if close.reconnect_generation != self.generation() {
                    return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
                }
                self.outbound_requests.cancel_all();
                self.inbound_requests.cancel_all();
                self.lifecycle.close(close.reason, close.remediation);
                self.transport.close().await?;
                Ok(SessionEvent::Close(close))
            }
            RecordKind::CancelRequest => {
                let request = decode_cancel_request(&payload)?;
                let ack = self.inbound_requests.cancel(request);
                self.send_logical(
                    RecordKind::CancelAck,
                    ChannelId::SESSION_CONTROL,
                    encode_cancel_ack(&ack),
                    Vec::new(),
                )
                .await?;
                Ok(SessionEvent::CancelRequest(ack))
            }
            RecordKind::CancelAck => {
                let ack = decode_cancel_ack(&payload)?;
                if ack.reconnect_generation != self.generation() {
                    return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
                }
                self.outbound_requests.signal(&ack.request_id);
                Ok(SessionEvent::CancelAck(ack))
            }
            RecordKind::SessionControl => self.receive_stream_control(&payload),
            RecordKind::Attachment => Err(SessionError::new(SessionErrorCode::InternalInvariant)),
        }
    }

    async fn receive_attachment(
        &mut self,
        header: RecordHeader,
        protected_payload: Vec<u8>,
        attachments: Vec<OwnedAttachment>,
    ) -> Result<SessionEvent> {
        if protected_payload.len() < FRAGMENT_HEADER_LEN {
            return Err(SessionError::new(SessionErrorCode::FragmentTruncated));
        }
        let fragment_len = protected_payload.len() - FRAGMENT_HEADER_LEN;
        let fragment_header = FragmentHeader::decode(
            &protected_payload[..FRAGMENT_HEADER_LEN],
            fragment_len as u32,
            self.offer.limits.session_control_queue_bytes,
        )?;
        if fragment_header.count != 1 {
            return Err(SessionError::new(
                SessionErrorCode::AttachmentDescriptorMismatch,
            ));
        }
        let payload = &protected_payload[FRAGMENT_HEADER_LEN..];
        let decoded = decode_attachment_control(payload)?;
        match decoded {
            AttachmentControl::Batch(packet) => {
                packet
                    .validate(
                        self.offer.attachment_policy,
                        attachments.len(),
                        false,
                        false,
                        false,
                    )
                    .map_err(|_| {
                        SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch)
                    })?;
                for (index, (declared, actual)) in packet
                    .descriptors
                    .iter()
                    .zip(attachments.iter())
                    .enumerate()
                {
                    if declared != actual.descriptor()
                        || declared.packet_sequence != header.sequence
                        || declared.reconnect_generation != self.generation()
                        || declared.service != self.offer.service
                    {
                        return Err(SessionError::new(
                            SessionErrorCode::AttachmentDescriptorMismatch,
                        ));
                    }
                    declared.validate(index as u16)?;
                }
                self.send_attachment_ack(header.sequence, packet.declared_count)
                    .await?;
                Ok(SessionEvent::Attachments(attachments))
            }
            AttachmentControl::Ack { sequence, count } => {
                if !attachments.is_empty() {
                    return Err(SessionError::new(
                        SessionErrorCode::AttachmentDescriptorMismatch,
                    ));
                }
                let pending = self.pending_attachment_credits.remove(&sequence);
                if pending != Some(count) {
                    return Err(SessionError::new(
                        SessionErrorCode::AttachmentCreditExceeded,
                    ));
                }
                self.outstanding_attachment_credits = self
                    .outstanding_attachment_credits
                    .checked_sub(count)
                    .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
                Ok(SessionEvent::AttachmentAcknowledged { count })
            }
        }
    }

    fn receive_stream_control(&mut self, payload: &[u8]) -> Result<SessionEvent> {
        let (kind, stream, value) = decode_stream_control(payload)?;
        match kind {
            STREAM_CLOSE => Ok(SessionEvent::NamedStream(
                self.streams.receive_close(stream)?,
            )),
            STREAM_CREDIT => {
                self.streams.grant_send_credit(stream, value)?;
                self.scheduler.grant_stream_credit(stream, value)?;
                Ok(SessionEvent::ControlProcessed)
            }
            STREAM_RESET => {
                self.scheduler.remove_stream(stream);
                Ok(SessionEvent::NamedStream(self.streams.reset(stream)?))
            }
            _ => Err(SessionError::new(SessionErrorCode::UnknownControl)),
        }
    }

    fn reassemble(&mut self, header: RecordHeader, payload: Vec<u8>) -> Result<Option<Vec<u8>>> {
        if payload.len() < FRAGMENT_HEADER_LEN {
            return Err(SessionError::new(SessionErrorCode::FragmentTruncated));
        }
        let logical_limit = logical_limit(header.kind, self.offer.limits);
        let fragment_len = payload.len() - FRAGMENT_HEADER_LEN;
        let fragment_header = FragmentHeader::decode(
            &payload[..FRAGMENT_HEADER_LEN],
            fragment_len as u32,
            logical_limit,
        )?;
        let key = (header.kind, header.channel);
        let fragment =
            Fragment::from_parts(fragment_header, payload[FRAGMENT_HEADER_LEN..].to_vec());
        let reassembler = self
            .reassemblers
            .entry(key)
            .or_insert(Reassembler::new(logical_limit)?);
        let complete = reassembler.accept(fragment)?;
        if complete.is_some() {
            self.reassemblers.remove(&key);
        }
        Ok(complete)
    }

    async fn flush(&mut self) -> Result<()> {
        while let Some(frame) = self.scheduler.dequeue() {
            let (kind, channel) = match frame.class() {
                QueueClass::SessionControl => (RecordKind::SessionControl, frame.channel()),
                QueueClass::TtrpcControl => (RecordKind::Ttrpc, frame.channel()),
                QueueClass::AttachmentControl => (RecordKind::Attachment, frame.channel()),
                QueueClass::NamedStream => (RecordKind::NamedStream, frame.channel()),
            };
            self.send_logical(kind, channel, frame.as_bytes().to_vec(), Vec::new())
                .await?;
        }
        Ok(())
    }

    async fn send_logical(
        &mut self,
        kind: RecordKind,
        channel: ChannelId,
        payload: Vec<u8>,
        attachments: Vec<OwnedAttachment>,
    ) -> Result<()> {
        let limit = logical_limit(kind, self.offer.limits);
        let message_id = self.next_message_id;
        self.next_message_id = self
            .next_message_id
            .checked_add(1)
            .ok_or_else(|| SessionError::new(SessionErrorCode::NonceExhausted))?;
        let fragments =
            Fragmenter::new(self.offer.limits, limit)?.fragment(message_id, &payload)?;
        if !attachments.is_empty() && fragments.len() != 1 {
            return Err(SessionError::new(
                SessionErrorCode::AttachmentDescriptorMismatch,
            ));
        }
        let mut attachments = Some(attachments);
        for fragment in fragments {
            let fragment_len = u32::try_from(fragment.as_bytes().len())
                .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
            let mut record_payload =
                Vec::with_capacity(FRAGMENT_HEADER_LEN + fragment.as_bytes().len());
            record_payload.extend_from_slice(&fragment.header.encode(fragment_len, limit)?);
            record_payload.extend_from_slice(fragment.as_bytes());
            let protected = self.protector.protect(kind, channel, &record_payload)?;
            let packet_attachments = attachments.take().unwrap_or_default();
            self.next_record_sequence = self
                .next_record_sequence
                .checked_add(1)
                .ok_or_else(|| SessionError::new(SessionErrorCode::NonceExhausted))?;
            if let Err(error) = self
                .transport
                .send(TransportPacket::with_attachments(
                    protected.into_bytes(),
                    packet_attachments,
                ))
                .await
            {
                let error = SessionError::from(error);
                self.fail_closed().await;
                return Err(error);
            }
        }
        Ok(())
    }

    async fn send_attachment_ack(&mut self, sequence: u64, count: u16) -> Result<()> {
        let mut payload = Vec::with_capacity(11);
        payload.push(ATTACHMENT_ACK);
        payload.extend_from_slice(&sequence.to_be_bytes());
        payload.extend_from_slice(&count.to_be_bytes());
        self.send_logical(
            RecordKind::Attachment,
            ChannelId::ATTACHMENT_CONTROL,
            payload,
            Vec::new(),
        )
        .await
    }

    async fn send_close_record(&mut self, record: CloseRecord) -> Result<()> {
        self.send_logical(
            RecordKind::Close,
            ChannelId::SESSION_CONTROL,
            encode_close(record),
            Vec::new(),
        )
        .await
    }

    pub async fn close(&mut self, reason: CloseReason, remediation: Remediation) -> Result<()> {
        let record = self.lifecycle.close(reason, remediation);
        self.outbound_requests.cancel_all();
        self.inbound_requests.cancel_all();
        self.pending_attachment_credits.clear();
        self.outstanding_attachment_credits = 0;
        let send = self.send_close_record(record).await;
        let close = self.transport.close().await.map_err(SessionError::from);
        send.and(close)
    }

    async fn fail_closed(&mut self) {
        self.outbound_requests.cancel_all();
        self.inbound_requests.cancel_all();
        self.pending_attachment_credits.clear();
        self.outstanding_attachment_credits = 0;
        self.lifecycle.close(
            CloseReason::InternalInvariant,
            Remediation::ReplaceGeneration,
        );
        let _ = self.transport.close().await;
    }
}

impl<T: OwnedTransport> fmt::Debug for SessionEngine<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionEngine")
            .field("purpose", &self.offer.purpose.as_str())
            .field("generation", &"<redacted>")
            .field("transport", &self.transport.descriptor())
            .field(
                "attachments_outstanding",
                &self.outstanding_attachment_credits,
            )
            .finish_non_exhaustive()
    }
}

fn validate_transport<T: OwnedTransport>(transport: &T, policy: &EndpointPolicy) -> Result<()> {
    let descriptor = transport.descriptor();
    let attachment_enabled = policy.attachment_policy.kind == AttachmentPolicyKind::PacketAtomic;
    if descriptor.class != policy.transport_binding.transport
        || descriptor.locality != policy.transport_binding.locality
        || descriptor.supports_attachments != attachment_enabled
        || (attachment_enabled && !descriptor.packet_atomic)
    {
        return Err(SessionError::new(SessionErrorCode::ChannelBindingMismatch));
    }
    Ok(())
}

async fn receive_clean<T: OwnedTransport>(transport: &mut T, limit: u32) -> Result<Vec<u8>> {
    let packet = transport.receive(limit as usize).await?;
    let (bytes, attachments) = packet.into_parts();
    if !attachments.is_empty() {
        return Err(SessionError::new(
            SessionErrorCode::AttachmentDescriptorMismatch,
        ));
    }
    Ok(bytes)
}

fn logical_limit(kind: RecordKind, limits: LimitProfile) -> u32 {
    match kind {
        RecordKind::Ttrpc => limits.logical_ttrpc_bytes,
        RecordKind::NamedStream => limits.logical_named_stream_bytes,
        RecordKind::Attachment
        | RecordKind::SessionControl
        | RecordKind::KeepalivePing
        | RecordKind::KeepalivePong
        | RecordKind::Close
        | RecordKind::CancelRequest
        | RecordKind::CancelAck => limits.session_control_queue_bytes,
    }
}

fn encode_keepalive(record: KeepaliveRecord) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&record.reconnect_generation.to_be_bytes());
    bytes.extend_from_slice(&record.nonce.to_be_bytes());
    bytes
}

fn decode_keepalive(bytes: &[u8]) -> Result<KeepaliveRecord> {
    if bytes.len() != 16 {
        return Err(SessionError::new(SessionErrorCode::UnknownControl));
    }
    Ok(KeepaliveRecord {
        reconnect_generation: read_u64(&bytes[..8])?,
        nonce: read_u64(&bytes[8..])?,
    })
}

fn encode_cancel_request(request: &CancelRequest) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(24);
    bytes.extend_from_slice(&request.reconnect_generation.to_be_bytes());
    bytes.extend_from_slice(request.request_id.as_bytes());
    bytes
}

fn decode_cancel_request(bytes: &[u8]) -> Result<CancelRequest> {
    if bytes.len() != 24 {
        return Err(SessionError::new(SessionErrorCode::UnknownControl));
    }
    Ok(CancelRequest {
        reconnect_generation: read_u64(&bytes[..8])?,
        request_id: RequestId::new(bytes[8..].to_vec())?,
    })
}

fn encode_cancel_ack(ack: &CancelAck) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(25);
    bytes.extend_from_slice(&ack.reconnect_generation.to_be_bytes());
    bytes.extend_from_slice(ack.request_id.as_bytes());
    bytes.push(cancel_result_tag(ack.result));
    bytes
}

fn decode_cancel_ack(bytes: &[u8]) -> Result<CancelAck> {
    if bytes.len() != 25 {
        return Err(SessionError::new(SessionErrorCode::UnknownControl));
    }
    Ok(CancelAck {
        reconnect_generation: read_u64(&bytes[..8])?,
        request_id: RequestId::new(bytes[8..24].to_vec())?,
        result: cancel_result_from_tag(bytes[24])?,
    })
}

fn cancel_result_tag(result: CancelResult) -> u8 {
    match result {
        CancelResult::CancelledBeforeDispatch => 1,
        CancelResult::CancellationSignalled => 2,
        CancelResult::AlreadyTerminal => 3,
        CancelResult::UnknownRequest => 4,
        CancelResult::GenerationMismatch => 5,
    }
}

fn cancel_result_from_tag(tag: u8) -> Result<CancelResult> {
    match tag {
        1 => Ok(CancelResult::CancelledBeforeDispatch),
        2 => Ok(CancelResult::CancellationSignalled),
        3 => Ok(CancelResult::AlreadyTerminal),
        4 => Ok(CancelResult::UnknownRequest),
        5 => Ok(CancelResult::GenerationMismatch),
        _ => Err(SessionError::new(SessionErrorCode::UnknownControl)),
    }
}

fn encode_close(record: CloseRecord) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(10);
    bytes.extend_from_slice(&record.reconnect_generation.to_be_bytes());
    bytes.push(close_reason_tag(record.reason));
    bytes.push(record.remediation.tag());
    bytes
}

fn decode_close(bytes: &[u8]) -> Result<CloseRecord> {
    if bytes.len() != 10 {
        return Err(SessionError::new(SessionErrorCode::UnknownControl));
    }
    Ok(CloseRecord {
        reconnect_generation: read_u64(&bytes[..8])?,
        reason: close_reason_from_tag(bytes[8])?,
        remediation: Remediation::from_tag(bytes[9])
            .map_err(|_| SessionError::new(SessionErrorCode::UnknownControl))?,
    })
}

fn close_reason_tag(reason: CloseReason) -> u8 {
    match reason {
        CloseReason::Normal => 1,
        CloseReason::PeerRequested => 2,
        CloseReason::AuthenticationFailed => 3,
        CloseReason::PurposeMismatch => 4,
        CloseReason::RoleMismatch => 5,
        CloseReason::SchemaMismatch => 6,
        CloseReason::LimitMismatch => 7,
        CloseReason::ChannelBindingMismatch => 8,
        CloseReason::Replay => 9,
        CloseReason::RecordTruncated => 10,
        CloseReason::FragmentInvalid => 11,
        CloseReason::NonceExhausted => 12,
        CloseReason::DeadlineExpired => 13,
        CloseReason::Cancelled => 14,
        CloseReason::AttachmentInvalid => 15,
        CloseReason::AttachmentTruncated => 16,
        CloseReason::UnknownControl => 17,
        CloseReason::CreditExhausted => 18,
        CloseReason::ControlResourceExhausted => 19,
        CloseReason::SchedulerStalled => 20,
        CloseReason::KeepaliveTimeout => 21,
        CloseReason::SessionLost => 22,
        CloseReason::InternalInvariant => 23,
    }
}

fn close_reason_from_tag(tag: u8) -> Result<CloseReason> {
    match tag {
        1 => Ok(CloseReason::Normal),
        2 => Ok(CloseReason::PeerRequested),
        3 => Ok(CloseReason::AuthenticationFailed),
        4 => Ok(CloseReason::PurposeMismatch),
        5 => Ok(CloseReason::RoleMismatch),
        6 => Ok(CloseReason::SchemaMismatch),
        7 => Ok(CloseReason::LimitMismatch),
        8 => Ok(CloseReason::ChannelBindingMismatch),
        9 => Ok(CloseReason::Replay),
        10 => Ok(CloseReason::RecordTruncated),
        11 => Ok(CloseReason::FragmentInvalid),
        12 => Ok(CloseReason::NonceExhausted),
        13 => Ok(CloseReason::DeadlineExpired),
        14 => Ok(CloseReason::Cancelled),
        15 => Ok(CloseReason::AttachmentInvalid),
        16 => Ok(CloseReason::AttachmentTruncated),
        17 => Ok(CloseReason::UnknownControl),
        18 => Ok(CloseReason::CreditExhausted),
        19 => Ok(CloseReason::ControlResourceExhausted),
        20 => Ok(CloseReason::SchedulerStalled),
        21 => Ok(CloseReason::KeepaliveTimeout),
        22 => Ok(CloseReason::SessionLost),
        23 => Ok(CloseReason::InternalInvariant),
        _ => Err(SessionError::new(SessionErrorCode::UnknownControl)),
    }
}

fn encode_stream_control(kind: u8, stream: StreamId, value: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(7);
    bytes.push(kind);
    bytes.extend_from_slice(&stream.channel().value().to_be_bytes());
    bytes.extend_from_slice(&value.to_be_bytes());
    bytes
}

fn decode_stream_control(bytes: &[u8]) -> Result<(u8, StreamId, u32)> {
    if bytes.len() != 7 {
        return Err(SessionError::new(SessionErrorCode::UnknownControl));
    }
    Ok((
        bytes[0],
        StreamId::new(u16::from_be_bytes([bytes[1], bytes[2]]))?,
        u32::from_be_bytes(
            bytes[3..7]
                .try_into()
                .map_err(|_| SessionError::new(SessionErrorCode::UnknownControl))?,
        ),
    ))
}

fn encode_attachment_batch(packet: &AttachmentPacket) -> Result<Vec<u8>> {
    let capacity = 3usize
        .checked_add(
            packet
                .descriptors
                .len()
                .checked_mul(ATTACHMENT_DESCRIPTOR_BYTES)
                .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?,
        )
        .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.push(ATTACHMENT_BATCH);
    bytes.extend_from_slice(&packet.declared_count.to_be_bytes());
    for descriptor in packet.descriptors.iter() {
        encode_attachment_descriptor(&mut bytes, descriptor);
    }
    Ok(bytes)
}

fn encode_attachment_descriptor(bytes: &mut Vec<u8>, descriptor: &AttachmentDescriptor) {
    bytes.extend_from_slice(&descriptor.index.to_be_bytes());
    bytes.push(descriptor.kind.tag());
    bytes.push(descriptor.object_type.tag());
    bytes.push(descriptor.access.tag());
    bytes.push(descriptor.purpose.tag());
    bytes.push(descriptor.service.tag());
    bytes.extend_from_slice(&descriptor.method_id.to_be_bytes());
    bytes.extend_from_slice(descriptor.request_id.as_bytes());
    match &descriptor.operation_id {
        Some(operation) => {
            bytes.push(1);
            bytes.extend_from_slice(operation.as_bytes());
        }
        None => {
            bytes.push(0);
            bytes.extend_from_slice(&[0; 16]);
        }
    }
    bytes.extend_from_slice(&descriptor.packet_sequence.to_be_bytes());
    bytes.extend_from_slice(&descriptor.reconnect_generation.to_be_bytes());
    bytes.push(u8::from(descriptor.duplicate_object_allowed));
    bytes.push(u8::from(descriptor.cloexec_required));
}

enum AttachmentControl {
    Batch(AttachmentPacket),
    Ack { sequence: u64, count: u16 },
}

fn decode_attachment_control(bytes: &[u8]) -> Result<AttachmentControl> {
    let Some(kind) = bytes.first().copied() else {
        return Err(SessionError::new(
            SessionErrorCode::AttachmentDescriptorMismatch,
        ));
    };
    if kind == ATTACHMENT_ACK {
        if bytes.len() != 11 {
            return Err(SessionError::new(
                SessionErrorCode::AttachmentDescriptorMismatch,
            ));
        }
        return Ok(AttachmentControl::Ack {
            sequence: read_u64(&bytes[1..9])?,
            count: u16::from_be_bytes([bytes[9], bytes[10]]),
        });
    }
    if kind != ATTACHMENT_BATCH || bytes.len() < 3 {
        return Err(SessionError::new(
            SessionErrorCode::AttachmentDescriptorMismatch,
        ));
    }
    let count = u16::from_be_bytes([bytes[1], bytes[2]]);
    let expected = 3usize
        .checked_add(
            usize::from(count)
                .checked_mul(ATTACHMENT_DESCRIPTOR_BYTES)
                .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?,
        )
        .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
    if bytes.len() != expected || count > MAX_PACKET_ATTACHMENTS {
        return Err(SessionError::new(
            SessionErrorCode::AttachmentDescriptorMismatch,
        ));
    }
    let mut descriptors = Vec::with_capacity(usize::from(count));
    let mut offset = 3;
    for _ in 0..count {
        descriptors.push(decode_attachment_descriptor(
            &bytes[offset..offset + ATTACHMENT_DESCRIPTOR_BYTES],
        )?);
        offset += ATTACHMENT_DESCRIPTOR_BYTES;
    }
    Ok(AttachmentControl::Batch(AttachmentPacket {
        declared_count: count,
        descriptors: BoundedVec::new(descriptors)?,
    }))
}

fn decode_attachment_descriptor(bytes: &[u8]) -> Result<AttachmentDescriptor> {
    if bytes.len() != ATTACHMENT_DESCRIPTOR_BYTES {
        return Err(SessionError::new(
            SessionErrorCode::AttachmentDescriptorMismatch,
        ));
    }
    let operation_id = match bytes[27] {
        0 if bytes[28..44] == [0; 16] => None,
        1 => Some(OperationId::new(bytes[28..44].to_vec())?),
        _ => {
            return Err(SessionError::new(
                SessionErrorCode::AttachmentDescriptorMismatch,
            ));
        }
    };
    Ok(AttachmentDescriptor {
        index: u16::from_be_bytes([bytes[0], bytes[1]]),
        kind: AttachmentKind::from_tag(bytes[2])
            .map_err(|_| SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch))?,
        object_type: KernelObjectType::from_tag(bytes[3])
            .map_err(|_| SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch))?,
        access: AttachmentAccess::from_tag(bytes[4])
            .map_err(|_| SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch))?,
        purpose: d2b_contracts::v2_component_session::AttachmentPurpose::from_tag(bytes[5])
            .map_err(|_| SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch))?,
        service: ServicePackage::from_tag(bytes[6])
            .map_err(|_| SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch))?,
        method_id: u32::from_be_bytes(
            bytes[7..11]
                .try_into()
                .map_err(|_| SessionError::new(SessionErrorCode::AttachmentDescriptorMismatch))?,
        ),
        request_id: RequestId::new(bytes[11..27].to_vec())?,
        operation_id,
        packet_sequence: read_u64(&bytes[44..52])?,
        reconnect_generation: read_u64(&bytes[52..60])?,
        duplicate_object_allowed: decode_bool(bytes[60])?,
        cloexec_required: decode_bool(bytes[61])?,
        credit_classes: BoundedVec::new(vec![
            AttachmentCreditClass::Packet,
            AttachmentCreditClass::Request,
            AttachmentCreditClass::Operation,
            AttachmentCreditClass::Session,
            AttachmentCreditClass::Process,
            AttachmentCreditClass::Host,
        ])?,
    })
}

fn decode_bool(value: u8) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(SessionError::new(
            SessionErrorCode::AttachmentDescriptorMismatch,
        )),
    }
}

fn read_u64(bytes: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| SessionError::new(SessionErrorCode::UnknownControl))?;
    Ok(u64::from_be_bytes(bytes))
}
