use std::{
    collections::BTreeMap,
    fmt,
    io::IoSliceMut,
    os::fd::{AsRawFd, OwnedFd},
    sync::Arc,
    time::Instant,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPolicyIdentity, EndpointPurpose,
        EndpointRole, IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile,
        PurposeClass, RequestId, ServicePackage, SessionErrorCode, TransportBinding,
        TransportClass, directional_channel_binding,
    },
    v2_services::{
        RUNTIME_SYSTEMD_USER_COMPOSITION, SERVICE_INVENTORY, broker, common, decode_strict, guest,
        service_schema_fingerprint, terminal,
    },
};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, OwnedTransport, SessionEngine,
    TransportDescriptor, TransportError, TransportPacket,
};
use d2b_session_unix::UnixSeqpacketTransport;
use nix::{
    cmsg_space,
    sys::socket::{
        ControlMessageOwned, MsgFlags, Shutdown, SockType, getsockopt, recvmsg, send, shutdown,
        sockopt,
    },
    unistd::close,
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{
    io::unix::AsyncFd,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc},
};
use ttrpc::{
    r#async::transport::Socket,
    proto::{MESSAGE_HEADER_LENGTH, MESSAGE_TYPE_REQUEST, MESSAGE_TYPE_RESPONSE, MessageHeader},
};

use crate::{
    ClientError, ComponentSessionConnector, ConnectedSession, ResolvedTarget, ServiceKind,
    TransportKind,
};

struct PendingSession {
    transport: HostTransport,
    identity: EndpointPolicyIdentity,
    credentials: HandshakeCredentials,
}

enum HostTransport {
    Unix(Box<UnixSeqpacketTransport>),
    Daemon(DaemonSeqpacketTransport),
}

#[async_trait]
impl OwnedTransport for HostTransport {
    fn descriptor(&self) -> TransportDescriptor {
        match self {
            Self::Unix(transport) => transport.descriptor(),
            Self::Daemon(transport) => transport.descriptor(),
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        match self {
            Self::Unix(transport) => transport.receive(protected_limit).await,
            Self::Daemon(transport) => transport.receive(protected_limit).await,
        }
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        match self {
            Self::Unix(transport) => transport.send(packet).await,
            Self::Daemon(transport) => transport.send(packet).await,
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        match self {
            Self::Unix(transport) => transport.close().await,
            Self::Daemon(transport) => transport.close().await,
        }
    }
}

struct DaemonSeqpacketTransport {
    socket: AsyncFd<OwnedFd>,
    closed: bool,
}

impl DaemonSeqpacketTransport {
    fn new(fd: OwnedFd, expected_peer_uid: u32) -> Result<Self, ClientError> {
        let peer = getsockopt(&fd, sockopt::PeerCredentials)
            .map_err(|_| ClientError::TransportPolicyMismatch)?;
        if getsockopt(&fd, sockopt::SockType).ok() != Some(SockType::SeqPacket)
            || peer.uid() != expected_peer_uid
        {
            return Err(ClientError::TransportPolicyMismatch);
        }
        Ok(Self {
            socket: AsyncFd::new(fd).map_err(|_| ClientError::ConnectFailed)?,
            closed: false,
        })
    }
}

impl fmt::Debug for DaemonSeqpacketTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonSeqpacketTransport")
            .field("closed", &self.closed)
            .finish()
    }
}

#[async_trait]
impl OwnedTransport for DaemonSeqpacketTransport {
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            packet_atomic: true,
            supports_attachments: false,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        loop {
            let mut ready = self
                .socket
                .readable()
                .await
                .map_err(|_| TransportError::Disconnected)?;
            match ready.try_io(|inner| receive_daemon_packet(inner.get_ref(), protected_limit)) {
                Ok(Ok(result)) => return result,
                Ok(Err(error)) => return Err(classify_io_error(error)),
                Err(_) => continue,
            }
        }
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        if !packet.attachments().is_empty() || packet.as_bytes().is_empty() {
            return Err(TransportError::InvalidAttachment);
        }
        loop {
            let mut ready = self
                .socket
                .writable()
                .await
                .map_err(|_| TransportError::Disconnected)?;
            let result = ready.try_io(|inner| {
                send(
                    inner.get_ref().as_raw_fd(),
                    packet.as_bytes(),
                    MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_NOSIGNAL,
                )
                .map_err(nix_io_error)
            });
            match result {
                Ok(Ok(sent)) if sent == packet.as_bytes().len() => return Ok(()),
                Ok(Ok(_)) => return Err(TransportError::Truncated),
                Ok(Err(error)) => return Err(classify_io_error(error)),
                Err(_) => continue,
            }
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if !self.closed {
            self.closed = true;
            shutdown(self.socket.get_ref().as_raw_fd(), Shutdown::Both)
                .map_err(|error| classify_io_error(nix_io_error(error)))?;
        }
        Ok(())
    }
}

fn receive_daemon_packet(
    fd: &OwnedFd,
    protected_limit: usize,
) -> std::io::Result<Result<TransportPacket, TransportError>> {
    let mut bytes = vec![0_u8; protected_limit];
    let mut io = [IoSliceMut::new(&mut bytes)];
    let mut ancillary = cmsg_space!([i32; 1]);
    let message = recvmsg::<()>(
        fd.as_raw_fd(),
        &mut io,
        Some(&mut ancillary),
        MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_TRUNC | MsgFlags::MSG_CMSG_CLOEXEC,
    )
    .map_err(nix_io_error)?;
    let received = message.bytes;
    let truncated = message
        .flags
        .intersects(MsgFlags::MSG_TRUNC | MsgFlags::MSG_CTRUNC)
        || received > protected_limit;
    let mut unexpected_control = false;
    if let Ok(controls) = message.cmsgs() {
        for control in controls {
            unexpected_control = true;
            if let ControlMessageOwned::ScmRights(rights) = control {
                for received_fd in rights {
                    let _ = close(received_fd);
                }
            }
        }
    }
    if truncated {
        return Ok(Err(TransportError::LimitExceeded));
    }
    if unexpected_control {
        return Ok(Err(TransportError::InvalidAttachment));
    }
    if received == 0 {
        return Ok(Err(TransportError::Disconnected));
    }
    bytes.truncate(received);
    Ok(Ok(TransportPacket::new(bytes)))
}

fn nix_io_error(error: nix::errno::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error as i32)
}

fn classify_io_error(error: std::io::Error) -> TransportError {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::NotConnected => TransportError::Disconnected,
        std::io::ErrorKind::WouldBlock => TransportError::WouldBlock,
        _ => TransportError::Other,
    }
}

pub struct HostSocketConnector {
    transport: TransportKind,
    pending: Mutex<Option<PendingSession>>,
}

impl HostSocketConnector {
    pub fn new(
        transport: UnixSeqpacketTransport,
        identity: EndpointPolicyIdentity,
        credentials: HandshakeCredentials,
    ) -> Result<Self, ClientError> {
        identity
            .validate_local_generation_discovery()
            .map_err(|_| ClientError::TransportPolicyMismatch)?;
        let selected = match identity.transport_binding.transport {
            TransportClass::UnixSeqpacket => TransportKind::LocalUnix,
            _ => return Err(ClientError::TransportPolicyMismatch),
        };
        Ok(Self {
            transport: selected,
            pending: Mutex::new(Some(PendingSession {
                transport: HostTransport::Unix(Box::new(transport)),
                identity,
                credentials,
            })),
        })
    }

