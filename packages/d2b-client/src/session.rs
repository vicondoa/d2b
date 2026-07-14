use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering},
    },
};

use async_trait::async_trait;
use d2b_contracts::v2_component_session::{
    KernelObjectType, MAX_LOGICAL_MESSAGE_BYTES, MAX_NAMED_STREAM_QUEUE_BYTES,
};
use d2b_contracts::v2_services::common::ServiceRequest;
use d2b_contracts::v2_services::common::ServiceResponse;
use tokio::sync::Notify;
use ttrpc::r#async::transport::Socket;

use crate::{ClientError, MethodHandle, ResolvedTarget, ServiceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFailure {
    BeforeDispatch,
    Retryable,
    Ambiguous,
    Disconnected,
    Deadline,
    Cancelled,
    Protocol,
}

pub struct SessionCall {
    pub method: MethodHandle,
    pub request: ServiceRequest,
    pub relative_timeout_nanos: u64,
}

impl fmt::Debug for SessionCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCall")
            .field("service", &self.method.service())
            .field("method_index", &self.method.index())
            .field(
                "has_attachments",
                &!self.request.attachment_indexes.is_empty(),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct SessionAttachment {
    index: u32,
    object_type: KernelObjectType,
}

impl SessionAttachment {
    pub const fn new(index: u32, object_type: KernelObjectType) -> Self {
        Self { index, object_type }
    }

    pub const fn index(&self) -> u32 {
        self.index
    }

    pub const fn object_type(&self) -> KernelObjectType {
        self.object_type
    }
}

impl fmt::Debug for SessionAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionAttachment")
            .field("index", &self.index)
            .field("object_type", &self.object_type)
            .finish()
    }
}

pub struct SessionReply {
    pub response: ServiceResponse,
    pub attachments: Vec<SessionAttachment>,
}

impl fmt::Debug for SessionReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionReply")
            .field("attachment_count", &self.attachments.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StreamId(String);

impl StreamId {
    pub fn new(value: impl Into<String>) -> Result<Self, ClientError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value.is_ascii()
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        valid
            .then_some(Self(value))
            .ok_or(ClientError::ContractViolation)
    }
}

impl fmt::Debug for StreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamId([redacted])")
    }
}

#[async_trait]
pub trait ComponentSession: Send + Sync {
    fn generation(&self) -> u64;

    async fn invoke(&self, call: SessionCall) -> Result<SessionReply, SessionFailure>;

    async fn cancel(&self, generation: u64, request_id: [u8; 16]) -> Result<(), SessionFailure>;

    async fn stream_send(&self, stream: &StreamId, message: &[u8]) -> Result<(), SessionFailure>;

    async fn stream_receive(&self, stream: &StreamId) -> Result<Vec<u8>, SessionFailure>;

    async fn stream_detach(&self, stream: &StreamId) -> Result<(), SessionFailure>;

    async fn stream_close(&self, stream: &StreamId) -> Result<(), SessionFailure>;

    async fn stream_cancel(&self, stream: &StreamId) -> Result<(), SessionFailure>;
}

pub struct ConnectedSession {
    pub control: Arc<dyn ComponentSession>,
    pub ttrpc_socket: Socket,
}

impl fmt::Debug for ConnectedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectedSession")
            .field("generation", &self.control.generation())
            .finish()
    }
}

#[async_trait]
pub trait ComponentSessionConnector: Send + Sync {
    async fn connect(
        &self,
        target: &ResolvedTarget,
        service: ServiceKind,
    ) -> Result<ConnectedSession, ClientError>;
}

const STREAM_OPEN: u8 = 0;
const STREAM_DETACHED: u8 = 1;
const STREAM_CLOSED: u8 = 2;
const STREAM_CANCELLED: u8 = 3;

pub struct NamedStream {
    session: Arc<dyn ComponentSession>,
    id: StreamId,
    state: AtomicU8,
    operation_lock: tokio::sync::Mutex<()>,
    changed: Notify,
    permit: StreamPermit,
}

impl fmt::Debug for NamedStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NamedStream")
            .field("state", &self.state.load(Ordering::Acquire))
            .finish()
    }
}

impl NamedStream {
    pub(crate) fn new(
        session: Arc<dyn ComponentSession>,
        id: StreamId,
        active: Arc<AtomicU16>,
    ) -> Self {
        Self {
            session,
            id,
            state: AtomicU8::new(STREAM_OPEN),
            operation_lock: tokio::sync::Mutex::new(()),
            changed: Notify::new(),
            permit: StreamPermit {
                active,
                released: AtomicBool::new(false),
            },
        }
    }

    fn require_open(&self) -> Result<(), ClientError> {
        match self.state.load(Ordering::Acquire) {
            STREAM_OPEN => Ok(()),
            STREAM_DETACHED => Err(ClientError::StreamDetached),
            _ => Err(ClientError::StreamClosed),
        }
    }

    pub async fn send(&self, message: &[u8]) -> Result<(), ClientError> {
        if message.len() > MAX_LOGICAL_MESSAGE_BYTES as usize
            || message.len() > MAX_NAMED_STREAM_QUEUE_BYTES as usize
        {
            return Err(ClientError::StreamLimitExceeded);
        }
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        self.session
            .stream_send(&self.id, message)
            .await
            .map_err(map_session_failure)
    }

    pub async fn receive(&self) -> Result<Vec<u8>, ClientError> {
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        let message = self
            .session
            .stream_receive(&self.id)
            .await
            .map_err(map_session_failure)?;
        if message.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
            return Err(ClientError::ContractViolation);
        }
        Ok(message)
    }

    pub async fn detach(&self) -> Result<(), ClientError> {
        self.transition(STREAM_DETACHED, |session, id| async move {
            session.stream_detach(&id).await
        })
        .await
    }

    pub async fn close(&self) -> Result<(), ClientError> {
        self.transition(STREAM_CLOSED, |session, id| async move {
            session.stream_close(&id).await
        })
        .await
    }

    pub async fn cancel(&self) -> Result<(), ClientError> {
        self.transition(STREAM_CANCELLED, |session, id| async move {
            session.stream_cancel(&id).await
        })
        .await
    }

    async fn transition<F, Fut>(&self, next: u8, operation: F) -> Result<(), ClientError>
    where
        F: FnOnce(Arc<dyn ComponentSession>, StreamId) -> Fut,
        Fut: std::future::Future<Output = Result<(), SessionFailure>>,
    {
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        operation(Arc::clone(&self.session), self.id.clone())
            .await
            .map_err(map_session_failure)?;
        self.state.store(next, Ordering::Release);
        self.permit.release();
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn is_terminal(&self) -> bool {
        self.state.load(Ordering::Acquire) != STREAM_OPEN
    }
}

struct StreamPermit {
    active: Arc<AtomicU16>,
    released: AtomicBool,
}

impl StreamPermit {
    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for NamedStream {
    fn drop(&mut self) {
        self.permit.release();
    }
}

pub(crate) fn map_session_failure(failure: SessionFailure) -> ClientError {
    match failure {
        SessionFailure::BeforeDispatch | SessionFailure::Retryable => ClientError::TransportFailed,
        SessionFailure::Ambiguous => ClientError::AmbiguousMutation,
        SessionFailure::Disconnected => ClientError::SessionLost,
        SessionFailure::Deadline => ClientError::DeadlineExpired,
        SessionFailure::Cancelled => ClientError::Cancelled,
        SessionFailure::Protocol => ClientError::ContractViolation,
    }
}
