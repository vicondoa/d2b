use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use d2b_client::{
    AttachmentPayload, AttachmentValidationError, CallOptions, CancellationToken, Client,
    ClientError, ComponentSessionConnector, ConnectedClient, ConnectedSession, DaemonClient,
    DaemonMethod, GuestCancelCall, GuestClient, GuestInspectCall, GuestOperation,
    GuestRetainedLogCall, MetadataInput, OwnedAttachment, OwnedTransport, RemoteErrorKind,
    Response, RetryClass, RetryPolicy, RouteRecord, RouteTable, ServiceKind, ServiceOwner,
    SessionFailure, SharedDriver, TargetInput, TargetResolver, TransportKind, TransportPacket,
    TransportSelection, WallClock,
};
use d2b_contracts::{
    v2_component_session::{
        AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
        AttachmentPolicy, AttachmentPurpose, BoundedVec, CloseReason, EndpointPolicy,
        EndpointPurpose, EndpointRole, HandshakeOffer, IdentityEvidenceRequirement,
        KernelObjectType, LimitProfile, Locality, NoiseProfile, PurposeClass, Remediation,
        RequestId, ServicePackage, SessionErrorCode, TransportBinding, TransportClass,
    },
    v2_identity::{ProviderId, RealmId, WorkloadId, WorkloadName},
    v2_services::{
        SERVICE_INVENTORY, ServiceContractError, common, daemon, decode_strict, encode_strict,
        guest, terminal,
    },
};
use d2b_session::{
    Cancellation, HandshakeCredentials, Result as SessionResult, SessionEngine, SessionEvent,
    StreamEvent, StreamId, TransportDescriptor, TransportError,
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Notify, mpsc},
};
use ttrpc::r#async::transport::Socket;
use ttrpc::proto::{MESSAGE_HEADER_LENGTH, MessageHeader};

const NOW: u64 = 1_800_000_000_000;
const GENERATION: u64 = 17;

#[derive(Debug)]
struct FixedClock;

impl WallClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        NOW
    }
}

struct FakeTransport {
    sender: mpsc::Sender<TransportPacket>,
    receiver: mpsc::Receiver<TransportPacket>,
}

#[async_trait]
impl OwnedTransport for FakeTransport {
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            packet_atomic: true,
            supports_attachments: true,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        let packet = self
            .receiver
            .recv()
            .await
            .ok_or(TransportError::Disconnected)?;
        if packet.as_bytes().len() > protected_limit {
            return Err(TransportError::LimitExceeded);
        }
        Ok(packet)
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        self.sender
            .send(packet)
            .await
            .map_err(|_| TransportError::Disconnected)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

fn transport_pair() -> (FakeTransport, FakeTransport) {
    let (a_tx, a_rx) = mpsc::channel(128);
    let (b_tx, b_rx) = mpsc::channel(128);
    (
        FakeTransport {
            sender: a_tx,
            receiver: b_rx,
        },
        FakeTransport {
            sender: b_tx,
            receiver: a_rx,
        },
    )
}

fn offer() -> HandshakeOffer {
    HandshakeOffer {
        purpose: EndpointPurpose::PrivilegedBroker,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::LocalRootController,
        responder_role: EndpointRole::LocalRootBroker,
        service: ServicePackage::DaemonV2,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: GENERATION,
        attachment_policy: AttachmentPolicy {
            kind: d2b_session::contract::AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 8,
            max_per_request: 8,
            max_per_operation: 8,
            max_per_session: 32,
            credentials_allowed: false,
        },
    }
}

fn policy(value: &HandshakeOffer) -> EndpointPolicy {
    EndpointPolicy {
        purpose: value.purpose,
        purpose_class: value.purpose_class,
        initiator_role: value.initiator_role,
        responder_role: value.responder_role,
        service: value.service,
        schema_fingerprint: value.schema_fingerprint,
        noise_profile: value.noise_profile,
        limits: value.limits,
        transport_binding: value.transport_binding,
        reconnect_generation: value.reconnect_generation,
        attachment_policy: value.attachment_policy,
    }
}

async fn drivers() -> (SharedDriver, SharedDriver) {
    let (initiator_transport, responder_transport) = transport_pair();
    let session_offer = offer();
    let now = Instant::now();
    let (initiator, responder) = tokio::join!(
        SessionEngine::establish_initiator(
            initiator_transport,
            policy(&session_offer),
            HandshakeCredentials::Nn,
            now,
        ),
        SessionEngine::establish_responder(
            responder_transport,
            policy(&session_offer),
            HandshakeCredentials::Nn,
            now,
        )
    );
    (
        Arc::new(initiator.unwrap().into_driver()),
        Arc::new(responder.unwrap().into_driver()),
    )
}

#[derive(Default)]
struct FakeState {
    attempts: AtomicUsize,
    calls: AtomicUsize,
    cancellations: AtomicUsize,
    failures: Mutex<VecDeque<SessionFailure>>,
    seen: Mutex<Vec<SeenCall>>,
    response_override: Mutex<Option<common::ServiceResponse>>,
    typed_responses: Mutex<BTreeMap<String, VecDeque<Vec<u8>>>>,
    response_attachments: Mutex<VecDeque<Vec<OwnedAttachment>>>,
    stream_id: Mutex<Option<String>>,
    stream_sent: Mutex<Vec<Vec<u8>>>,
    stream_received: Mutex<VecDeque<Vec<u8>>>,
    stream_send_started: AtomicUsize,
    stream_send_completed: AtomicUsize,
    stream_progress: Notify,
    stream_event: Mutex<Option<RemoteStreamEvent>>,
    terminal_script: AtomicBool,
    terminal_open_generation_delta: AtomicUsize,
    granted_stream_credit: Mutex<Vec<u32>>,
    closed: AtomicUsize,
    stream_cancelled: AtomicUsize,
    block: AtomicBool,
    blocker: Notify,
    fail_connect: AtomicBool,
}

#[derive(Clone, Copy)]
enum RemoteStreamEvent {
    Close,
    Reset,
    WrongStream,
    InterleavedStreams,
}

#[derive(Debug)]
struct SeenCall {
    generation: u64,
    request_id: Vec<u8>,
    idempotency_key: Vec<u8>,
    timeout_nanos: u64,
    attachment_count: usize,
}

#[derive(Clone)]
struct FakeConnector(Arc<FakeState>);

impl FakeConnector {
    fn new() -> Self {
        Self(Arc::new(FakeState::default()))
    }

    fn fail_once(&self, failure: SessionFailure) {
        self.0.failures.lock().unwrap().push_back(failure);
    }

    fn respond_typed<M: d2b_contracts::v2_services::StrictWireMessage>(
        &self,
        method: &str,
        response: &M,
    ) {
        self.0
            .typed_responses
            .lock()
            .unwrap()
            .entry(method.to_owned())
            .or_default()
            .push_back(encode_strict(response, false).unwrap());
    }
}

#[async_trait]
impl ComponentSessionConnector for FakeConnector {
    async fn connect(
        &self,
        _target: &d2b_client::ResolvedTarget,
        _service: ServiceKind,
    ) -> Result<ConnectedSession, ClientError> {
        self.0.attempts.fetch_add(1, Ordering::SeqCst);
        if self.0.fail_connect.load(Ordering::Acquire) {
            return Err(ClientError::ConnectFailed);
        }
        let (initiator, responder) = drivers().await;
        let client_driver: SharedDriver = Arc::new(GrantRecordingDriver {
            inner: Arc::clone(&initiator),
            state: Arc::clone(&self.0),
        });
        let (client, peer) = tokio::io::duplex(4 * 1024 * 1024);
        tokio::spawn(ttrpc_bridge(
            peer,
            Arc::clone(&initiator),
            Arc::clone(&self.0),
        ));
        tokio::spawn(remote_ttrpc(Arc::clone(&responder), Arc::clone(&self.0)));
        tokio::spawn(remote_streams(Arc::clone(&responder), Arc::clone(&self.0)));
        tokio::spawn(remote_controls(responder, Arc::clone(&self.0)));
        Ok(ConnectedSession {
            driver: client_driver,
            ttrpc_socket: Socket::new(client),
            limits: d2b_contracts::v2_component_session::LimitProfile::local_default(),
        })
    }
}

struct GrantRecordingDriver {
    inner: SharedDriver,
    state: Arc<FakeState>,
}

#[async_trait]
impl d2b_session::ComponentSessionDriver for GrantRecordingDriver {
    fn generation(&self) -> u64 {
        self.inner.generation()
    }