    pub fn from_seqpacket_fd(
        fd: OwnedFd,
        expected_peer_uid: u32,
        identity: EndpointPolicyIdentity,
        credentials: HandshakeCredentials,
    ) -> Result<Self, ClientError> {
        identity
            .validate_local_generation_discovery()
            .map_err(|_| ClientError::TransportPolicyMismatch)?;
        if identity.transport_binding.transport != TransportClass::UnixSeqpacket {
            return Err(ClientError::TransportPolicyMismatch);
        }
        Ok(Self {
            transport: TransportKind::LocalUnix,
            pending: Mutex::new(Some(PendingSession {
                transport: HostTransport::Daemon(DaemonSeqpacketTransport::new(
                    fd,
                    expected_peer_uid,
                )?),
                identity,
                credentials,
            })),
        })
    }
}

pub fn local_daemon_endpoint_identity(
    client_uid: u32,
    client_gid: u32,
) -> Result<EndpointPolicyIdentity, ClientError> {
    let service = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.daemon.v2" && service.service == "DaemonService")
        .ok_or(ClientError::InvalidService)?;
    let mut binding = Sha256::new();
    binding.update(b"d2b.daemon.v2\0unix-seqpacket\0");
    binding.update(client_uid.to_be_bytes());
    binding.update(client_gid.to_be_bytes());
    let identity = EndpointPolicyIdentity {
        purpose: EndpointPurpose::DaemonLocal,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::CommandClient,
        responder_role: EndpointRole::LocalRootController,
        service: ServicePackage::DaemonV2,
        schema_fingerprint: service_schema_fingerprint(service),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: binding.finalize().into(),
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        attachment_policy: AttachmentPolicy::disabled(),
    };
    identity
        .validate_local_generation_discovery()
        .map_err(|_| ClientError::TransportPolicyMismatch)?;
    Ok(identity)
}

/// The canonical, fully-formed [`EndpointPolicyIdentity`] for the frozen
/// RuntimeSystemdUser+Shell+Tty composition (see
/// [`RUNTIME_SYSTEMD_USER_COMPOSITION`]).
///
/// `responder_uid`/`responder_gid` are the target runtime-systemd-user
/// helper's own identity — known out of band by the initiator (the
/// local-root controller always connects to a specific, already-known
/// target user's session) and never supplied by the peer on the wire. The
/// helper independently derives the same channel-binding digest from its
/// own process identity via
/// `d2b_session_unix::ResponderIdentity::current().channel_binding(..)`, so
/// this function and that responder-side call must keep agreeing on
/// [`d2b_contracts::v2_component_session::directional_channel_binding`]'s
/// domain, transport, and role inputs.
///
/// `service`/`schema_fingerprint` come from
/// [`ServiceComposition::endpoint_policy_identity`], so a caller can never
/// drift from the frozen composition by hand-rolling either field: any of
/// [`ServiceKind::RuntimeSystemdUser`], [`ServiceKind::Shell`], or
/// [`ServiceKind::Tty`] negotiate this SAME identity, and none of them can
/// be reached with a standalone single-service identity instead.
///
/// This crate's own (non-test) code never calls this directly: it is the
/// initiator-side constructor a future external caller (the local-root
/// controller / `d2bd`) uses to build the identity it hands to
/// [`HostSocketConnector::new`]/[`HostSocketConnector::from_seqpacket_fd`]
/// before calling [`crate::Client::connect`] with
/// [`ServiceKind::RuntimeSystemdUser`]. It is covered by this module's own
/// tests today; exposing it outside this crate additionally requires a
/// follow-up `pub use` in `lib.rs`.
#[allow(dead_code)]
pub fn runtime_systemd_user_composition_endpoint_identity(
    responder_uid: u32,
    responder_gid: u32,
) -> Result<EndpointPolicyIdentity, ClientError> {
    let identity = RUNTIME_SYSTEMD_USER_COMPOSITION.endpoint_policy_identity(
        EndpointPurpose::RuntimeSystemdUser,
        PurposeClass::Local,
        EndpointRole::LocalRootController,
        EndpointRole::RuntimeSystemdUserAgent,
        NoiseProfile::Nn25519ChaChaPolySha256,
        LimitProfile::local_default(),
        TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: directional_channel_binding(
                RUNTIME_SYSTEMD_USER_COMPOSITION.primary(),
                TransportClass::UnixSeqpacket,
                EndpointRole::RuntimeSystemdUserAgent,
                responder_uid,
                responder_gid,
            ),
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 64,
            credentials_allowed: false,
        },
    );
    identity
        .validate_local_generation_discovery()
        .map_err(|_| ClientError::TransportPolicyMismatch)?;
    Ok(identity)
}

/// Whether `identity` may be used to negotiate `service` on this connector.
///
/// A composition member (currently `RuntimeSystemdUser`, `Shell`, and `Tty`)
/// is only admitted when `identity` carries the exact frozen composition tag
/// — [`RUNTIME_SYSTEMD_USER_COMPOSITION`]'s primary package and schema
/// fingerprint — never a standalone single-service identity for that same
/// member package. Every non-member service keeps the prior exact
/// service-package equality check unchanged.
fn identity_admits_service(identity: &EndpointPolicyIdentity, service: ServiceKind) -> bool {
    let expected = service_package(service);
    if RUNTIME_SYSTEMD_USER_COMPOSITION.contains(expected) {
        identity.service == RUNTIME_SYSTEMD_USER_COMPOSITION.primary()
            && identity.schema_fingerprint == RUNTIME_SYSTEMD_USER_COMPOSITION.schema_fingerprint()
    } else {
        identity.service == expected
    }
}

impl fmt::Debug for HostSocketConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HostSocketConnector([redacted])")
    }
}

#[async_trait]
impl ComponentSessionConnector for HostSocketConnector {
    async fn connect(
        &self,
        target: &ResolvedTarget,
        service: ServiceKind,
    ) -> Result<ConnectedSession, ClientError> {
        if target.transport() != self.transport {
            return Err(ClientError::TransportPolicyMismatch);
        }
        let pending = self
            .pending
            .lock()
            .await
            .take()
            .ok_or(ClientError::ConnectFailed)?;
        if !identity_admits_service(&pending.identity, service) {
            return Err(ClientError::InvalidService);
        }
        let limits = pending.identity.limits;
        let engine = SessionEngine::establish_initiator_with_generation_discovery(
            pending.transport,
            pending.identity,
            pending.credentials,
            Instant::now(),
        )
        .await
        .map_err(|error| ClientError::SessionEstablishment(error.code()))?;
        let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
        let (client, bridge) = tokio::io::duplex(2 * 1024 * 1024);
        tokio::spawn(pump_ttrpc(bridge, Arc::clone(&driver)));
        Ok(ConnectedSession {
            driver,
            ttrpc_socket: Socket::new(client),
            limits,
        })
    }
}

