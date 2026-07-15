use std::{collections::BTreeMap, fmt, sync::Arc, time::Instant};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        EndpointPolicy, RequestId, ServicePackage, SessionErrorCode, TransportClass,
    },
    v2_services::{common, decode_strict},
};
use d2b_session::{ComponentSessionDriver, HandshakeCredentials, SessionEngine};
use d2b_session_unix::UnixSeqpacketTransport;
use protobuf::{EnumOrUnknown, Message, MessageField};
use tokio::{
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
    transport: UnixSeqpacketTransport,
    policy: EndpointPolicy,
    credentials: HandshakeCredentials,
}

pub struct HostSocketConnector {
    transport: TransportKind,
    pending: Mutex<Option<PendingSession>>,
}

impl HostSocketConnector {
    pub fn new(
        transport: UnixSeqpacketTransport,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
    ) -> Result<Self, ClientError> {
        let selected = match policy.transport_binding.transport {
            TransportClass::UnixSeqpacket => TransportKind::LocalUnix,
            TransportClass::InheritedSocketpair => TransportKind::InheritedSocket,
            _ => return Err(ClientError::TransportPolicyMismatch),
        };
        Ok(Self {
            transport: selected,
            pending: Mutex::new(Some(PendingSession {
                transport,
                policy,
                credentials,
            })),
        })
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
        if pending.policy.service != service_package(service) {
            return Err(ClientError::InvalidService);
        }
        let engine = SessionEngine::establish_initiator(
            pending.transport,
            pending.policy,
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
    if request.method == "Cancel" {
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
    let bytes = decode_strict::<common::ServiceRequest>(&request.payload, false)
        .map_err(|_| ())?
        .metadata
        .as_ref()
        .ok_or(())?
        .request_id
        .clone();
    RequestId::new(bytes).map_err(|_| ())
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
    use protobuf::MessageField;
    use tokio::{io::AsyncWriteExt, sync::Notify};

    use super::*;

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
            1
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
            service: "d2b.daemon.v2.Daemon".to_owned(),
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
        metadata.session_generation = 1;
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
        request.session_generation = 1;
        request.validate_wire(false).unwrap();
        request.write_to_bytes().unwrap()
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
}
