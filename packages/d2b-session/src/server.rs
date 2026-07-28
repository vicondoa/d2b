use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts::v3::component_session::RequestId;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use ttrpc::{
    r#async::{MethodHandler, Service, StreamHandler, StreamInner, TtrpcContext},
    proto::{MESSAGE_HEADER_LENGTH, MessageHeader, Request, Response},
};

use crate::{Cancellation, ComponentSessionDriver, Result as SessionResult};

tokio::task_local! {
    static HANDLER_CANCELLATION: Cancellation;
}

/// Return the cancellation token for the generated handler running in this task.
pub fn current_handler_cancellation() -> Option<Cancellation> {
    HANDLER_CANCELLATION.try_with(Clone::clone).ok()
}

struct ActiveInboundCall {
    request_id: RequestId,
    cancellation: Cancellation,
}

type ActiveInboundCalls = Arc<Mutex<BTreeMap<u32, ActiveInboundCall>>>;

struct CancellableMethodHandler {
    inner: Box<dyn MethodHandler + Send + Sync>,
    active: ActiveInboundCalls,
}

#[async_trait]
impl MethodHandler for CancellableMethodHandler {
    async fn handler(&self, context: TtrpcContext, request: Request) -> ttrpc::Result<Response> {
        let cancellation = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&context.mh.stream_id)
            .map(|active| active.cancellation.clone())
            .ok_or_else(|| ttrpc::Error::Others("component-session-call-inactive".to_owned()))?;
        HANDLER_CANCELLATION
            .scope(cancellation, self.inner.handler(context, request))
            .await
    }
}

struct CancellableStreamHandler {
    inner: Arc<dyn StreamHandler + Send + Sync>,
    active: ActiveInboundCalls,
}

#[async_trait]
impl StreamHandler for CancellableStreamHandler {
    async fn handler(
        &self,
        context: TtrpcContext,
        stream: StreamInner,
    ) -> ttrpc::Result<Option<Response>> {
        let cancellation = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&context.mh.stream_id)
            .map(|active| active.cancellation.clone())
            .ok_or_else(|| ttrpc::Error::Others("component-session-call-inactive".to_owned()))?;
        HANDLER_CANCELLATION
            .scope(cancellation, self.inner.handler(context, stream))
            .await
    }
}

fn with_cancellation_context(
    services: HashMap<String, Service>,
    active: &ActiveInboundCalls,
) -> HashMap<String, Service> {
    services
        .into_iter()
        .map(|(name, service)| {
            let methods = service
                .methods
                .into_iter()
                .map(|(name, handler)| {
                    (
                        name,
                        Box::new(CancellableMethodHandler {
                            inner: handler,
                            active: Arc::clone(active),
                        }) as Box<dyn MethodHandler + Send + Sync>,
                    )
                })
                .collect();
            let streams = service
                .streams
                .into_iter()
                .map(|(name, handler)| {
                    (
                        name,
                        Arc::new(CancellableStreamHandler {
                            inner: handler,
                            active: Arc::clone(active),
                        }) as Arc<dyn StreamHandler + Send + Sync>,
                    )
                })
                .collect();
            (name, Service { methods, streams })
        })
        .collect()
}

#[async_trait]
trait TtrpcServerDriver: Send + Sync {
    fn generation(&self) -> u64;
    async fn receive_ttrpc(&self) -> SessionResult<Vec<u8>>;
    async fn send_ttrpc(&self, frame: Vec<u8>) -> SessionResult<()>;
    async fn send_ttrpc_cancellable(
        &self,
        frame: Vec<u8>,
        cancellation: Cancellation,
    ) -> SessionResult<()> {
        if cancellation.is_cancelled() {
            Err(crate::SessionError::new(
                d2b_contracts::v3::component_session::SessionErrorCode::Cancelled,
            ))
        } else {
            self.send_ttrpc(frame).await
        }
    }
    async fn register_inbound_call(&self, request_id: RequestId) -> SessionResult<Cancellation>;
    async fn mark_inbound_dispatched(&self, request_id: RequestId) -> SessionResult<()>;
    async fn complete_inbound_call(&self, request_id: RequestId) -> SessionResult<bool>;
    async fn remove_inbound_call(&self, request_id: RequestId) -> SessionResult<bool>;
}

struct ComponentDriverAdapter(Arc<dyn ComponentSessionDriver>);

#[async_trait]
impl TtrpcServerDriver for ComponentDriverAdapter {
    fn generation(&self) -> u64 {
        self.0.generation()
    }

