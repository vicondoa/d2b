use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering},
    },
};

use async_trait::async_trait;
use d2b_contracts::v2_component_session::MAX_LOGICAL_MESSAGE_BYTES;
use d2b_session::{ComponentSessionDriver, StreamEvent, StreamId, TransportPacket};
use tokio::sync::{Mutex, Notify};
use ttrpc::r#async::transport::Socket;

use crate::{ClientError, MethodHandle, ResolvedTarget, ServiceKind};

pub type SharedDriver = Arc<dyn ComponentSessionDriver>;

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
    pub packet: TransportPacket,
    pub relative_timeout_nanos: u64,
}

impl fmt::Debug for SessionCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCall")
            .field("service", &self.method.service())
            .field("method_index", &self.method.index())
            .field("attachment_count", &self.packet.attachments().len())
            .finish()
    }
}

pub struct SessionReply {
    pub packet: TransportPacket,
}

impl fmt::Debug for SessionReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionReply")
            .field("attachment_count", &self.packet.attachments().len())
            .finish()
    }
}

pub struct ConnectedSession {
    pub driver: SharedDriver,
    pub ttrpc_socket: Socket,
}

impl fmt::Debug for ConnectedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConnectedSession { driver: [redacted] }")
    }
}

#[async_trait]
pub trait ComponentSessionConnector: Send + Sync + 'static {
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
    driver: SharedDriver,
    id: StreamId,
    state: AtomicU8,
    operation_lock: Mutex<()>,
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
    pub(crate) fn new(driver: SharedDriver, id: StreamId, active: Arc<AtomicU16>) -> Self {
        Self {
            driver,
            id,
            state: AtomicU8::new(STREAM_OPEN),
            operation_lock: Mutex::new(()),
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
        if message.is_empty() || message.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
            return Err(ClientError::StreamLimitExceeded);
        }
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        self.driver
            .send_named_stream(self.id, message.to_vec())
            .await
            .map_err(|_| ClientError::TransportFailed)
    }

    pub async fn receive(&self) -> Result<Vec<u8>, ClientError> {
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        match self
            .driver
            .receive_named_stream()
            .await
            .map_err(|_| ClientError::TransportFailed)?
        {
            StreamEvent::Data { stream, bytes } if stream == self.id => {
                if bytes.is_empty() || bytes.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
                    return Err(ClientError::ContractViolation);
                }
                Ok(bytes)
            }
            StreamEvent::RemoteClosed { stream } if stream == self.id => {
                self.finish(STREAM_CLOSED);
                Err(ClientError::StreamClosed)
            }
            StreamEvent::Reset { stream } if stream == self.id => {
                self.finish(STREAM_CANCELLED);
                Err(ClientError::StreamClosed)
            }
            _ => Err(ClientError::ContractViolation),
        }
    }

    pub async fn detach(&self) -> Result<(), ClientError> {
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        self.finish(STREAM_DETACHED);
        Ok(())
    }

    pub async fn close(&self) -> Result<(), ClientError> {
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        self.driver
            .close_named_stream(self.id)
            .await
            .map_err(|_| ClientError::TransportFailed)?;
        self.finish(STREAM_CLOSED);
        Ok(())
    }

    pub async fn cancel(&self) -> Result<(), ClientError> {
        let _guard = self.operation_lock.lock().await;
        self.require_open()?;
        self.driver
            .reset_named_stream(self.id)
            .await
            .map_err(|_| ClientError::TransportFailed)?;
        self.finish(STREAM_CANCELLED);
        Ok(())
    }

    fn finish(&self, next: u8) {
        self.state.store(next, Ordering::Release);
        self.permit.release();
        self.changed.notify_waiters();
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