    async fn start_ttrpc(&self, request_id: RequestId, frame: Vec<u8>) -> SessionResult<()> {
        self.inner.start_ttrpc(request_id, frame).await
    }

    async fn complete_ttrpc(&self, request_id: RequestId) -> SessionResult<bool> {
        self.inner.complete_ttrpc(request_id).await
    }

    async fn cancel(&self, generation: u64, request_id: RequestId) -> SessionResult<()> {
        self.inner.cancel(generation, request_id).await
    }

    async fn send_ttrpc(&self, frame: Vec<u8>) -> SessionResult<()> {
        self.inner.send_ttrpc(frame).await
    }

    async fn receive_ttrpc(&self) -> SessionResult<Vec<u8>> {
        self.inner.receive_ttrpc().await
    }

    async fn register_inbound_call(&self, request_id: RequestId) -> SessionResult<Cancellation> {
        self.inner.register_inbound_call(request_id).await
    }

    async fn complete_inbound_call(&self, request_id: RequestId) -> SessionResult<bool> {
        self.inner.complete_inbound_call(request_id).await
    }

    async fn remove_inbound_call(&self, request_id: RequestId) -> SessionResult<bool> {
        self.inner.remove_inbound_call(request_id).await
    }

    async fn send_attachments(&self, attachments: Vec<OwnedAttachment>) -> SessionResult<()> {
        self.inner.send_attachments(attachments).await
    }

    async fn receive_attachments(&self) -> SessionResult<Vec<OwnedAttachment>> {
        self.inner.receive_attachments().await
    }

    async fn open_named_stream(
        &self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> SessionResult<()> {
        self.inner
            .open_named_stream(stream, send_credit, receive_credit)
            .await
    }

    async fn send_named_stream(&self, stream: StreamId, bytes: Vec<u8>) -> SessionResult<()> {
        self.inner.send_named_stream(stream, bytes).await
    }

    async fn receive_named_stream(&self) -> SessionResult<StreamEvent> {
        self.inner.receive_named_stream().await
    }

    async fn grant_named_stream_credit(&self, stream: StreamId, bytes: u32) -> SessionResult<()> {
        self.inner.grant_named_stream_credit(stream, bytes).await?;
        self.state.granted_stream_credit.lock().unwrap().push(bytes);
        Ok(())
    }

    async fn close_named_stream(&self, stream: StreamId) -> SessionResult<()> {
        self.inner.close_named_stream(stream).await
    }

    async fn reset_named_stream(&self, stream: StreamId) -> SessionResult<()> {
        self.inner.reset_named_stream(stream).await
    }

    async fn drive_keepalive(&self, now: Instant) -> SessionResult<()> {
        self.inner.drive_keepalive(now).await
    }

    async fn receive_control(&self) -> SessionResult<SessionEvent> {
        self.inner.receive_control().await
    }

    async fn close(&self, reason: CloseReason, remediation: Remediation) -> SessionResult<()> {
        self.inner.close(reason, remediation).await
    }
}

async fn ttrpc_bridge(
    mut socket: DuplexStream,
    driver: SharedDriver,
    state: Arc<FakeState>,
) -> Result<(), ()> {
    loop {
        let mut header_bytes = [0_u8; MESSAGE_HEADER_LENGTH];
        socket.read_exact(&mut header_bytes).await.map_err(|_| ())?;
        let header = MessageHeader::from(header_bytes);
        let mut body = vec![0_u8; header.length as usize];
        socket.read_exact(&mut body).await.map_err(|_| ())?;
        let request = ttrpc::Request::parse_from_bytes(&body).map_err(|_| ())?;
        let metadata = request_metadata(&request)?;
        let request_id = RequestId::new(metadata.request_id.clone()).map_err(|_| ())?;
        let mut frame = header_bytes.to_vec();
        frame.extend_from_slice(&body);
        driver
            .start_ttrpc(request_id.clone(), frame)
            .await
            .map_err(|_| ())?;
        let reply = driver.receive_ttrpc().await.map_err(|_| ())?;
        if !driver.complete_ttrpc(request_id).await.map_err(|_| ())? {
            return Err(());
        }
        socket.write_all(&reply).await.map_err(|_| ())?;
        state.calls.fetch_add(1, Ordering::SeqCst);
    }
}

async fn remote_ttrpc(driver: SharedDriver, state: Arc<FakeState>) -> Result<(), ()> {
    loop {
        let frame = driver.receive_ttrpc().await.map_err(|_| ())?;
        if frame.len() < MESSAGE_HEADER_LENGTH {
            return Err(());
        }
        let header = MessageHeader::from(&frame[..MESSAGE_HEADER_LENGTH]);
        let request =
            ttrpc::Request::parse_from_bytes(&frame[MESSAGE_HEADER_LENGTH..]).map_err(|_| ())?;
        let service_request = (!matches!(
            request.method.as_str(),
            "Exec" | "Shell" | "OpenConsole" | "CancelExec" | "InspectExec" | "OpenExecRetainedLog"
        ))
        .then(|| decode_strict::<common::ServiceRequest>(&request.payload, false))
        .transpose()
        .map_err(|_| ())?;
        let metadata = request_metadata(&request)?;
        state.seen.lock().unwrap().push(SeenCall {
            generation: metadata.session_generation,
            request_id: metadata.request_id.clone(),
            idempotency_key: metadata.idempotency_key.clone(),
            timeout_nanos: u64::try_from(request.timeout_nano).unwrap_or_default(),
            attachment_count: service_request
                .as_ref()
                .map_or(0, |request| request.attachment_indexes.len()),
        });
        if service_request
            .as_ref()
            .is_some_and(|request| !request.attachment_indexes.is_empty())
        {
            drop(driver.receive_attachments().await.map_err(|_| ())?);
        }
        if state.block.load(Ordering::Acquire) {
            state.blocker.notified().await;
        }

        let typed_response = state
            .typed_responses
            .lock()
            .unwrap()
            .get_mut(&request.method)
            .and_then(VecDeque::pop_front);
        let terminal_response =
            if matches!(request.method.as_str(), "Exec" | "Shell" | "OpenConsole") {
                let open = decode_strict::<terminal::TerminalOpenRequest>(&request.payload, true)
                    .map_err(|_| ())?;
                let mut response = terminal::TerminalOpenResponse::new();
                response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED);
                response.operation_id = open.operation_id;
                response.stream_id = state
                    .stream_id
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| "stream-256".to_owned());
                response.session_generation = metadata.session_generation
                    + state.terminal_open_generation_delta.load(Ordering::Acquire) as u64;
                response.request_id = metadata.request_id.clone();
                response.resource_handle = "resource-1".to_owned();
                Some(response.write_to_bytes().map_err(|_| ())?)
            } else {
                None
            };
        let mut response = if matches!(
            state.failures.lock().unwrap().pop_front(),
            Some(SessionFailure::Retryable)
        ) {
            let mut response = common::ServiceResponse::new();
            response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_FAILED);
            let mut error = common::ErrorEnvelope::new();
            error.kind = EnumOrUnknown::new(common::ErrorKind::ERROR_KIND_UNAVAILABLE);
            error.retry = EnumOrUnknown::new(common::RetryClass::RETRY_CLASS_SAME_OPERATION);
            response.error = MessageField::some(error);
            response
        } else {
            state
                .response_override
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| {
                    let mut response = common::ServiceResponse::new();
                    response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
                    response.stream_id =
                        state.stream_id.lock().unwrap().clone().unwrap_or_default();
                    response
                })
        };
        let attachments = state
            .response_attachments
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        response.attachment_indexes = (0..attachments.len())
            .map(|index| u32::try_from(index).unwrap())
            .collect();
        if !attachments.is_empty() {
            driver.send_attachments(attachments).await.map_err(|_| ())?;
        }
        let payload = typed_response
            .or(terminal_response)
            .unwrap_or(encode_strict(&response, false).map_err(|_| ())?);
        let ttrpc_response = ttrpc::Response {
            payload,
            ..Default::default()
        };
        let body = ttrpc_response.write_to_bytes().map_err(|_| ())?;
        let mut reply = Vec::from(MessageHeader::new_response(
            header.stream_id,
            u32::try_from(body.len()).map_err(|_| ())?,
        ));
        reply.extend_from_slice(&body);
        driver.send_ttrpc(reply).await.map_err(|_| ())?;
    }
}