    async fn receive_ttrpc(&self) -> SessionResult<Vec<u8>> {
        self.0.receive_ttrpc().await
    }

    async fn send_ttrpc(&self, frame: Vec<u8>) -> SessionResult<()> {
        self.0.send_ttrpc(frame).await
    }

    async fn send_ttrpc_cancellable(
        &self,
        frame: Vec<u8>,
        cancellation: Cancellation,
    ) -> SessionResult<()> {
        self.0.send_ttrpc_cancellable(frame, cancellation).await
    }

    async fn register_inbound_call(&self, request_id: RequestId) -> SessionResult<Cancellation> {
        self.0.register_inbound_call(request_id).await
    }

    async fn mark_inbound_dispatched(&self, request_id: RequestId) -> SessionResult<()> {
        self.0.mark_inbound_dispatched(request_id).await
    }

    async fn complete_inbound_call(&self, request_id: RequestId) -> SessionResult<bool> {
        self.0.complete_inbound_call(request_id).await
    }

    async fn remove_inbound_call(&self, request_id: RequestId) -> SessionResult<bool> {
        self.0.remove_inbound_call(request_id).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionServerError {
    Service,
    Session,
    Transport,
    Frame,
}

impl std::fmt::Display for SessionServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Service => "component-session-service-failed",
            Self::Session => "component-session-driver-failed",
            Self::Transport => "component-session-service-transport-failed",
            Self::Frame => "component-session-ttrpc-frame-invalid",
        })
    }
}

impl std::error::Error for SessionServerError {}

pub async fn serve_ttrpc_services(
    driver: Arc<dyn ComponentSessionDriver>,
    services: HashMap<String, Service>,
) -> Result<(), SessionServerError> {
    serve_ttrpc_services_inner(Arc::new(ComponentDriverAdapter(driver)), services).await
}

async fn serve_ttrpc_services_inner(
    driver: Arc<dyn TtrpcServerDriver>,
    services: HashMap<String, Service>,
) -> Result<(), SessionServerError> {
    if services.is_empty() {
        return Err(SessionServerError::Service);
    }
    let capacity =
        d2b_contracts::v3::component_session::LimitProfile::local_default().logical_ttrpc_bytes;
    let capacity = usize::try_from(capacity).map_err(|_| SessionServerError::Transport)?;
    let (server_transport, bridge_transport) = tokio::io::duplex(capacity);
    let listener =
        ttrpc::r#async::transport::Listener::new(futures_util::stream::once(async move {
            Ok::<_, std::io::Error>(server_transport)
        }));
    let active = Arc::new(Mutex::new(BTreeMap::<u32, ActiveInboundCall>::new()));
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(listener)
        .register_service(with_cancellation_context(services, &active));
    server
        .start()
        .await
        .map_err(|_| SessionServerError::Service)?;

    let (mut bridge_reader, mut bridge_writer) = tokio::io::split(bridge_transport);
    let receive_driver = Arc::clone(&driver);
    let receive_active = Arc::clone(&active);
    let receive = async move {
        loop {
            let frame = receive_driver
                .receive_ttrpc()
                .await
                .map_err(|_| SessionServerError::Session)?;
            let header = validate_frame(&frame)?;
            let request_id = ttrpc_request_id(receive_driver.generation(), &frame)?;
            let cancellation = receive_driver
                .register_inbound_call(request_id.clone())
                .await
                .map_err(|_| SessionServerError::Session)?;
            receive_active
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    header.stream_id,
                    ActiveInboundCall {
                        request_id: request_id.clone(),
                        cancellation,
                    },
                );
            if receive_driver
                .mark_inbound_dispatched(request_id.clone())
                .await
                .is_err()
                || bridge_writer.write_all(&frame).await.is_err()
                || bridge_writer.flush().await.is_err()
            {
                receive_active
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&header.stream_id);
                let _ = receive_driver.remove_inbound_call(request_id).await;
                return Err(SessionServerError::Transport);
            }
        }
    };
    let send_active = Arc::clone(&active);
    let send_driver = Arc::clone(&driver);
    let send = async move {
        loop {
            let mut header_bytes = [0_u8; MESSAGE_HEADER_LENGTH];
            bridge_reader
                .read_exact(&mut header_bytes)
                .await
                .map_err(|_| SessionServerError::Transport)?;
            let header = MessageHeader::from(header_bytes);
            let body_len = usize::try_from(header.length).map_err(|_| SessionServerError::Frame)?;
            if body_len > d2b_contracts::v3::component_session::MAX_LOGICAL_MESSAGE_BYTES as usize {
                return Err(SessionServerError::Frame);
            }
            let mut frame = header_bytes.to_vec();
            frame.resize(
                MESSAGE_HEADER_LENGTH
                    .checked_add(body_len)
                    .ok_or(SessionServerError::Frame)?,
                0,
            );
            bridge_reader
                .read_exact(&mut frame[MESSAGE_HEADER_LENGTH..])
                .await
                .map_err(|_| SessionServerError::Transport)?;
            let active_call = send_active
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&header.stream_id);
            let Some(active_call) = active_call else {
                return Err(SessionServerError::Frame);
            };
            let send_result = if active_call.cancellation.is_cancelled() {
                Ok(())
            } else {
                send_driver
                    .send_ttrpc_cancellable(frame, active_call.cancellation.clone())
                    .await
            };
            let _ = send_driver
                .complete_inbound_call(active_call.request_id)
                .await;
            send_result.map_err(|_| SessionServerError::Session)?;
        }
    };
    let result = tokio::select! {
        result = receive => result,
        result = send => result,
    };
    let terminal = {
        let mut active = active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *active)
    };
    for (_, active) in terminal {
        let _ = driver.remove_inbound_call(active.request_id).await;
    }
    server.disconnect().await;
    result
}