async fn pump_ttrpc(
    socket: DuplexStream,
    driver: Arc<dyn ComponentSessionDriver>,
) -> Result<(), ()> {
    const MAX_IN_FLIGHT_REQUESTS: usize = 128;
    let (reader, mut writer) = tokio::io::split(socket);
    let (responses, mut response_receiver) = mpsc::channel(MAX_IN_FLIGHT_REQUESTS);
    let in_flight = Arc::new(Mutex::new(BTreeMap::new()));
    let mut dispatcher = tokio::spawn(dispatch_ttrpc_requests(
        reader,
        Arc::clone(&driver),
        responses.clone(),
        MAX_IN_FLIGHT_REQUESTS,
        Arc::clone(&in_flight),
    ));
    let mut receiver = tokio::spawn(receive_ttrpc_responses(
        Arc::clone(&driver),
        responses,
        in_flight,
    ));
    let mut control = tokio::spawn(drain_session_controls(driver));
    loop {
        tokio::select! {
            result = &mut dispatcher => {
                receiver.abort();
                control.abort();
                return result.map_err(|_| ())?;
            }
            result = &mut receiver => {
                dispatcher.abort();
                control.abort();
                return result.map_err(|_| ())?;
            }
            result = &mut control => {
                dispatcher.abort();
                receiver.abort();
                return result.map_err(|_| ())?;
            }
            response = response_receiver.recv() => {
                let response = response.ok_or(())??;
                if writer.write_all(&response).await.is_err() {
                    dispatcher.abort();
                    receiver.abort();
                    control.abort();
                    return Err(());
                }
            }
        }
    }
}

async fn drain_session_controls(driver: Arc<dyn ComponentSessionDriver>) -> Result<(), ()> {
    loop {
        if matches!(
            driver.receive_control().await.map_err(|_| ())?,
            d2b_session::SessionEvent::Close(_)
        ) {
            return Err(());
        }
    }
}

async fn dispatch_ttrpc_requests<R>(
    mut reader: R,
    driver: Arc<dyn ComponentSessionDriver>,
    responses: mpsc::Sender<Result<Vec<u8>, ()>>,
    maximum_in_flight: usize,
    in_flight: Arc<Mutex<BTreeMap<u32, InFlightRequest>>>,
) -> Result<(), ()>
where
    R: AsyncRead + Unpin,
{
    let permits = Arc::new(Semaphore::new(maximum_in_flight));
    loop {
        let (header, request, frame) = match read_ttrpc_request(&mut reader).await {
            Ok(request) => request,
            Err(()) => return Err(()),
        };
        if header.type_ != MESSAGE_TYPE_REQUEST {
            return Err(());
        }
        if request.method == "Cancel" {
            let response =
                dispatch_cancel_request(header, request, Arc::clone(&driver), &in_flight).await;
            responses.send(response).await.map_err(|_| ())?;
            continue;
        }
        let permit = match Arc::clone(&permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                responses
                    .send(ttrpc_error_response(
                        header.stream_id,
                        ttrpc::Code::RESOURCE_EXHAUSTED,
                        "client ttrpc concurrency limit exceeded",
                    ))
                    .await
                    .map_err(|_| ())?;
                continue;
            }
        };
        let request_id = request_id(&request)?;
        {
            let mut requests = in_flight.lock().await;
            if requests
                .insert(
                    header.stream_id,
                    InFlightRequest {
                        request_id: request_id.clone(),
                        _permit: permit,
                    },
                )
                .is_some()
            {
                return Err(());
            }
        }
        match driver.start_ttrpc(request_id, frame).await {
            Ok(()) => {}
            Err(error) => {
                in_flight.lock().await.remove(&header.stream_id);
                responses
                    .send(session_error_response(header.stream_id, error))
                    .await
                    .map_err(|_| ())?;
                continue;
            }
        }
    }
}

struct InFlightRequest {
    request_id: RequestId,
    _permit: OwnedSemaphorePermit,
}

async fn receive_ttrpc_responses(
    driver: Arc<dyn ComponentSessionDriver>,
    responses: mpsc::Sender<Result<Vec<u8>, ()>>,
    in_flight: Arc<Mutex<BTreeMap<u32, InFlightRequest>>>,
) -> Result<(), ()> {
    loop {
        let frame = driver.receive_ttrpc().await.map_err(|_| ())?;
        let header = ttrpc_frame_header(&frame)?;
        if header.type_ != MESSAGE_TYPE_RESPONSE {
            return Err(());
        }
        let request = in_flight.lock().await.remove(&header.stream_id).ok_or(())?;
        if !driver
            .complete_ttrpc(request.request_id)
            .await
            .map_err(|_| ())?
        {
            return Err(());
        }
        responses.send(Ok(frame)).await.map_err(|_| ())?;
    }
}

async fn read_ttrpc_request<R>(
    reader: &mut R,
) -> Result<(MessageHeader, ttrpc::Request, Vec<u8>), ()>
where
    R: AsyncRead + Unpin,
{
    let mut header_bytes = [0_u8; MESSAGE_HEADER_LENGTH];
    reader.read_exact(&mut header_bytes).await.map_err(|_| ())?;
    let header = MessageHeader::from(header_bytes);
    if header.length as usize
        > d2b_contracts::v2_component_session::MAX_LOGICAL_MESSAGE_BYTES as usize
    {
        return Err(());
    }
    let mut body = vec![0_u8; header.length as usize];
    reader.read_exact(&mut body).await.map_err(|_| ())?;
    let request = ttrpc::Request::parse_from_bytes(&body).map_err(|_| ())?;
    let mut frame = header_bytes.to_vec();
    frame.extend_from_slice(&body);
    Ok((header, request, frame))
}

async fn dispatch_cancel_request(
    header: MessageHeader,
    request: ttrpc::Request,
    driver: Arc<dyn ComponentSessionDriver>,
    in_flight: &Mutex<BTreeMap<u32, InFlightRequest>>,
) -> Result<Vec<u8>, ()> {
    if request.method == "Cancel" && service_method(&request).is_some() {
        let cancel =
            decode_strict::<common::CancelRequest>(&request.payload, false).map_err(|_| ())?;
        let request_id = RequestId::new(cancel.request_id).map_err(|_| ())?;
        if !in_flight
            .lock()
            .await
            .values()
            .any(|request| request.request_id == request_id)
        {
            let mut response = common::CancelResponse::new();
            response.outcome =
                EnumOrUnknown::new(common::CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST);
            return ttrpc_response(header.stream_id, response.write_to_bytes().map_err(|_| ())?);
        }
        if let Err(error) = driver.cancel(cancel.session_generation, request_id).await {
            return session_error_response(header.stream_id, error);
        }
        let mut response = common::CancelResponse::new();
        response.outcome =
            EnumOrUnknown::new(common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED);
        return ttrpc_response(header.stream_id, response.write_to_bytes().map_err(|_| ())?);
    }
    Err(())
}

fn ttrpc_frame_header(frame: &[u8]) -> Result<MessageHeader, ()> {
    let header_bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())?;
    let header = MessageHeader::from(header_bytes);
    if header.length as usize != frame.len().saturating_sub(MESSAGE_HEADER_LENGTH) {
        return Err(());
    }
    Ok(header)
}

fn ttrpc_response(stream_id: u32, payload: Vec<u8>) -> Result<Vec<u8>, ()> {
    encode_ttrpc_response(
        stream_id,
        ttrpc::Response {
            payload,
            ..Default::default()
        },
    )
}