async fn remote_streams(driver: SharedDriver, state: Arc<FakeState>) -> Result<(), ()> {
    let stream = StreamId::new(0x0100).map_err(|_| ())?;
    driver
        .open_named_stream(stream, 256 * 1024, 256 * 1024)
        .await
        .map_err(|_| ())?;
    loop {
        match driver.receive_named_stream().await.map_err(|_| ())? {
            StreamEvent::Data {
                stream: received,
                bytes,
            } if received == stream => {
                state.stream_sent.lock().unwrap().push(bytes.clone());
                driver
                    .grant_named_stream_credit(stream, u32::try_from(bytes.len()).map_err(|_| ())?)
                    .await
                    .map_err(|_| ())?;
                if state.terminal_script.load(Ordering::Acquire)
                    && let Ok(frame) = decode_strict::<terminal::TerminalStreamFrame>(&bytes, false)
                {
                    respond_terminal_frame(&driver, stream, &frame).await?;
                    continue;
                }
                let scripted_event = { state.stream_event.lock().unwrap().take() };
                match scripted_event {
                    Some(RemoteStreamEvent::Close) => {
                        driver.close_named_stream(stream).await.map_err(|_| ())?;
                        return Ok(());
                    }

                    Some(RemoteStreamEvent::Reset) => {
                        driver.reset_named_stream(stream).await.map_err(|_| ())?;
                        return Ok(());
                    }
                    Some(RemoteStreamEvent::WrongStream) => {
                        let wrong = StreamId::new(0x0101).map_err(|_| ())?;
                        driver
                            .open_named_stream(wrong, 256 * 1024, 256 * 1024)
                            .await
                            .map_err(|_| ())?;
                        driver
                            .send_named_stream(wrong, b"wrong-stream".to_vec())
                            .await
                            .map_err(|_| ())?;
                        return Ok(());
                    }
                    Some(RemoteStreamEvent::InterleavedStreams) => {
                        let other = StreamId::new(0x0101).map_err(|_| ())?;
                        driver
                            .open_named_stream(other, 256 * 1024, 256 * 1024)
                            .await
                            .map_err(|_| ())?;
                        driver
                            .send_named_stream(other, b"other-stream".to_vec())
                            .await
                            .map_err(|_| ())?;
                        driver
                            .send_named_stream(stream, b"primary-stream".to_vec())
                            .await
                            .map_err(|_| ())?;
                        return Ok(());
                    }
                    None => {}
                }
                loop {
                    let reply = state.stream_received.lock().unwrap().pop_front();
                    let Some(reply) = reply else {
                        break;
                    };
                    state.stream_send_started.fetch_add(1, Ordering::SeqCst);
                    state.stream_progress.notify_waiters();
                    driver
                        .send_named_stream(stream, reply)
                        .await
                        .map_err(|_| ())?;
                    state.stream_send_completed.fetch_add(1, Ordering::SeqCst);
                    state.stream_progress.notify_waiters();
                }
            }

            StreamEvent::RemoteClosed { stream: received } if received == stream => {
                state.closed.fetch_add(1, Ordering::SeqCst);
            }
            StreamEvent::Reset { stream: received } if received == stream => {
                state.stream_cancelled.fetch_add(1, Ordering::SeqCst);
            }
            _ => return Err(()),
        }
    }
}

fn request_metadata(request: &ttrpc::Request) -> Result<common::RequestMetadata, ()> {
    match request.method.as_str() {
        "Exec" | "Shell" | "OpenConsole" => {
            let open = decode_strict::<terminal::TerminalOpenRequest>(&request.payload, true)
                .map_err(|_| ())?;
            open.metadata.into_option().ok_or(())
        }
        "CancelExec" => decode_strict::<guest::GuestCancelExecRequest>(&request.payload, true)
            .map_err(|_| ())?
            .context
            .as_ref()
            .and_then(|context| context.metadata.as_ref())
            .cloned()
            .ok_or(()),
        "InspectExec" => decode_strict::<guest::GuestInspectExecRequest>(&request.payload, false)
            .map_err(|_| ())?
            .context
            .as_ref()
            .and_then(|context| context.metadata.as_ref())
            .cloned()
            .ok_or(()),
        "OpenExecRetainedLog" => {
            decode_strict::<guest::GuestOpenExecRetainedLogRequest>(&request.payload, true)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .cloned()
                .ok_or(())
        }
        _ => {
            let service =
                decode_strict::<common::ServiceRequest>(&request.payload, false).map_err(|_| ())?;
            service.metadata.into_option().ok_or(())
        }
    }
}

async fn respond_terminal_frame(
    driver: &SharedDriver,
    stream: StreamId,
    request: &terminal::TerminalStreamFrame,
) -> Result<(), ()> {
    use terminal::terminal_stream_frame::Frame;
    if let Some(Frame::Select(terminal::TerminalSelection {
        selection: Some(terminal::terminal_selection::Selection::Shell(shell)),
        ..
    })) = request.frame.as_ref()
    {
        let action = shell.action.enum_value_or_default();
        if matches!(
            action,
            terminal::ShellAction::SHELL_ACTION_LIST
                | terminal::ShellAction::SHELL_ACTION_DETACH
                | terminal::ShellAction::SHELL_ACTION_KILL
        ) {
            let list = action == terminal::ShellAction::SHELL_ACTION_LIST;
            let result = terminal::ShellManagementResult {
                action: EnumOrUnknown::new(action),
                sessions: vec![terminal::ShellSession {
                    shell_handle: "shell-1".to_owned(),
                    state: EnumOrUnknown::new(
                        terminal::ShellSessionState::SHELL_SESSION_STATE_DETACHED,
                    ),
                    is_default: true,
                    ..Default::default()
                }],
                affected_shell_handle: if list {
                    String::new()
                } else {
                    shell.shell_handle.clone()
                },
                applied: !list,
                ..Default::default()
            };
            for (sequence, frame) in [
                (0, Frame::ShellResult(result)),
                (
                    1,
                    Frame::Outcome(terminal::TerminalOutcome {
                        outcome: Some(terminal::terminal_outcome::Outcome::Closed(
                            terminal::TerminalClosed::new(),
                        )),
                        ..Default::default()
                    }),
                ),
            ] {
                let response = terminal::TerminalStreamFrame {
                    session_generation: request.session_generation,
                    request_id: request.request_id.clone(),
                    sequence,
                    operation_id: request.operation_id.clone(),
                    resource_handle: request.resource_handle.clone(),
                    frame: Some(frame),
                    ..Default::default()
                };
                driver
                    .send_named_stream(stream, encode_strict(&response, false).map_err(|_| ())?)
                    .await
                    .map_err(|_| ())?;
            }
            return Ok(());
        }
    }
    let response = match request.frame.as_ref().ok_or(())? {
        Frame::Select(selection) => {
            let (kind, tty) = match selection.selection.as_ref().ok_or(())? {
                terminal::terminal_selection::Selection::Exec(exec) => {
                    (terminal::TerminalKind::TERMINAL_KIND_EXEC, exec.tty)
                }
                terminal::terminal_selection::Selection::Shell(_) => {
                    (terminal::TerminalKind::TERMINAL_KIND_SHELL, true)
                }
                terminal::terminal_selection::Selection::Console(_) => {
                    (terminal::TerminalKind::TERMINAL_KIND_CONSOLE, true)
                }
                _ => return Err(()),
            };
            Frame::Started(terminal::TerminalStarted {
                kind: EnumOrUnknown::new(kind),
                tty,
                ..Default::default()
            })
        }
        Frame::Stdin(stdin) => Frame::Status(terminal::TerminalStatus {
            status: EnumOrUnknown::new(
                terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED,
            ),
            next_stdin_offset: stdin.offset + stdin.data.len() as u64,
            ..Default::default()
        }),
        Frame::Cancel(_) => Frame::Outcome(terminal::TerminalOutcome {
            outcome: Some(terminal::terminal_outcome::Outcome::Cancelled(
                terminal::TerminalCancelled::new(),
            )),
            ..Default::default()
        }),
        Frame::Detach(_) => Frame::Outcome(terminal::TerminalOutcome {
            outcome: Some(terminal::terminal_outcome::Outcome::Detached(
                terminal::TerminalDetached::new(),
            )),
            ..Default::default()
        }),
        Frame::Close(_) => Frame::Outcome(terminal::TerminalOutcome {
            outcome: Some(terminal::terminal_outcome::Outcome::Closed(
                terminal::TerminalClosed::new(),
            )),
            ..Default::default()
        }),
        _ => return Ok(()),
    };
    let sequence = match request.frame.as_ref() {
        Some(Frame::Select(_)) => 0,
        Some(Frame::Stdin(_)) => 1,
        _ => 2,
    };
    let response = terminal::TerminalStreamFrame {
        session_generation: request.session_generation,
        request_id: request.request_id.clone(),
        sequence,
        operation_id: request.operation_id.clone(),
        resource_handle: request.resource_handle.clone(),
        frame: Some(response),
        ..Default::default()
    };
    driver
        .send_named_stream(stream, encode_strict(&response, false).map_err(|_| ())?)
        .await
        .map_err(|_| ())
}