fn validate_frame(frame: &[u8]) -> Result<MessageHeader, SessionServerError> {
    let header_bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .ok_or(SessionServerError::Frame)?
        .try_into()
        .map_err(|_| SessionServerError::Frame)?;
    let header = MessageHeader::from(header_bytes);
    let body_len = usize::try_from(header.length).map_err(|_| SessionServerError::Frame)?;
    if body_len > d2b_contracts::v3::component_session::MAX_LOGICAL_MESSAGE_BYTES as usize
        || frame.len() != MESSAGE_HEADER_LENGTH.saturating_add(body_len)
    {
        return Err(SessionServerError::Frame);
    }
    Ok(header)
}

/// Derive the ComponentSession request id from authenticated generation and
/// the ttrpc stream id.
pub fn ttrpc_request_id(generation: u64, frame: &[u8]) -> Result<RequestId, SessionServerError> {
    let header = validate_frame(frame)?;
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&generation.to_be_bytes());
    bytes.extend_from_slice(&header.stream_id.to_be_bytes());
    bytes.extend_from_slice(b"ttrp");
    RequestId::new(bytes).map_err(|_| SessionServerError::Frame)
}

/// Return the ttrpc stream identifier after exact frame validation.
pub fn ttrpc_stream_id(frame: &[u8]) -> Result<u32, SessionServerError> {
    Ok(validate_frame(frame)?.stream_id)
}