fn session_error_response(stream_id: u32, error: d2b_session::SessionError) -> Result<Vec<u8>, ()> {
    let code = match error.code() {
        SessionErrorCode::Cancelled => ttrpc::Code::CANCELLED,
        SessionErrorCode::DeadlineExpired
        | SessionErrorCode::DeadlineInvalid
        | SessionErrorCode::HandshakeTimeout => ttrpc::Code::DEADLINE_EXCEEDED,
        SessionErrorCode::QueueBackpressure | SessionErrorCode::ReassemblyLimitExceeded => {
            ttrpc::Code::RESOURCE_EXHAUSTED
        }
        SessionErrorCode::GenerationMismatch => ttrpc::Code::FAILED_PRECONDITION,
        SessionErrorCode::SessionDisconnected => ttrpc::Code::UNAVAILABLE,
        _ => ttrpc::Code::INTERNAL,
    };
    ttrpc_error_response(stream_id, code, error.code().as_str())
}

fn ttrpc_error_response(
    stream_id: u32,
    code: ttrpc::Code,
    reason: &'static str,
) -> Result<Vec<u8>, ()> {
    encode_ttrpc_response(
        stream_id,
        ttrpc::Response {
            status: MessageField::some(ttrpc::get_status(code, reason)),
            ..Default::default()
        },
    )
}

fn encode_ttrpc_response(stream_id: u32, response: ttrpc::Response) -> Result<Vec<u8>, ()> {
    let body = response.write_to_bytes().map_err(|_| ())?;
    let mut frame = Vec::from(MessageHeader::new_response(
        stream_id,
        u32::try_from(body.len()).map_err(|_| ())?,
    ));
    frame.extend_from_slice(&body);
    Ok(frame)
}

fn request_id(request: &ttrpc::Request) -> Result<RequestId, ()> {
    let service = service_method(request).ok_or(())?;
    let metadata = match (service.package, request.method.as_str()) {
        ("d2b.daemon.v2", "Exec" | "Shell" | "OpenConsole") => {
            decode_strict::<terminal::TerminalOpenRequest>(&request.payload, true)
                .map_err(|_| ())?
                .metadata
                .into_option()
                .ok_or(())?
        }
        ("d2b.guest.v2", "Exec") => {
            decode_strict::<guest::GuestExecRequest>(&request.payload, true)
                .map_err(|_| ())?
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.metadata.as_ref())
                .cloned()
                .ok_or(())?
        }
        ("d2b.guest.v2", "OpenShell") => {
            decode_strict::<guest::GuestOpenShellRequest>(&request.payload, true)
                .map_err(|_| ())?
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.metadata.as_ref())
                .cloned()
                .ok_or(())?
        }
        ("d2b.guest.v2", method) => guest_request_metadata(method, &request.payload)?,
        ("d2b.broker.v2", "Allocate") => {
            decode_strict::<broker::AllocateRequest>(&request.payload, true)
                .map_err(|_| ())?
                .metadata
                .into_option()
                .ok_or(())?
        }
        ("d2b.broker.v2", "Spawn") => {
            decode_strict::<broker::SpawnRealmChildrenRequest>(&request.payload, true)
                .map_err(|_| ())?
                .metadata
                .into_option()
                .ok_or(())?
        }
        (package, "Capabilities") if package.starts_with("d2b.provider.") => {
            decode_strict::<common::CapabilityRequest>(&request.payload, false)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .cloned()
                .ok_or(())?
        }
        (package, _) if package.starts_with("d2b.provider.") => {
            decode_strict::<common::ProviderRequest>(&request.payload, false)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .cloned()
                .ok_or(())?
        }
        _ => decode_strict::<common::ServiceRequest>(&request.payload, false)
            .map_err(|_| ())?
            .metadata
            .into_option()
            .ok_or(())?,
    };
    RequestId::new(metadata.request_id).map_err(|_| ())
}

fn service_method(
    request: &ttrpc::Request,
) -> Option<&'static d2b_contracts::v2_services::ServiceSpec> {
    SERVICE_INVENTORY.iter().find(|service| {
        request.service == format!("{}.{}", service.package, service.service)
            && service
                .methods
                .iter()
                .any(|method| method.name == request.method)
    })
}

fn guest_request_metadata(method: &str, payload: &[u8]) -> Result<common::RequestMetadata, ()> {
    macro_rules! context_metadata {
        ($message:ty, $requires_idempotency:expr) => {{
            decode_strict::<$message>(payload, $requires_idempotency)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .cloned()
                .ok_or(())
        }};
    }
    match method {
        "Bootstrap" => context_metadata!(guest::GuestBootstrapRequest, true),
        "Reconnect" => context_metadata!(guest::GuestReconnectRequest, true),
        "CancelExec" => context_metadata!(guest::GuestCancelExecRequest, true),
        "InspectExec" => context_metadata!(guest::GuestInspectExecRequest, false),
        "OpenExecRetainedLog" => {
            context_metadata!(guest::GuestOpenExecRetainedLogRequest, true)
        }
        "FileTransfer" => context_metadata!(guest::GuestFileTransferRequest, true),
        "SecurityKey" => context_metadata!(guest::GuestSecurityKeyRequest, true),
        "Shutdown" => context_metadata!(guest::GuestShutdownRequest, true),
        _ => Err(()),
    }
}