async fn remote_controls(driver: SharedDriver, state: Arc<FakeState>) -> Result<(), ()> {
    loop {
        if matches!(
            driver.receive_control().await.map_err(|_| ())?,
            d2b_session::SessionEvent::CancelRequest(_)
        ) {
            state.cancellations.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn ids() -> (RealmId, WorkloadId, ProviderId) {
    (
        RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
        WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        ProviderId::parse("ccccccccccccccccccca").unwrap(),
    )
}

fn routes() -> RouteTable {
    let (realm, workload, provider) = ids();
    RouteTable::new(vec![
        RouteRecord {
            owner: ServiceOwner::LocalRoot(realm.clone()),
            transport: TransportKind::LocalUnix,
        },
        RouteRecord {
            owner: ServiceOwner::Realm(realm.clone()),
            transport: TransportKind::Provider,
        },
        RouteRecord {
            owner: ServiceOwner::Workload {
                realm: realm.clone(),
                workload,
            },
            transport: TransportKind::NativeVsock,
        },
        RouteRecord {
            owner: ServiceOwner::Provider { realm, provider },
            transport: TransportKind::InheritedSocket,
        },
    ])
}

fn options(mutating: bool) -> CallOptions {
    let metadata = MetadataInput::new([7; 16], NOW, NOW + 30_000)
        .unwrap()
        .with_correlation("correlation-1")
        .unwrap()
        .with_trace([8; 16])
        .unwrap();
    CallOptions {
        metadata: if mutating {
            metadata.with_idempotency(vec![9; 32]).unwrap()
        } else {
            metadata
        },
        retry: RetryPolicy::new(3).unwrap(),
    }
}

async fn daemon_client() -> (FakeConnector, ConnectedClient) {
    daemon_client_with(FakeConnector::new()).await
}

async fn daemon_client_with(connector: FakeConnector) -> (FakeConnector, ConnectedClient) {
    let (realm, _, _) = ids();
    let client = Client::with_clock(routes(), connector.clone(), FixedClock);
    let connected = client
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await
        .unwrap();
    (connector, connected)
}

async fn guest_client() -> (FakeConnector, GuestClient) {
    let connector = FakeConnector::new();
    let (realm, workload, _) = ids();
    let connected = Client::with_clock(routes(), connector.clone(), FixedClock)
        .connect(
            TargetInput::Workload { realm, workload },
            ServiceKind::Guest,
            TransportSelection::exact(TransportKind::NativeVsock),
        )
        .await
        .unwrap();
    (connector, GuestClient::new(connected).unwrap())
}

fn guest_list_query() -> guest::GuestInspectExecQuery {
    guest::GuestInspectExecQuery {
        query: Some(guest::guest_inspect_exec_query::Query::ListPage(
            guest::GuestExecListPageQuery {
                page_size: 32,
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

fn guest_operation(id: &str) -> GuestOperation {
    GuestOperation {
        operation_id: id.to_owned(),
        request_digest: [0x44; 32],
    }
}

#[tokio::test]
async fn daemon_guest_proxy_reuses_the_authenticated_session() {
    let (connector, connected) = daemon_client().await;
    let daemon = DaemonClient::new(connected).unwrap();
    let guest = daemon
        .guest_proxy(&WorkloadName::parse("workload").unwrap())
        .unwrap();
    assert_eq!(guest.session_generation(), daemon.session_generation());
    assert_eq!(connector.0.attempts.load(Ordering::SeqCst), 1);

    connector.respond_typed(
        "InspectExec",
        &guest::GuestInspectExecResponse {
            outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
            operation_id: "proxy-inspect".to_owned(),
            session_generation: GENERATION,
            request_id: vec![7; 16],
            result: Some(guest::guest_inspect_exec_response::Result::ListPage(
                guest::GuestExecListPage::new(),
            )),
            ..Default::default()
        },
    );
    let response = guest
        .inspect_exec(
            GuestInspectCall {
                operation: guest_operation("proxy-inspect"),
                query: guest_list_query(),
            },
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        response.result,
        Some(guest::guest_inspect_exec_response::Result::ListPage(_))
    ));
    assert_eq!(connector.0.attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn absent_daemon_guest_proxy_fails_closed_without_reconnecting() {
    let (connector, connected) = daemon_client().await;
    let daemon = DaemonClient::new(connected).unwrap();
    let guest = daemon
        .guest_proxy(&WorkloadName::parse("workload").unwrap())
        .unwrap();
    let result = guest
        .inspect_exec(
            GuestInspectCall {
                operation: guest_operation("missing-proxy"),
                query: guest_list_query(),
            },
            options(false),
            &CancellationToken::default(),
        )
        .await;
    assert!(matches!(
        result,
        Err(ClientError::ServiceContract(_)) | Err(ClientError::TransportFailed)
    ));
    assert_eq!(connector.0.attempts.load(Ordering::SeqCst), 1);
}

fn workload_projection() -> daemon::WorkloadProjection {
    daemon::WorkloadProjection {
        identity: MessageField::some(daemon::WorkloadIdentityProjection {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
            realm_path: "local-root".to_owned(),
            workload_name: "workload".to_owned(),
            canonical_target: "workload.local-root.d2b".to_owned(),
            ..Default::default()
        }),
        name: "workload".to_owned(),
        environment: "work".to_owned(),
        graphics: true,
        tpm: true,
        usbip: false,
        static_ip: vec![10, 42, 0, 2],
        lifecycle: MessageField::some(daemon::WorkloadLifecycleProjection {
            state: EnumOrUnknown::new(
                daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RUNNING,
            ),
            generation: 9,
            ..Default::default()
        }),
        runtime: MessageField::some(daemon::RuntimeProjection {
            kind: EnumOrUnknown::new(daemon::RuntimeKind::RUNTIME_KIND_NIXOS),
            detail: "nixos".to_owned(),
            supported_capabilities: vec![EnumOrUnknown::new(
                daemon::RuntimeCapability::RUNTIME_CAPABILITY_EXEC,
            )],
            ..Default::default()
        }),
        services: vec![daemon::ServiceProjection {
            kind: EnumOrUnknown::new(daemon::ServiceKind::SERVICE_KIND_DAEMON),
            role_id: "daemon".to_owned(),
            state: EnumOrUnknown::new(daemon::ServiceState::SERVICE_STATE_ACTIVE),
            ..Default::default()
        }],
        autostart: MessageField::some(daemon::AutostartProjection {
            mode: EnumOrUnknown::new(daemon::AutostartMode::AUTOSTART_MODE_ENABLED),
            reason: "declared".to_owned(),
            ..Default::default()
        }),
        deployment: MessageField::some(daemon::DeploymentProjection {
            declared_guest_closure: "/nix/store/declared".to_owned(),
            current_generation: "/nix/store/current".to_owned(),
            booted_generation: "/nix/store/booted".to_owned(),
            ..Default::default()
        }),
        declared_roles: vec!["daemon".to_owned()],
        readiness: vec![daemon::ReadinessProjection {
            role_id: "daemon".to_owned(),
            predicate_id: "ready".to_owned(),
            state: EnumOrUnknown::new(daemon::ServiceState::SERVICE_STATE_ACTIVE),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn page(returned: u32, truncated: bool) -> daemon::PageInfo {
    daemon::PageInfo {
        truncated,
        next_page_cursor: if truncated {
            "next-page".to_owned()
        } else {
            String::new()
        },
        returned_items: returned,
        total_items_known: true,
        total_items: returned + u32::from(truncated),
        ..Default::default()
    }
}

async fn wait_for_count(value: &AtomicUsize, expected: usize, notify: &Notify) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while value.load(Ordering::SeqCst) < expected {
            notify.notified().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_routes_select_exact_transport_without_fallback() {
    let (realm, _, _) = ids();
    let connector = FakeConnector::new();
    let client = Client::with_clock(routes(), connector.clone(), FixedClock);
    client
        .connect(
            TargetInput::LocalRoot(realm.clone()),
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await
        .unwrap();
    assert_eq!(connector.0.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        client
            .connect(
                TargetInput::LocalRoot(realm.clone()),
                ServiceKind::Daemon,
                TransportSelection::exact(TransportKind::Provider),
            )
            .await
            .unwrap_err(),
        ClientError::TransportPolicyMismatch
    );
    assert_eq!(connector.0.attempts.load(Ordering::SeqCst), 1);
    connector.0.fail_connect.store(true, Ordering::Release);
    assert_eq!(
        client
            .connect(
                TargetInput::LocalRoot(realm),
                ServiceKind::Daemon,
                TransportSelection::exact(TransportKind::LocalUnix),
            )
            .await
            .unwrap_err(),
        ClientError::ConnectFailed
    );
    assert_eq!(connector.0.attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn daemon_typed_list_preserves_projection_and_truncation() {
    let (connector, connected) = daemon_client().await;
    let response = daemon::ListWorkloadsResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
        workloads: vec![workload_projection()],
        page: MessageField::some(page(1, true)),
        ..Default::default()
    };
    connector.respond_typed("ListWorkloads", &response);
    let client = DaemonClient::new(connected).unwrap();
    let received = client
        .list_workloads(None, 1, None, options(false), &CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(received.workloads[0].name, "workload");
    assert_eq!(
        received.workloads[0]
            .identity
            .as_ref()
            .unwrap()
            .canonical_target,
        "workload.local-root.d2b"
    );
    assert!(received.page.as_ref().unwrap().truncated);
    assert_eq!(
        received.page.as_ref().unwrap().next_page_cursor,
        "next-page"
    );
    assert_eq!(connector.0.seen.lock().unwrap()[0].generation, GENERATION);
}

#[tokio::test]
async fn daemon_typed_errors_and_generation_mismatch_are_actionable() {
    let (connector, connected) = daemon_client().await;
    let response = daemon::InspectResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_FAILED),
        error: MessageField::some(common::ErrorEnvelope {
            kind: EnumOrUnknown::new(common::ErrorKind::ERROR_KIND_GENERATION_MISMATCH),
            retry: EnumOrUnknown::new(common::RetryClass::RETRY_CLASS_AFTER_OBSERVATION),
            correlation_id: "corr-1".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };
    connector.respond_typed("Inspect", &response);
    let client = DaemonClient::new(connected).unwrap();
    assert_eq!(
        client
            .inspect(
                Some("demo"),
                1,
                None,
                options(false),
                &CancellationToken::default(),
            )
            .await
            .unwrap_err(),
        ClientError::Remote {
            kind: RemoteErrorKind::GenerationMismatch,
            retry: RetryClass::Observe,
        }
    );
}

#[tokio::test]
async fn guest_exec_management_preserves_typed_state_and_cancel_correlation() {
    let (connector, client) = guest_client().await;
    let inspect_response = guest::GuestInspectExecResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
        operation_id: "inspect-1".to_owned(),
        session_generation: GENERATION,
        request_id: vec![7; 16],
        result: Some(guest::guest_inspect_exec_response::Result::ListPage(
            guest::GuestExecListPage {
                entries: vec![guest::GuestExecListEntry {
                    resource_handle: "exec-1".to_owned(),
                    state: EnumOrUnknown::new(guest::GuestExecState::GUEST_EXEC_STATE_RUNNING),
                    created_at_unix_ms: NOW,
                    argv_digest: vec![0x33; 32],
                    stdout_bytes: 4,
                    stderr_bytes: 2,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    connector.respond_typed("InspectExec", &inspect_response);
    let query = guest::GuestInspectExecQuery {
        query: Some(guest::guest_inspect_exec_query::Query::ListPage(
            guest::GuestExecListPageQuery {
                page_size: 32,
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let inspected = client
        .inspect_exec(
            GuestInspectCall {
                operation: guest_operation("inspect-1"),
                query,
            },
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let Some(guest::guest_inspect_exec_response::Result::ListPage(page)) = inspected.result else {
        panic!("expected list page");
    };
    assert_eq!(page.entries[0].resource_handle, "exec-1");
    assert_eq!(
        page.entries[0].state.enum_value_or_default(),
        guest::GuestExecState::GUEST_EXEC_STATE_RUNNING
    );

    let cancel_response = guest::GuestCancelExecResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
        operation_id: "cancel-1".to_owned(),
        session_generation: GENERATION,
        request_id: vec![7; 16],
        resource_handle: "exec-1".to_owned(),
        cancellation: EnumOrUnknown::new(
            guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_SIGNALLED,
        ),
        ..Default::default()
    };
    connector.respond_typed("CancelExec", &cancel_response);
    let cancelled = client
        .cancel_exec(
            GuestCancelCall {
                operation: guest_operation("cancel-1"),
                resource_handle: "exec-1".to_owned(),
                control_sequence: 1,
                reason: guest::GuestExecCancelReason::GUEST_EXEC_CANCEL_REASON_USER_REQUESTED,
            },
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.resource_handle, "exec-1");
}

#[tokio::test]
async fn guest_retained_log_open_binds_range_resource_and_selection() {
    use terminal::terminal_stream_frame::Frame;

    let (connector, client) = guest_client().await;
    let response = terminal::TerminalOpenResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
        operation_id: "logs-1".to_owned(),
        stream_id: "stream-256".to_owned(),
        session_generation: GENERATION,
        request_id: vec![7; 16],
        resource_handle: "exec-1".to_owned(),
        retained_log: MessageField::some(terminal::TerminalRetainedLogRange {
            output: EnumOrUnknown::new(terminal::OutputStream::OUTPUT_STREAM_STDOUT),
            requested_offset: 4,
            start_offset: 4,
            end_offset: 7,
            max_bytes: 3,
            eof: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    connector.respond_typed("OpenExecRetainedLog", &response);
    let stream = client
        .open_exec_retained_log(
            GuestRetainedLogCall {
                operation: guest_operation("logs-1"),
                resource_handle: "exec-1".to_owned(),
                output: terminal::OutputStream::OUTPUT_STREAM_STDOUT,
                offset: 4,
                max_bytes: 3,
            },
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(stream.resource_handle(), "exec-1");
    tokio::time::timeout(Duration::from_secs(1), async {
        while connector.0.stream_sent.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let sent = connector.0.stream_sent.lock().unwrap();
    let selection = decode_strict::<terminal::TerminalStreamFrame>(&sent[0], false).unwrap();
    assert_eq!(selection.operation_id, "logs-1");
    assert_eq!(selection.resource_handle, "exec-1");
    assert!(matches!(
        selection.frame,
        Some(Frame::Select(terminal::TerminalSelection {
            selection: Some(terminal::terminal_selection::Selection::RetainedLog(_)),
            ..
        }))
    ));
}

#[tokio::test]
async fn terminal_uses_server_stream_and_validates_bidirectional_lifecycle() {
    use terminal::{
        terminal_outcome::Outcome as TerminalOutcome, terminal_selection::Selection,
        terminal_stream_frame::Frame,
    };

    let connector = FakeConnector::new();
    *connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
    connector.0.terminal_script.store(true, Ordering::Release);
    let (connector, connected) = daemon_client_with(connector).await;
    let client = DaemonClient::new(connected).unwrap();
    let terminal = client
        .open_terminal(
            DaemonMethod::Exec,
            "demo",
            "op-1",
            terminal::TerminalSelection {
                selection: Some(Selection::Exec(terminal::ExecSelection {
                    authority: EnumOrUnknown::new(
                        terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY,
                    ),
                    selection: Some(terminal::exec_selection::Selection::Arbitrary(
                        terminal::ArbitraryExecSelection {
                            argv: vec![b"printf".to_vec()],
                            ..Default::default()
                        },
                    )),
                    tty: true,
                    initial_size: MessageField::some(terminal::TerminalSize {
                        rows: 24,
                        columns: 80,
                        ..Default::default()
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            },
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(terminal.session_generation(), GENERATION);
    let started = terminal.receive().await.unwrap();
    assert!(matches!(started.frame, Some(Frame::Started(_))));
    terminal
        .send(Frame::Stdin(terminal::TerminalStdin {
            offset: 0,
            data: b"input".to_vec(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let status = terminal.receive().await.unwrap();
    assert!(matches!(status.frame, Some(Frame::Status(_))));
    terminal
        .send(Frame::Cancel(terminal::TerminalCancel::new()))
        .await
        .unwrap();
    let outcome = terminal.receive().await.unwrap();
    assert!(matches!(
        outcome.frame,
        Some(Frame::Outcome(terminal::TerminalOutcome {
            outcome: Some(TerminalOutcome::Cancelled(_)),
            ..
        }))
    ));
    assert!(terminal.is_terminal());
    assert_eq!(
        terminal
            .send(Frame::Cancel(terminal::TerminalCancel::new()))
            .await
            .unwrap_err(),
        ClientError::StreamClosed
    );
    terminal.close_transport().await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(connector.0.closed.load(Ordering::Acquire), 1);

    let sent = connector.0.stream_sent.lock().unwrap();
    let selection =
        decode_strict::<terminal::TerminalStreamFrame>(&sent[0], false).expect("selection frame");
    assert_eq!(selection.sequence, 0);
    assert_eq!(selection.session_generation, GENERATION);
    assert_eq!(selection.request_id, vec![7; 16]);
    assert_eq!(selection.operation_id, "op-1");
    assert_eq!(selection.resource_handle, "resource-1");
    assert!(matches!(selection.frame, Some(Frame::Select(_))));
}

#[tokio::test]
async fn shell_management_uses_typed_selection_result_and_terminal_outcome() {
    use terminal::terminal_stream_frame::Frame;

    let connector = FakeConnector::new();
    connector.0.terminal_script.store(true, Ordering::Release);
    let (_connector, connected) = daemon_client_with(connector).await;
    let client = DaemonClient::new(connected).unwrap();
    let stream = client
        .open_terminal(
            DaemonMethod::Shell,
            "demo",
            "shell-list-1",
            terminal::TerminalSelection {
                selection: Some(terminal::terminal_selection::Selection::Shell(
                    terminal::ShellSelection {
                        action: EnumOrUnknown::new(terminal::ShellAction::SHELL_ACTION_LIST),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let result = stream.receive().await.unwrap();
    let Some(Frame::ShellResult(result)) = result.frame else {
        panic!("expected shell result");
    };
    assert_eq!(result.sessions[0].shell_handle, "shell-1");
    assert!(result.sessions[0].is_default);
    let outcome = stream.receive().await.unwrap();
    assert!(matches!(outcome.frame, Some(Frame::Outcome(_))));
    assert!(stream.is_terminal());
}

#[tokio::test]
async fn terminal_rejects_response_generation_and_non_server_stream_ids() {
    let connector = FakeConnector::new();
    connector
        .0
        .terminal_open_generation_delta
        .store(1, Ordering::Release);
    let (_connector, connected) = daemon_client_with(connector).await;
    let client = DaemonClient::new(connected).unwrap();
    let selection = terminal::TerminalSelection {
        selection: Some(terminal::terminal_selection::Selection::Console(
            terminal::ConsoleSelection {
                initial_size: MessageField::some(terminal::TerminalSize {
                    rows: 24,
                    columns: 80,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    assert!(matches!(
        client
            .open_terminal(
                DaemonMethod::OpenConsole,
                "demo",
                "op-2",
                selection,
                options(true),
                &CancellationToken::default(),
            )
            .await,
        Err(ClientError::ServiceContract(
            ServiceContractError::InconsistentResponse
        ))
    ));

    let connector = FakeConnector::new();
    *connector.0.stream_id.lock().unwrap() = Some("client-stream".to_owned());
    let (_connector, connected) = daemon_client_with(connector).await;
    let client = DaemonClient::new(connected).unwrap();
    let selection = terminal::TerminalSelection {
        selection: Some(terminal::terminal_selection::Selection::Console(
            terminal::ConsoleSelection {
                initial_size: MessageField::some(terminal::TerminalSize {
                    rows: 24,
                    columns: 80,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    assert!(matches!(
        client
            .open_terminal(
                DaemonMethod::OpenConsole,
                "demo",
                "op-3",
                selection,
                options(true),
                &CancellationToken::default(),
            )
            .await,
        Err(ClientError::ServiceContract(
            ServiceContractError::InvalidId
        ))
    ));
}

#[test]
fn exact_selection_disambiguates_multiple_owner_transports() {
    let realm = ids().0;
    let owner = ServiceOwner::LocalRoot(realm.clone());
    let table = RouteTable::new(vec![
        RouteRecord {
            owner: owner.clone(),
            transport: TransportKind::LocalUnix,
        },
        RouteRecord {
            owner,
            transport: TransportKind::NativeVsock,
        },
    ]);

    for transport in [TransportKind::LocalUnix, TransportKind::NativeVsock] {
        let resolved = table
            .resolve(
                &TargetInput::LocalRoot(realm.clone()),
                ServiceKind::Daemon,
                TransportSelection::exact(transport),
            )
            .unwrap();
        assert_eq!(resolved.transport(), transport);
    }
    assert_eq!(
        table
            .resolve(
                &TargetInput::LocalRoot(realm),
                ServiceKind::Daemon,
                TransportSelection::exact(TransportKind::Provider),
            )
            .unwrap_err(),
        ClientError::TransportPolicyMismatch
    );
}

#[test]
fn unspecified_transport_never_selects_a_route() {
    let realm = ids().0;
    let table = RouteTable::new(vec![
        RouteRecord {
            owner: ServiceOwner::LocalRoot(realm.clone()),
            transport: TransportKind::LocalUnix,
        },
        RouteRecord {
            owner: ServiceOwner::LocalRoot(realm.clone()),
            transport: TransportKind::NativeVsock,
        },
    ]);

    assert_eq!(
        table
            .resolve(
                &TargetInput::LocalRoot(realm),
                ServiceKind::Daemon,
                TransportSelection::unspecified(),
            )
            .unwrap_err(),
        ClientError::TransportPolicyMismatch
    );
}

#[test]
fn final_daemon_and_guest_fingerprints_are_exact() {
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    assert_eq!(
        hex(&d2b_contracts::v2_services::service_schema_fingerprint(
            ServiceKind::Daemon.spec()
        )),
        "4b2834c89162e5a2c17ea879052c066fd546cdc440d1473955a99e2d9521a54a"
    );
    assert_eq!(
        hex(&d2b_contracts::v2_services::service_schema_fingerprint(
            ServiceKind::Guest.spec()
        )),
        "e6d2fd47db903deff84b5b9cb58a0aed17e2f6ef43010182925890878a15dd3d"
    );
}

#[test]
fn terminal_quit_is_distinct_and_unknown_signals_remain_rejected() {
    let frame = |signal| terminal::TerminalStreamFrame {
        session_generation: GENERATION,
        request_id: vec![7; 16],
        operation_id: "signal-1".to_owned(),
        resource_handle: "exec-1".to_owned(),
        frame: Some(terminal::terminal_stream_frame::Frame::Signal(
            terminal::TerminalSignal {
                operation_sequence: 1,
                signal,
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    encode_strict(
        &frame(EnumOrUnknown::new(
            terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_QUIT,
        )),
        false,
    )
    .expect("QUIT is a valid distinct terminal signal");
    assert!(matches!(
        encode_strict(&frame(EnumOrUnknown::from_i32(999)), false,),
        Err(ServiceContractError::InvalidEnum)
    ));
}

#[test]
fn duplicate_records_fail_only_the_selected_transport() {
    let realm = ids().0;
    let owner = ServiceOwner::LocalRoot(realm.clone());
    let table = RouteTable::new(vec![
        RouteRecord {
            owner: owner.clone(),
            transport: TransportKind::LocalUnix,
        },
        RouteRecord {
            owner: owner.clone(),
            transport: TransportKind::LocalUnix,
        },
        RouteRecord {
            owner,
            transport: TransportKind::NativeVsock,
        },
    ]);

    assert_eq!(
        table
            .resolve(
                &TargetInput::LocalRoot(realm.clone()),
                ServiceKind::Daemon,
                TransportSelection::exact(TransportKind::LocalUnix),
            )
            .unwrap_err(),
        ClientError::InvalidTarget
    );
    assert_eq!(
        table
            .resolve(
                &TargetInput::LocalRoot(realm),
                ServiceKind::Daemon,
                TransportSelection::exact(TransportKind::NativeVsock),
            )
            .unwrap()
            .transport(),
        TransportKind::NativeVsock
    );
}

#[tokio::test]
async fn constructs_every_generated_inventory_client() {
    assert_eq!(ServiceKind::ALL.len(), SERVICE_INVENTORY.len());
    let connector = FakeConnector::new();
    let client = Client::with_clock(routes(), connector, FixedClock);
    let realm = ids().0;
    for service in ServiceKind::ALL {
        let connected = client
            .connect(
                TargetInput::Service {
                    owner: ServiceOwner::LocalRoot(realm.clone()),
                    service: *service,
                },
                *service,
                TransportSelection::exact(TransportKind::LocalUnix),
            )
            .await
            .unwrap();
        assert_eq!(connected.service().generated().kind(), *service);
    }
}

#[tokio::test]
async fn metadata_retries_and_cancellation_use_canonical_driver() {
    let (connector, client) = daemon_client().await;
    let read = client.service().method(3).unwrap();
    connector.fail_once(SessionFailure::Retryable);
    client
        .invoke(
            read,
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    {
        let seen = connector.0.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].generation, GENERATION);
        assert_eq!(seen[0].request_id, seen[1].request_id);
        assert!(seen[0].idempotency_key.is_empty());
        assert!(seen[0].timeout_nanos > 0);
        assert!(seen[0].attachment_count == 0);
    }

    connector.0.block.store(true, Ordering::Release);
    let cancellation = CancellationToken::default();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        trigger.cancel();
    });
    assert_eq!(
        client
            .invoke(
                read,
                common::ServiceRequest::new(),
                options(false),
                &cancellation,
            )
            .await
            .unwrap_err(),
        ClientError::Cancelled
    );
    assert_eq!(connector.0.cancellations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mutating_retries_require_stable_idempotency() {
    let (connector, client) = daemon_client().await;
    let method = client.service().method(4).unwrap();
    connector.fail_once(SessionFailure::Retryable);
    client
        .invoke(
            method,
            common::ServiceRequest::new(),
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    {
        let seen = connector.0.seen.lock().unwrap();
        assert_eq!(seen[0].request_id, seen[1].request_id);
        assert_eq!(seen[0].idempotency_key, seen[1].idempotency_key);
    }

    let missing = CallOptions {
        metadata: MetadataInput::new([7; 16], NOW, NOW + 30_000).unwrap(),
        retry: RetryPolicy::new(3).unwrap(),
    };
    assert_eq!(
        client
            .invoke(
                method,
                common::ServiceRequest::new(),
                missing,
                &CancellationToken::default(),
            )
            .await
            .unwrap_err(),
        ClientError::IdempotencyRequired
    );
}

struct CountingAttachment(Arc<AtomicUsize>);

impl AttachmentPayload for CountingAttachment {
    fn close(self: Box<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    fn validate_descriptor(
        &self,
        _descriptor: &AttachmentDescriptor,
    ) -> Result<(), AttachmentValidationError> {
        Ok(())
    }
}

fn descriptor(
    purpose: AttachmentPurpose,
    index: u16,
    request_id: [u8; 16],
    method_id: u32,
) -> AttachmentDescriptor {
    AttachmentDescriptor {
        index,
        kind: AttachmentKind::FileDescriptor,
        object_type: KernelObjectType::Memfd,
        access: AttachmentAccess::ReadOnly,
        purpose,
        service: ServicePackage::DaemonV2,
        method_id,
        request_id: RequestId::new(request_id.to_vec()).unwrap(),
        operation_id: None,
        packet_sequence: 1,
        reconnect_generation: GENERATION,
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

#[tokio::test]
async fn owned_attachments_are_authenticated_and_mismatches_close_once() {
    let (connector, client) = daemon_client().await;
    let method = client.service().method(3).unwrap();
    let method_id = method.spec().method_id(
        method.service().spec().package,
        method.service().spec().service,
    );
    let closes = Arc::new(AtomicUsize::new(0));
    connector
        .0
        .response_attachments
        .lock()
        .unwrap()
        .push_back(vec![OwnedAttachment::new(
            descriptor(
                AttachmentPurpose::ResponseOutput,
                0,
                [7; 16],
                method_id.wrapping_add(1),
            ),
            Box::new(CountingAttachment(Arc::clone(&closes))),
        )]);
    assert_eq!(
        client
            .invoke(
                method,
                common::ServiceRequest::new(),
                options(false),
                &CancellationToken::default(),
            )
            .await
            .unwrap_err(),
        ClientError::AttachmentMismatch
    );
    assert_eq!(closes.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn outbound_attachments_are_owned_by_the_canonical_engine() {
    let (connector, client) = daemon_client().await;
    let method = client.service().method(3).unwrap();
    let method_id = method.spec().method_id(
        method.service().spec().package,
        method.service().spec().service,
    );
    let closes = Arc::new(AtomicUsize::new(0));
    let attachment = OwnedAttachment::new(
        descriptor(AttachmentPurpose::RequestInput, 0, [7; 16], method_id),
        Box::new(CountingAttachment(Arc::clone(&closes))),
    );
    client
        .invoke_with_attachments(
            method,
            common::ServiceRequest::new(),
            vec![attachment],
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(connector.0.seen.lock().unwrap()[0].attachment_count, 1);
    assert_eq!(closes.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn validates_closed_remote_errors() {
    let (connector, client) = daemon_client().await;
    let mut response = common::ServiceResponse::new();
    response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_DENIED);
    let mut error = common::ErrorEnvelope::new();
    error.kind = EnumOrUnknown::new(common::ErrorKind::ERROR_KIND_UNAUTHORIZED);
    error.retry = EnumOrUnknown::new(common::RetryClass::RETRY_CLASS_NEVER);
    response.error = MessageField::some(error);
    *connector.0.response_override.lock().unwrap() = Some(response);
    assert_eq!(
        client
            .invoke(
                client.service().method(3).unwrap(),
                common::ServiceRequest::new(),
                options(false),
                &CancellationToken::default(),
            )
            .await
            .unwrap_err(),
        ClientError::Remote {
            kind: RemoteErrorKind::Forbidden,
            retry: RetryClass::Never,
        }
    );
}

#[tokio::test]
async fn invalid_terminal_selection_is_rejected_before_open_rpc() {
    let (connector, connected) = daemon_client().await;
    let client = DaemonClient::new(connected).unwrap();
    let selection = terminal::TerminalSelection {
        selection: Some(terminal::terminal_selection::Selection::Exec(
            terminal::ExecSelection {
                authority: EnumOrUnknown::new(
                    terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY,
                ),
                selection: Some(terminal::exec_selection::Selection::Arbitrary(
                    terminal::ArbitraryExecSelection::new(),
                )),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    assert!(matches!(
        client
            .open_terminal(
                DaemonMethod::Exec,
                "demo",
                "invalid-selection",
                selection,
                options(true),
                &CancellationToken::default(),
            )
            .await,
        Err(ClientError::ServiceContract(_))
    ));
    assert_eq!(connector.0.calls.load(Ordering::Acquire), 0);
    assert!(connector.0.stream_sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn named_stream_fragments_over_queue_credit_and_has_terminal_actions() {
    let (connector, client) = daemon_client().await;
    *connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
    connector
        .0
        .stream_received
        .lock()
        .unwrap()
        .push_back(vec![2; 256 * 1024 + 32]);
    let response = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let stream = client.named_stream(&response).await.unwrap();
    let logical = vec![9; 256 * 1024 + 37];
    stream.send(&logical).await.unwrap();
    assert_eq!(
        connector.0.stream_sent.lock().unwrap().as_slice(),
        &[logical]
    );
    assert_eq!(stream.receive().await.unwrap().len(), 256 * 1024 + 32);
    stream.detach().await.unwrap();
    assert_eq!(
        stream.send(b"late").await.unwrap_err(),
        ClientError::StreamDetached
    );

    let (close_connector, close_client) = daemon_client().await;
    *close_connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
    let close_response = close_client
        .invoke(
            close_client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let close = close_client.named_stream(&close_response).await.unwrap();
    close.close().await.unwrap();

    let (cancel_connector, cancel_client) = daemon_client().await;
    *cancel_connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
    let cancel_response = cancel_client
        .invoke(
            cancel_client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let cancel = cancel_client.named_stream(&cancel_response).await.unwrap();
    cancel.cancel().await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(close_connector.0.closed.load(Ordering::SeqCst), 1);
    assert_eq!(
        cancel_connector.0.stream_cancelled.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn named_stream_grants_only_consumed_data_and_releases_blocked_sender() {
    let connector = FakeConnector::new();
    connector
        .0
        .stream_received
        .lock()
        .unwrap()
        .extend([vec![1; 256 * 1024], vec![2]]);
    *connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
    let (connector, client) = daemon_client_with(connector).await;
    let response = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let stream = client.named_stream(&response).await.unwrap();

    stream.send(b"prime").await.unwrap();
    wait_for_count(
        &connector.0.stream_send_started,
        2,
        &connector.0.stream_progress,
    )
    .await;
    assert_eq!(connector.0.stream_send_completed.load(Ordering::SeqCst), 1);
    assert!(connector.0.granted_stream_credit.lock().unwrap().is_empty());

    assert_eq!(stream.receive().await.unwrap().len(), 256 * 1024);
    assert_eq!(
        connector.0.granted_stream_credit.lock().unwrap().as_slice(),
        &[256 * 1024]
    );
    wait_for_count(
        &connector.0.stream_send_completed,
        2,
        &connector.0.stream_progress,
    )
    .await;
    assert_eq!(stream.receive().await.unwrap(), vec![2]);
    assert_eq!(
        connector.0.granted_stream_credit.lock().unwrap().as_slice(),
        &[256 * 1024, 1]
    );
}

#[tokio::test]
async fn concurrent_named_streams_route_events_without_cross_consumption() {
    let connector = FakeConnector::new();
    *connector.0.stream_event.lock().unwrap() = Some(RemoteStreamEvent::InterleavedStreams);
    *connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
    let (_connector, client) = daemon_client_with(connector).await;
    let response = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let primary = client.named_stream(&response).await.unwrap();
    let mut other_message = common::ServiceResponse::new();
    other_message.stream_id = "stream-257".to_owned();
    let other = client
        .named_stream(&Response {
            message: other_message,
            attachments: Vec::new(),
        })
        .await
        .unwrap();

    primary.send(b"prime").await.unwrap();
    assert_eq!(other.receive().await.unwrap(), b"other-stream");
    assert_eq!(primary.receive().await.unwrap(), b"primary-stream");
}

#[tokio::test]
async fn named_stream_terminal_and_error_events_do_not_grant_credit() {
    for (event, expected) in [
        (RemoteStreamEvent::Close, ClientError::StreamClosed),
        (RemoteStreamEvent::Reset, ClientError::StreamClosed),
        (RemoteStreamEvent::WrongStream, ClientError::TransportFailed),
    ] {
        let connector = FakeConnector::new();
        *connector.0.stream_event.lock().unwrap() = Some(event);
        *connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
        let (connector, client) = daemon_client_with(connector).await;
        let response = client
            .invoke(
                client.service().method(3).unwrap(),
                common::ServiceRequest::new(),
                options(false),
                &CancellationToken::default(),
            )
            .await
            .unwrap();
        let stream = client.named_stream(&response).await.unwrap();
        stream.send(b"prime").await.unwrap();
        assert_eq!(stream.receive().await.unwrap_err(), expected);
        assert!(connector.0.granted_stream_credit.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn stream_logical_bound_is_one_mib() {
    let (connector, client) = daemon_client().await;
    *connector.0.stream_id.lock().unwrap() = Some("stream-256".to_owned());
    let response = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let stream = client.named_stream(&response).await.unwrap();
    stream.send(&vec![0; 1024 * 1024]).await.unwrap();
    assert_eq!(connector.0.stream_sent.lock().unwrap().len(), 1);
    assert_eq!(
        stream.send(&vec![0; 1024 * 1024 + 1]).await.unwrap_err(),
        ClientError::StreamLimitExceeded
    );
}

#[test]
fn debug_and_errors_are_redacted() {
    let metadata = MetadataInput::new([7; 16], NOW, NOW + 30_000)
        .unwrap()
        .with_correlation("sensitive-correlation")
        .unwrap()
        .with_idempotency(b"sensitive-idempotency".to_vec())
        .unwrap();
    assert!(!format!("{metadata:?}").contains("sensitive"));
    let remote = ClientError::Remote {
        kind: RemoteErrorKind::Internal,
        retry: RetryClass::Never,
    };
    assert_eq!(remote.to_string(), "client-remote-internal-retry-never");
    assert_eq!(
        ClientError::SessionEstablishment(SessionErrorCode::IdentityEvidenceMismatch).to_string(),
        "client-session-establishment-identity-evidence-mismatch"
    );
    assert_eq!(
        ClientError::ServiceContract(ServiceContractError::InvalidDeadline).to_string(),
        "client-v2-service-invalid-deadline"
    );
}