/// Replace only the ttrpc stream identifier in an otherwise unchanged frame.
pub fn rewrite_ttrpc_stream_id(frame: &mut [u8], stream_id: u32) -> Result<(), SessionServerError> {
    let mut header = validate_frame(frame)?;
    header.stream_id = stream_id;
    let encoded = Vec::from(header);
    frame[..MESSAGE_HEADER_LENGTH].copy_from_slice(&encoded);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };

    use protobuf::Message;
    use tokio::sync::Notify;
    use ttrpc::proto::Request;

    use super::*;
    use crate::RequestRegistry;

    struct BlockingHandler {
        started: Arc<Notify>,
        observed: Arc<Notify>,
    }

    #[async_trait]
    impl MethodHandler for BlockingHandler {
        async fn handler(
            &self,
            _context: TtrpcContext,
            _request: Request,
        ) -> ttrpc::Result<Response> {
            let cancellation =
                current_handler_cancellation().expect("handler cancellation context");
            self.started.notify_one();
            cancellation.cancelled().await;
            self.observed.notify_one();
            Ok(Response::default())
        }
    }

    struct BlockingDriver {
        frame: StdMutex<Option<Vec<u8>>>,
        registry: StdMutex<RequestRegistry>,
        cancellation: StdMutex<Option<Cancellation>>,
        sends: AtomicUsize,
        completed: Notify,
    }

    impl BlockingDriver {
        fn new(frame: Vec<u8>) -> Self {
            Self {
                frame: StdMutex::new(Some(frame)),
                registry: StdMutex::new(RequestRegistry::new(1).unwrap()),
                cancellation: StdMutex::new(None),
                sends: AtomicUsize::new(0),
                completed: Notify::new(),
            }
        }

        fn cancel_handler(&self) {
            self.cancellation
                .lock()
                .unwrap()
                .as_ref()
                .expect("inbound call registered")
                .cancel();
        }
    }

    #[async_trait]
    impl TtrpcServerDriver for BlockingDriver {
        fn generation(&self) -> u64 {
            1
        }

        async fn receive_ttrpc(&self) -> SessionResult<Vec<u8>> {
            if let Some(frame) = self.frame.lock().unwrap().take() {
                return Ok(frame);
            }
            std::future::pending().await
        }

        async fn send_ttrpc(&self, _frame: Vec<u8>) -> SessionResult<()> {
            self.sends.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        async fn register_inbound_call(
            &self,
            request_id: RequestId,
        ) -> SessionResult<Cancellation> {
            let cancellation = self.registry.lock().unwrap().register(request_id)?;
            *self.cancellation.lock().unwrap() = Some(cancellation.clone());
            Ok(cancellation)
        }

        async fn mark_inbound_dispatched(&self, request_id: RequestId) -> SessionResult<()> {
            self.registry.lock().unwrap().mark_dispatched(&request_id)
        }

        async fn complete_inbound_call(&self, request_id: RequestId) -> SessionResult<bool> {
            let completed = self.registry.lock().unwrap().complete(&request_id);
            self.completed.notify_one();
            Ok(completed)
        }

        async fn remove_inbound_call(&self, request_id: RequestId) -> SessionResult<bool> {
            Ok(self.registry.lock().unwrap().remove(&request_id))
        }
    }

    fn request_frame(stream_id: u32) -> Vec<u8> {
        let request = Request {
            service: "test.Service".to_owned(),
            method: "Block".to_owned(),
            ..Request::default()
        };
        let payload = request.write_to_bytes().unwrap();
        let length = u32::try_from(payload.len()).unwrap();
        let mut frame = Vec::from(MessageHeader::new_request(stream_id, length));
        frame.extend_from_slice(&payload);
        frame
    }

    #[test]
    fn frame_validation_is_exact_and_bounded() {
        let header = MessageHeader {
            length: 3,
            stream_id: 7,
            type_: 1,
            flags: 0,
        };
        let mut frame = Vec::from(header);
        frame.extend_from_slice(b"abc");
        assert_eq!(validate_frame(&frame).unwrap(), header);
        frame.push(0);
        assert_eq!(validate_frame(&frame), Err(SessionServerError::Frame));
    }

    #[test]
    fn request_correlation_binds_generation_and_stream() {
        let frame = Vec::from(MessageHeader::new_request(7, 0));
        let first = ttrpc_request_id(3, &frame).unwrap();
        assert_ne!(first, ttrpc_request_id(4, &frame).unwrap());
        assert_ne!(
            first,
            ttrpc_request_id(3, &Vec::from(MessageHeader::new_request(8, 0))).unwrap()
        );
    }

    #[test]
    fn stream_rewrite_preserves_payload_and_changes_correlation() {
        let mut frame = Vec::from(MessageHeader::new_request(7, 3));
        frame.extend_from_slice(b"abc");
        assert_eq!(ttrpc_stream_id(&frame).unwrap(), 7);
        rewrite_ttrpc_stream_id(&mut frame, 19).unwrap();
        assert_eq!(ttrpc_stream_id(&frame).unwrap(), 19);
        assert_eq!(&frame[MESSAGE_HEADER_LENGTH..], b"abc");
    }

    #[tokio::test]
    async fn service_cancellation_reaches_handler_and_suppresses_late_response() {
        let started = Arc::new(Notify::new());
        let observed = Arc::new(Notify::new());
        let driver = Arc::new(BlockingDriver::new(request_frame(23)));
        let service = Service {
            methods: HashMap::from([(
                "Block".to_owned(),
                Box::new(BlockingHandler {
                    started: Arc::clone(&started),
                    observed: Arc::clone(&observed),
                }) as Box<dyn MethodHandler + Send + Sync>,
            )]),
            streams: HashMap::new(),
        };
        let serving_driver: Arc<dyn TtrpcServerDriver> = driver.clone();
        let serving = tokio::spawn(serve_ttrpc_services_inner(
            serving_driver,
            HashMap::from([("test.Service".to_owned(), service)]),
        ));

        tokio::time::timeout(std::time::Duration::from_secs(1), started.notified())
            .await
            .expect("handler starts");
        driver.cancel_handler();
        tokio::time::timeout(std::time::Duration::from_secs(1), observed.notified())
            .await
            .expect("handler observes cancellation");
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            driver.completed.notified(),
        )
        .await
        .expect("cancelled response is completed locally");
        assert_eq!(driver.sends.load(Ordering::Acquire), 0);

        serving.abort();
        let error = serving.await.unwrap_err();
        assert!(error.is_cancelled());
    }
}