fn service_package(service: ServiceKind) -> ServicePackage {
    match service {
        ServiceKind::Daemon => ServicePackage::DaemonV2,
        ServiceKind::Realm => ServicePackage::RealmV2,
        ServiceKind::Guest => ServicePackage::GuestV2,
        ServiceKind::ProviderRuntime
        | ServiceKind::ProviderInfrastructure
        | ServiceKind::ProviderTransport
        | ServiceKind::ProviderSubstrate
        | ServiceKind::ProviderCredential
        | ServiceKind::ProviderDisplay
        | ServiceKind::ProviderNetwork
        | ServiceKind::ProviderStorage
        | ServiceKind::ProviderDevice
        | ServiceKind::ProviderAudio
        | ServiceKind::ProviderObservability => ServicePackage::ProviderV2,
        ServiceKind::Broker => ServicePackage::BrokerV2,
        ServiceKind::User => ServicePackage::UserV2,
        ServiceKind::RuntimeSystemdUser => ServicePackage::RuntimeSystemdUserV2,
        ServiceKind::Shell => ServicePackage::ShellV2,
        ServiceKind::Clipboard => ServicePackage::ClipboardV2,
        ServiceKind::ClipboardPicker => ServicePackage::ClipboardPickerV2,
        ServiceKind::Notify => ServicePackage::NotifyV2,
        ServiceKind::SecurityKey => ServicePackage::SecurityKeyV2,
        ServiceKind::Wayland => ServicePackage::WaylandV2,
        ServiceKind::Activation => ServicePackage::ActivationV2,
        ServiceKind::Tty => ServicePackage::TtyV2,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use async_trait::async_trait;
    use d2b_contracts::{
        v2_component_session::{CloseReason, Remediation, SessionErrorCode},
        v2_services::{StrictWireMessage, encode_strict},
    };
    use d2b_session::{
        Cancellation, OwnedAttachment, Result as SessionResult, SessionError, SessionEvent,
        StreamEvent, StreamId,
    };
    use d2b_session_unix::{PeerIdentityPolicy, negotiated_descriptor_policy_resolver};
    use protobuf::MessageField;
    use tokio::{io::AsyncWriteExt, sync::Notify};

    use super::*;
    use crate::{Client, RouteRecord, RouteTable, ServiceOwner, TargetInput, TransportSelection};

    const TEST_GENERATION: u64 = 23;

    struct BlockingDriver {
        started: AtomicUsize,
        completed: AtomicUsize,
        cancelled: AtomicUsize,
        progress: Notify,
        responses: mpsc::Sender<Vec<u8>>,
        response_receiver: Mutex<mpsc::Receiver<Vec<u8>>>,
    }

    impl BlockingDriver {
        fn new() -> Self {
            let (responses, response_receiver) = mpsc::channel(4);
            Self {
                started: AtomicUsize::new(0),
                completed: AtomicUsize::new(0),
                cancelled: AtomicUsize::new(0),
                progress: Notify::new(),
                responses,
                response_receiver: Mutex::new(response_receiver),
            }
        }

        async fn wait_for(&self, counter: &AtomicUsize, expected: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                while counter.load(Ordering::Acquire) < expected {
                    self.progress.notified().await;
                }
            })
            .await
            .unwrap();
        }
    }

    fn unsupported<T>() -> SessionResult<T> {
        Err(SessionError::new(SessionErrorCode::InternalInvariant))
    }

    #[async_trait]
    impl ComponentSessionDriver for BlockingDriver {
        fn generation(&self) -> u64 {
            TEST_GENERATION
        }

        async fn start_ttrpc(&self, _request_id: RequestId, _frame: Vec<u8>) -> SessionResult<()> {
            self.started.fetch_add(1, Ordering::AcqRel);
            self.progress.notify_waiters();
            Ok(())
        }

        async fn complete_ttrpc(&self, _request_id: RequestId) -> SessionResult<bool> {
            self.completed.fetch_add(1, Ordering::AcqRel);
            self.progress.notify_waiters();
            Ok(true)
        }

        async fn cancel(&self, generation: u64, _request_id: RequestId) -> SessionResult<()> {
            if generation != self.generation() {
                return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
            }
            self.cancelled.fetch_add(1, Ordering::AcqRel);
            self.progress.notify_waiters();
            Ok(())
        }

        async fn send_ttrpc(&self, _frame: Vec<u8>) -> SessionResult<()> {
            unsupported()
        }

        async fn receive_ttrpc(&self) -> SessionResult<Vec<u8>> {
            self.response_receiver
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| SessionError::new(SessionErrorCode::SessionDisconnected))
        }

        async fn register_inbound_call(
            &self,
            _request_id: RequestId,
        ) -> SessionResult<Cancellation> {
            unsupported()
        }

        async fn complete_inbound_call(&self, _request_id: RequestId) -> SessionResult<bool> {
            unsupported()
        }

        async fn remove_inbound_call(&self, _request_id: RequestId) -> SessionResult<bool> {
            unsupported()
        }

        async fn send_attachments(&self, _attachments: Vec<OwnedAttachment>) -> SessionResult<()> {
            unsupported()
        }

        async fn receive_attachments(&self) -> SessionResult<Vec<OwnedAttachment>> {
            unsupported()
        }

        async fn open_named_stream(
            &self,
            _stream: StreamId,
            _send_credit: u32,
            _receive_credit: u32,
        ) -> SessionResult<()> {
            unsupported()
        }

        async fn send_named_stream(&self, _stream: StreamId, _bytes: Vec<u8>) -> SessionResult<()> {
            unsupported()
        }

        async fn receive_named_stream(&self) -> SessionResult<StreamEvent> {
            unsupported()
        }

        async fn grant_named_stream_credit(
            &self,
            _stream: StreamId,
            _bytes: u32,
        ) -> SessionResult<()> {
            unsupported()
        }

        async fn close_named_stream(&self, _stream: StreamId) -> SessionResult<()> {
            unsupported()
        }

        async fn reset_named_stream(&self, _stream: StreamId) -> SessionResult<()> {
            unsupported()
        }

        async fn drive_keepalive(&self, _now: Instant) -> SessionResult<()> {
            unsupported()
        }

        async fn receive_control(&self) -> SessionResult<SessionEvent> {
            std::future::pending().await
        }

        async fn close(
            &self,
            _reason: CloseReason,
            _remediation: Remediation,
        ) -> SessionResult<()> {
            unsupported()
        }
    }

    fn request_frame(stream_id: u32, method: &str, payload: Vec<u8>) -> Vec<u8> {
        let request = ttrpc::Request {
            service: "d2b.daemon.v2.DaemonService".to_owned(),
            method: method.to_owned(),
            payload,
            ..Default::default()
        };
        let body = request.write_to_bytes().unwrap();
        let mut frame = Vec::from(MessageHeader::new_request(
            stream_id,
            u32::try_from(body.len()).unwrap(),
        ));
        frame.extend_from_slice(&body);
        frame
    }

    fn service_payload(request_id: Vec<u8>) -> Vec<u8> {
        let mut metadata = common::RequestMetadata::new();
        metadata.request_id = request_id;
        metadata.issued_at_unix_ms = 1;
        metadata.expires_at_unix_ms = 2;
        metadata.session_generation = TEST_GENERATION;
        let mut scope = common::IdentityScope::new();
        scope.realm_id = "aaaaaaaaaaaaaaaaaaaa".to_owned();
        let mut request = common::ServiceRequest::new();
        request.metadata = MessageField::some(metadata);
        request.scope = MessageField::some(scope);
        encode_strict(&request, false).unwrap()
    }

    fn cancel_payload(request_id: Vec<u8>) -> Vec<u8> {
        let mut request = common::CancelRequest::new();
        request.request_id = request_id;
        request.session_generation = TEST_GENERATION;
        request.validate_wire(false).unwrap();
        request.write_to_bytes().unwrap()
    }

    fn guest_context(request_id: [u8; 16], idempotent: bool) -> guest::GuestOperationContext {
        let mut metadata = common::RequestMetadata::new();
        metadata.request_id = request_id.to_vec();
        metadata.issued_at_unix_ms = 1;
        metadata.expires_at_unix_ms = 2;
        metadata.session_generation = TEST_GENERATION;
        if idempotent {
            metadata.idempotency_key = vec![8; 16];
        }
        let mut scope = common::IdentityScope::new();
        scope.realm_id = "aaaaaaaaaaaaaaaaaaaa".to_owned();
        scope.workload_id = "bbbbbbbbbbbbbbbbbbba".to_owned();
        guest::GuestOperationContext {
            metadata: MessageField::some(metadata),
            scope: MessageField::some(scope),
            operation_id: "operation-1".to_owned(),
            request_digest: vec![9; 32],
            ..Default::default()
        }
    }

    async fn read_response_frame(socket: &mut DuplexStream) -> Vec<u8> {
        let mut header = [0_u8; MESSAGE_HEADER_LENGTH];
        socket.read_exact(&mut header).await.unwrap();
        let parsed = MessageHeader::from(header);
        let mut body = vec![0_u8; parsed.length as usize];
        socket.read_exact(&mut body).await.unwrap();
        let mut frame = header.to_vec();
        frame.extend_from_slice(&body);
        frame
    }

    #[tokio::test]
    async fn ttrpc_pump_reads_cancel_while_invoke_is_pending() {
        let driver = Arc::new(BlockingDriver::new());
        let shared: Arc<dyn ComponentSessionDriver> = driver.clone();
        let (mut client, bridge) = tokio::io::duplex(64 * 1024);
        let pump = tokio::spawn(pump_ttrpc(bridge, shared));
        let request_id = vec![7; 16];

        client
            .write_all(&request_frame(
                1,
                "Inspect",
                service_payload(request_id.clone()),
            ))
            .await
            .unwrap();
        driver.wait_for(&driver.started, 1).await;
        client
            .write_all(&request_frame(2, "Cancel", cancel_payload(request_id)))
            .await
            .unwrap();
        driver.wait_for(&driver.cancelled, 1).await;

        assert_eq!(driver.started.load(Ordering::Acquire), 1);
        assert_eq!(driver.cancelled.load(Ordering::Acquire), 1);
        tokio::task::yield_now().await;
        assert!(!pump.is_finished());
        pump.abort();
    }

    #[test]
    fn request_ids_are_decoded_by_service_and_final_guest_method() {
        let request_id_bytes = [7; 16];
        let requests = [
            (
                "InspectExec",
                encode_strict(
                    &guest::GuestInspectExecRequest {
                        context: MessageField::some(guest_context(request_id_bytes, false)),
                        query: MessageField::some(guest::GuestInspectExecQuery {
                            query: Some(guest::guest_inspect_exec_query::Query::Status(
                                guest::GuestExecStatusQuery {
                                    resource_handle: "exec-1".to_owned(),
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    false,
                )
                .unwrap(),
            ),
            (
                "CancelExec",
                encode_strict(
                    &guest::GuestCancelExecRequest {
                        context: MessageField::some(guest_context(request_id_bytes, true)),
                        resource_handle: "exec-1".to_owned(),
                        control_sequence: 1,
                        reason: EnumOrUnknown::new(
                            guest::GuestExecCancelReason::GUEST_EXEC_CANCEL_REASON_USER_REQUESTED,
                        ),
                        ..Default::default()
                    },
                    true,
                )
                .unwrap(),
            ),
            (
                "OpenExecRetainedLog",
                encode_strict(
                    &guest::GuestOpenExecRetainedLogRequest {
                        context: MessageField::some(guest_context(request_id_bytes, true)),
                        resource_handle: "exec-1".to_owned(),
                        output: EnumOrUnknown::new(terminal::OutputStream::OUTPUT_STREAM_STDOUT),
                        max_bytes: 64,
                        ..Default::default()
                    },
                    true,
                )
                .unwrap(),
            ),
        ];
        for (method, payload) in requests {
            let request = ttrpc::Request {
                service: "d2b.guest.v2.GuestService".to_owned(),
                method: method.to_owned(),
                payload,
                ..Default::default()
            };
            assert_eq!(
                request_id(&request).unwrap().as_bytes(),
                request_id_bytes.as_slice()
            );
        }

        let mistyped = ttrpc::Request {
            service: "d2b.daemon.v2.DaemonService".to_owned(),
            method: "Inspect".to_owned(),
            payload: requests_forbidden_guest_payload(),
            ..Default::default()
        };
        assert!(request_id(&mistyped).is_err());
    }

    fn requests_forbidden_guest_payload() -> Vec<u8> {
        encode_strict(
            &guest::GuestInspectExecRequest {
                context: MessageField::some(guest_context([7; 16], false)),
                query: MessageField::some(guest::GuestInspectExecQuery {
                    query: Some(guest::guest_inspect_exec_query::Query::Status(
                        guest::GuestExecStatusQuery {
                            resource_handle: "exec-1".to_owned(),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
                ..Default::default()
            },
            false,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn saturated_pump_rejects_excess_work_but_still_reads_cancel() {
        let driver = Arc::new(BlockingDriver::new());
        let shared: Arc<dyn ComponentSessionDriver> = driver.clone();
        let (mut client, bridge) = tokio::io::duplex(64 * 1024);
        let (reader, _writer) = tokio::io::split(bridge);
        let (responses, mut response_receiver) = mpsc::channel(4);
        let in_flight = Arc::new(Mutex::new(BTreeMap::new()));
        let dispatcher = tokio::spawn(dispatch_ttrpc_requests(
            reader, shared, responses, 1, in_flight,
        ));
        let request_id = vec![8; 16];

        client
            .write_all(&request_frame(
                1,
                "Inspect",
                service_payload(request_id.clone()),
            ))
            .await
            .unwrap();
        driver.wait_for(&driver.started, 1).await;
        client
            .write_all(&request_frame(2, "Inspect", service_payload(vec![9; 16])))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), response_receiver.recv())
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
        client
            .write_all(&request_frame(3, "Cancel", cancel_payload(request_id)))
            .await
            .unwrap();
        driver.wait_for(&driver.cancelled, 1).await;

        assert!(!dispatcher.is_finished());
        dispatcher.abort();
    }

    #[tokio::test]
    async fn pump_forwards_out_of_order_responses_by_ttrpc_stream() {
        let driver = Arc::new(BlockingDriver::new());
        let shared: Arc<dyn ComponentSessionDriver> = driver.clone();
        let (mut client, bridge) = tokio::io::duplex(64 * 1024);
        let pump = tokio::spawn(pump_ttrpc(bridge, shared));

        client
            .write_all(&request_frame(11, "Inspect", service_payload(vec![1; 16])))
            .await
            .unwrap();
        client
            .write_all(&request_frame(12, "Inspect", service_payload(vec![2; 16])))
            .await
            .unwrap();
        driver.wait_for(&driver.started, 2).await;
        driver
            .responses
            .send(ttrpc_response(12, b"second".to_vec()).unwrap())
            .await
            .unwrap();
        driver
            .responses
            .send(ttrpc_response(11, b"first".to_vec()).unwrap())
            .await
            .unwrap();

        let second = read_response_frame(&mut client).await;
        let first = read_response_frame(&mut client).await;
        assert_eq!(ttrpc_frame_header(&second).unwrap().stream_id, 12);
        assert_eq!(ttrpc_frame_header(&first).unwrap().stream_id, 11);
        driver.wait_for(&driver.completed, 2).await;
        assert!(!pump.is_finished());
        pump.abort();
    }

    #[tokio::test]
    async fn connector_discovers_and_authenticates_the_driver_generation() {
        let generation = 41;
        let identity = local_daemon_endpoint_identity(1000, 100).unwrap();
        let policy = identity.with_generation(generation).unwrap();
        let (client_fd, server_fd) = nix::sys::socket::socketpair(
            nix::sys::socket::AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            nix::sys::socket::SockFlag::SOCK_CLOEXEC | nix::sys::socket::SockFlag::SOCK_NONBLOCK,
        )
        .unwrap();
        let uid = nix::unistd::Uid::effective().as_raw();
        let server_transport = DaemonSeqpacketTransport::new(server_fd, uid).unwrap();
        let server = tokio::spawn(async move {
            SessionEngine::establish_responder(
                HostTransport::Daemon(server_transport),
                policy,
                HandshakeCredentials::Nn,
                Instant::now(),
            )
            .await
            .unwrap()
        });
        let connector = HostSocketConnector::from_seqpacket_fd(
            client_fd,
            uid,
            identity,
            HandshakeCredentials::Nn,
        )
        .unwrap();
        let realm = d2b_contracts::v2_identity::RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap();
        let connected = Client::new(
            RouteTable::new(vec![RouteRecord {
                owner: ServiceOwner::LocalRoot(realm.clone()),
                transport: TransportKind::LocalUnix,
            }]),
            connector,
        )
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await
        .unwrap();
        assert_eq!(connected.session_generation(), generation);
        assert_eq!(server.await.unwrap().generation(), generation);
    }

    fn standalone_service_identity(
        package: ServicePackage,
        responder_role: EndpointRole,
        attachment_policy: AttachmentPolicy,
    ) -> EndpointPolicyIdentity {
        let service = SERVICE_INVENTORY
            .iter()
            .find(|service| service.package == package.as_str())
            .unwrap();
        EndpointPolicyIdentity {
            purpose: EndpointPurpose::RuntimeSystemdUser,
            purpose_class: PurposeClass::Local,
            initiator_role: EndpointRole::LocalRootController,
            responder_role,
            service: package,
            schema_fingerprint: service_schema_fingerprint(service),
            noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
            limits: LimitProfile::local_default(),
            transport_binding: TransportBinding {
                transport: TransportClass::UnixSeqpacket,
                locality: Locality::HostLocal,
                channel_binding: directional_channel_binding(
                    package,
                    TransportClass::UnixSeqpacket,
                    responder_role,
                    nix::unistd::Uid::effective().as_raw(),
                    nix::unistd::Gid::effective().as_raw(),
                ),
                identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
            },
            attachment_policy,
        }
    }

    #[test]
    fn identity_admits_service_gates_composition_members_to_the_frozen_composition_identity() {
        let uid = nix::unistd::Uid::effective().as_raw();
        let gid = nix::unistd::Gid::effective().as_raw();
        let composed = runtime_systemd_user_composition_endpoint_identity(uid, gid).unwrap();
        assert!(identity_admits_service(
            &composed,
            ServiceKind::RuntimeSystemdUser
        ));
        assert!(identity_admits_service(&composed, ServiceKind::Shell));
        assert!(identity_admits_service(&composed, ServiceKind::Tty));
        assert!(!identity_admits_service(&composed, ServiceKind::Daemon));

        // A standalone identity that only claims the composition's primary
        // service package, but with that single service's OWN (uncomposed)
        // fingerprint, must never be admitted for any composition member —
        // the fingerprint must be the exact bound composition, not merely a
        // matching primary tag.
        let runtime_only = standalone_service_identity(
            ServicePackage::RuntimeSystemdUserV2,
            EndpointRole::RuntimeSystemdUserAgent,
            AttachmentPolicy::disabled(),
        );
        assert_ne!(
            runtime_only.schema_fingerprint,
            RUNTIME_SYSTEMD_USER_COMPOSITION.schema_fingerprint()
        );
        assert!(!identity_admits_service(
            &runtime_only,
            ServiceKind::RuntimeSystemdUser
        ));
        assert!(!identity_admits_service(&runtime_only, ServiceKind::Shell));
        assert!(!identity_admits_service(&runtime_only, ServiceKind::Tty));

        // A standalone Shell-only identity must never be admitted for the
        // Shell composition member either: it is a completely different,
        // unauthenticated peer identity for this endpoint.
        let shell_only = standalone_service_identity(
            ServicePackage::ShellV2,
            EndpointRole::RuntimeSystemdUserAgent,
            AttachmentPolicy::disabled(),
        );
        assert!(!identity_admits_service(&shell_only, ServiceKind::Shell));
        assert!(!identity_admits_service(
            &shell_only,
            ServiceKind::RuntimeSystemdUser
        ));
    }

    #[tokio::test]
    async fn connect_rejects_a_standalone_single_service_identity_requesting_a_composition_member()
    {
        // The client's own routing check (`identity_admits_service`) must
        // reject a standalone Shell identity before any transport I/O, so
        // the (unconnected) server end of the pair is never read.
        let identity = standalone_service_identity(
            ServicePackage::ShellV2,
            EndpointRole::RuntimeSystemdUserAgent,
            AttachmentPolicy::disabled(),
        );
        let (client_fd, _server_fd) = nix::sys::socket::socketpair(
            nix::sys::socket::AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            nix::sys::socket::SockFlag::SOCK_CLOEXEC | nix::sys::socket::SockFlag::SOCK_NONBLOCK,
        )
        .unwrap();
        let uid = nix::unistd::Uid::effective().as_raw();
        let connector = HostSocketConnector::from_seqpacket_fd(
            client_fd,
            uid,
            identity,
            HandshakeCredentials::Nn,
        )
        .unwrap();
        let realm = d2b_contracts::v2_identity::RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap();
        let result = Client::new(
            RouteTable::new(vec![RouteRecord {
                owner: ServiceOwner::LocalRoot(realm.clone()),
                transport: TransportKind::LocalUnix,
            }]),
            connector,
        )
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::Shell,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await;
        assert!(matches!(result, Err(ClientError::InvalidService)));
    }

    /// A real client/server `UnixSeqpacketTransport` pair (a genuine
    /// `AF_UNIX SOCK_SEQPACKET` socketpair, negotiated peer credentials, and
    /// the exact per-connection [`negotiated_descriptor_policy_resolver`]
    /// resolver), suitable for exercising the composed `PacketAtomic`
    /// attachment policy end to end. `allowlist` is empty in every current
    /// caller: these tests exercise composition/session semantics, not
    /// attachment transfer, so the resolver is present (satisfying the
    /// transport's structural requirement) but never invoked.
    fn unix_seqpacket_transport_pair(
        policy: AttachmentPolicy,
    ) -> (UnixSeqpacketTransport, UnixSeqpacketTransport) {
        use d2b_session_unix::{CreditPool, CreditScopeSet, SeqpacketSocket};

        let (left, right) = nix::sys::socket::socketpair(
            nix::sys::socket::AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            nix::sys::socket::SockFlag::SOCK_CLOEXEC | nix::sys::socket::SockFlag::SOCK_NONBLOCK,
        )
        .unwrap();
        let client_socket = SeqpacketSocket::from_owned(left).unwrap();
        let server_socket = SeqpacketSocket::from_owned(right).unwrap();
        let client_peer = client_socket.acceptor_peer_credentials().unwrap();
        let server_peer = server_socket.acceptor_peer_credentials().unwrap();
        let scopes = || {
            let pool = || CreditPool::new(8).unwrap();
            CreditScopeSet::new(pool(), pool(), pool(), pool(), pool(), pool())
        };
        let generation = TEST_GENERATION;
        let client_resolver =
            negotiated_descriptor_policy_resolver(client_peer.uid(), generation, Vec::new())
                .unwrap();
        let server_resolver =
            negotiated_descriptor_policy_resolver(server_peer.uid(), generation, Vec::new())
                .unwrap();
        let client = UnixSeqpacketTransport::new(
            client_socket,
            Locality::HostLocal,
            LimitProfile::local_default(),
            policy,
            scopes(),
            client_resolver,
            PeerIdentityPolicy::accepted(client_peer),
        )
        .unwrap();
        let server = UnixSeqpacketTransport::new(
            server_socket,
            Locality::HostLocal,
            LimitProfile::local_default(),
            policy,
            scopes(),
            server_resolver,
            PeerIdentityPolicy::accepted(server_peer),
        )
        .unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn composed_client_identity_is_rejected_by_a_runtime_only_non_composed_responder() {
        // Real end-to-end proof, via the actual `SessionEngine` handshake
        // over a real `UnixSeqpacketTransport`, that a responder offering
        // only the standalone RuntimeSystemdUser service (not the frozen
        // 3-member composition) rejects a client negotiating the composed
        // identity: the wire-level fingerprints genuinely differ, not
        // merely the client's own local check.
        let generation = 7;
        let uid = nix::unistd::Uid::effective().as_raw();
        let gid = nix::unistd::Gid::effective().as_raw();
        let composed_identity =
            runtime_systemd_user_composition_endpoint_identity(uid, gid).unwrap();
        let runtime_only_policy = standalone_service_identity(
            ServicePackage::RuntimeSystemdUserV2,
            EndpointRole::RuntimeSystemdUserAgent,
            composed_identity.attachment_policy,
        )
        .with_generation(generation)
        .unwrap();
        let (client_transport, server_transport) =
            unix_seqpacket_transport_pair(composed_identity.attachment_policy);

        let server = tokio::spawn(async move {
            SessionEngine::establish_responder(
                server_transport,
                runtime_only_policy,
                HandshakeCredentials::Nn,
                Instant::now(),
            )
            .await
        });
        let connector = HostSocketConnector::new(
            client_transport,
            composed_identity,
            HandshakeCredentials::Nn,
        )
        .unwrap();
        let realm = d2b_contracts::v2_identity::RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap();
        let result = Client::new(
            RouteTable::new(vec![RouteRecord {
                owner: ServiceOwner::LocalRoot(realm.clone()),
                transport: TransportKind::LocalUnix,
            }]),
            connector,
        )
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::RuntimeSystemdUser,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await;
        // The responder rejects the mismatched schema before ever
        // completing the Noise handshake, so it never sends the client a
        // detailed reject reason over this unauthenticated pre-auth wire —
        // the client only observes the resulting disconnect. The
        // responder's own `SessionEngine::establish_responder` result is
        // the authoritative proof that the exact failure was a schema
        // mismatch between the composed client offer and the standalone
        // policy.
        assert!(result.is_err(), "{result:?}");
        assert_eq!(
            server.await.unwrap().unwrap_err().code(),
            SessionErrorCode::SchemaMismatch
        );
    }

    #[tokio::test]
    async fn connected_client_proxies_within_the_composition_but_rejects_outside_members() {
        // Real end-to-end proof (actual `SessionEngine` handshake over a
        // real `UnixSeqpacketTransport`) that the one authenticated
        // composition session for `RuntimeSystemdUser` reaches
        // `Shell`/`Tty` only via `runtime_systemd_user_composition_member_proxy`
        // on the SAME driver/generation, and rejects any non-member service.
        let generation = 53;
        let uid = nix::unistd::Uid::effective().as_raw();
        let gid = nix::unistd::Gid::effective().as_raw();
        let identity = runtime_systemd_user_composition_endpoint_identity(uid, gid).unwrap();
        let policy = identity.clone().with_generation(generation).unwrap();
        let (client_transport, server_transport) =
            unix_seqpacket_transport_pair(identity.attachment_policy);
        let server = tokio::spawn(async move {
            SessionEngine::establish_responder(
                server_transport,
                policy,
                HandshakeCredentials::Nn,
                Instant::now(),
            )
            .await
            .unwrap()
        });
        let connector =
            HostSocketConnector::new(client_transport, identity, HandshakeCredentials::Nn).unwrap();
        let realm = d2b_contracts::v2_identity::RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap();
        let connected = Client::new(
            RouteTable::new(vec![RouteRecord {
                owner: ServiceOwner::LocalRoot(realm.clone()),
                transport: TransportKind::LocalUnix,
            }]),
            connector,
        )
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::RuntimeSystemdUser,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await
        .unwrap();
        assert_eq!(connected.session_generation(), generation);
        assert_eq!(connected.service().kind(), ServiceKind::RuntimeSystemdUser);

        let shell = connected
            .runtime_systemd_user_composition_member_proxy(ServiceKind::Shell)
            .unwrap();
        assert_eq!(shell.service().kind(), ServiceKind::Shell);
        assert_eq!(shell.session_generation(), generation);

        let tty = connected
            .runtime_systemd_user_composition_member_proxy(ServiceKind::Tty)
            .unwrap();
        assert_eq!(tty.service().kind(), ServiceKind::Tty);
        assert_eq!(tty.session_generation(), generation);

        // Re-proxying from the Shell member back to RuntimeSystemdUser and
        // Tty must also succeed: composition membership, not the current
        // member, gates the proxy.
        let back = shell
            .runtime_systemd_user_composition_member_proxy(ServiceKind::RuntimeSystemdUser)
            .unwrap();
        assert_eq!(back.service().kind(), ServiceKind::RuntimeSystemdUser);

        assert!(matches!(
            connected.runtime_systemd_user_composition_member_proxy(ServiceKind::Daemon),
            Err(ClientError::TransportPolicyMismatch)
        ));
        assert!(matches!(
            shell.runtime_systemd_user_composition_member_proxy(ServiceKind::Guest),
            Err(ClientError::TransportPolicyMismatch)
        ));

        assert_eq!(server.await.unwrap().generation(), generation);
    }

    #[test]
    fn host_socket_feature_provides_a_tokio_io_driver() {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .expect("host-socket must enable Tokio I/O");
    }

    #[tokio::test]
    async fn daemon_transport_rejects_ancillary_data_and_oversized_packets() {
        use std::io::IoSlice;

        use nix::sys::socket::{AddressFamily, ControlMessage, SockFlag, sendmsg, socketpair};

        let pair = || {
            socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            )
            .unwrap()
        };
        let (receiver, _sender) = pair();
        assert_eq!(
            DaemonSeqpacketTransport::new(
                receiver,
                nix::unistd::Uid::effective().as_raw().wrapping_add(1),
            )
            .unwrap_err(),
            ClientError::TransportPolicyMismatch
        );
        let (receiver, sender) = pair();
        let (passed, _peer) = pair();
        sendmsg::<()>(
            sender.as_raw_fd(),
            &[IoSlice::new(b"protected")],
            &[ControlMessage::ScmRights(&[passed.as_raw_fd()])],
            MsgFlags::empty(),
            None,
        )
        .unwrap();
        let uid = nix::unistd::Uid::effective().as_raw();
        let mut transport = DaemonSeqpacketTransport::new(receiver, uid).unwrap();
        assert_eq!(
            transport.receive(64).await.unwrap_err(),
            TransportError::InvalidAttachment
        );

        let (receiver, sender) = pair();
        send(sender.as_raw_fd(), &[7; 128], MsgFlags::empty()).unwrap();
        let mut transport = DaemonSeqpacketTransport::new(receiver, uid).unwrap();
        assert_eq!(
            transport.receive(16).await.unwrap_err(),
            TransportError::LimitExceeded
        );
    }
}
