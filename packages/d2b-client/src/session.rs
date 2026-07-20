use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use d2b_contracts::v2_component_session::{
    LimitProfile, MAX_ACTIVE_NAMED_STREAMS, MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES,
    MAX_LOGICAL_MESSAGE_BYTES, MAX_NAMED_STREAM_QUEUE_BYTES,
};
use d2b_session::{ComponentSessionDriver, StreamEvent, StreamId, TransportPacket};
use tokio::{
    sync::{Mutex, Notify},
    task::AbortHandle,
};
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
    pub limits: LimitProfile,
}

impl fmt::Debug for ConnectedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConnectedSession { driver: [redacted], limits: [negotiated] }")
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
const STREAM_DISPATCH_RUNNING: u8 = 0;
const STREAM_DISPATCH_TRANSPORT_FAILED: u8 = 1;
const STREAM_DISPATCH_PROTOCOL_FAILED: u8 = 2;
const STREAM_INBOX_MAX_BYTES: usize =
    MAX_LOGICAL_MESSAGE_BYTES as usize + MAX_NAMED_STREAM_QUEUE_BYTES as usize;
const STREAM_INBOX_MAX_EVENTS: usize = MAX_NAMED_STREAM_QUEUE_BYTES as usize + 2;
pub(crate) struct StreamDispatcher {
    routes: Arc<StdMutex<StreamRoutes>>,
    terminal: Arc<AtomicU8>,
    task: AbortHandle,
}

type DispatchedStreamEvent = Result<StreamEvent, ClientError>;

struct StreamRoutes {
    streams: BTreeMap<StreamId, StreamRoute>,
    retired: BTreeSet<StreamId>,
    aggregate_bytes: Arc<AtomicUsize>,
}

impl StreamRoutes {
    fn new() -> Self {
        Self {
            streams: BTreeMap::new(),
            retired: BTreeSet::new(),
            aggregate_bytes: Arc::new(AtomicUsize::new(0)),
        }
    }
}

struct StreamRoute {
    inbox: Arc<StreamInbox>,
    registered: bool,
}

struct StreamInbox {
    state: StdMutex<StreamInboxState>,
    changed: Notify,
    aggregate_bytes: Arc<AtomicUsize>,
}

#[derive(Default)]
struct StreamInboxState {
    events: VecDeque<DispatchedStreamEvent>,
    queued_bytes: usize,
    queued_events: usize,
    failed: bool,
}

impl StreamInbox {
    fn new(aggregate_bytes: Arc<AtomicUsize>) -> Self {
        Self {
            state: StdMutex::new(StreamInboxState::default()),
            changed: Notify::new(),
            aggregate_bytes,
        }
    }

    fn push(&self, event: DispatchedStreamEvent) -> Result<(), ClientError> {
        let weight = stream_event_weight(&event);
        if self
            .aggregate_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(weight)
                    .filter(|total| *total <= MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES as usize)
            })
            .is_err()
        {
            return Err(ClientError::ContractViolation);
        }
        let mut state = self.state.lock().unwrap();
        if state.failed
            || state.queued_events >= STREAM_INBOX_MAX_EVENTS
            || state
                .queued_bytes
                .checked_add(weight)
                .is_none_or(|total| total > STREAM_INBOX_MAX_BYTES)
        {
            self.aggregate_bytes.fetch_sub(weight, Ordering::AcqRel);
            return Err(ClientError::ContractViolation);
        }
        state.queued_bytes += weight;
        state.queued_events += 1;
        state.events.push_back(event);
        drop(state);
        self.changed.notify_one();
        Ok(())
    }

    fn fail(&self, error: ClientError) {
        let mut state = self.state.lock().unwrap();
        self.aggregate_bytes
            .fetch_sub(state.queued_bytes, Ordering::AcqRel);
        state.events.clear();
        state.queued_bytes = 0;
        state.queued_events = 1;
        state.failed = true;
        state.events.push_back(Err(error));
        drop(state);
        self.changed.notify_waiters();
    }

    async fn receive(&self) -> Result<StreamEvent, ClientError> {
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock().unwrap();
                if let Some(event) = state.events.pop_front() {
                    state.queued_events = state.queued_events.saturating_sub(1);
                    state.queued_bytes = state
                        .queued_bytes
                        .saturating_sub(stream_event_weight(&event));
                    self.aggregate_bytes
                        .fetch_sub(stream_event_weight(&event), Ordering::AcqRel);
                    return event;
                }
                if state.failed {
                    return Err(ClientError::TransportFailed);
                }
            }
            changed.await;
        }
    }

    fn discard(&self) {
        let mut state = self.state.lock().unwrap();
        self.aggregate_bytes
            .fetch_sub(state.queued_bytes, Ordering::AcqRel);
        state.events.clear();
        state.queued_bytes = 0;
        state.queued_events = 0;
        state.failed = true;
        drop(state);
        self.changed.notify_waiters();
    }
}

impl Drop for StreamInbox {
    fn drop(&mut self) {
        let queued_bytes = self.state.get_mut().unwrap().queued_bytes;
        if queued_bytes != 0 {
            self.aggregate_bytes
                .fetch_sub(queued_bytes, Ordering::AcqRel);
        }
    }
}

fn stream_event_weight(event: &DispatchedStreamEvent) -> usize {
    match event {
        Ok(StreamEvent::Data { bytes, .. }) => bytes.len().max(1),
        Ok(StreamEvent::RemoteClosed { .. } | StreamEvent::Reset { .. }) => 0,
        Err(_) => 0,
    }
}

impl StreamDispatcher {
    pub(crate) fn new(driver: SharedDriver) -> Arc<Self> {
        let routes = Arc::new(StdMutex::new(StreamRoutes::new()));
        let terminal = Arc::new(AtomicU8::new(STREAM_DISPATCH_RUNNING));
        let task = tokio::spawn(run_stream_dispatcher(
            driver,
            Arc::clone(&routes),
            Arc::clone(&terminal),
        ));
        Arc::new(Self {
            routes,
            terminal,
            task: task.abort_handle(),
        })
    }

    fn register(
        self: &Arc<Self>,
        stream: StreamId,
    ) -> Result<(Arc<StreamInbox>, StreamRegistration), ClientError> {
        let mut routes = self.routes.lock().unwrap();
        if let Some(error) = stream_dispatch_error(self.terminal.load(Ordering::Acquire)) {
            return Err(error);
        }
        if routes.retired.contains(&stream) {
            return Err(ClientError::ContractViolation);
        }
        let inbox = match routes.streams.get_mut(&stream) {
            Some(route) if route.registered => return Err(ClientError::ContractViolation),
            Some(route) => {
                route.registered = true;
                Arc::clone(&route.inbox)
            }
            None => {
                if routes.streams.len() >= usize::from(MAX_ACTIVE_NAMED_STREAMS) {
                    return Err(ClientError::StreamLimitExceeded);
                }
                let inbox = Arc::new(StreamInbox::new(Arc::clone(&routes.aggregate_bytes)));
                routes.streams.insert(
                    stream,
                    StreamRoute {
                        inbox: Arc::clone(&inbox),
                        registered: true,
                    },
                );
                inbox
            }
        };
        Ok((
            inbox,
            StreamRegistration {
                dispatcher: Arc::clone(self),
                stream,
                removed: AtomicBool::new(false),
            },
        ))
    }

    fn unregister(&self, stream: StreamId) {
        let mut routes = self.routes.lock().unwrap();
        if let Some(route) = routes.streams.remove(&stream) {
            route.inbox.discard();
        }
        routes.retired.insert(stream);
    }
}

impl Drop for StreamDispatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_stream_dispatcher(
    driver: SharedDriver,
    routes: Arc<StdMutex<StreamRoutes>>,
    terminal: Arc<AtomicU8>,
) {
    loop {
        let event = match driver.receive_named_stream().await {
            Ok(event) => event,
            Err(_) => {
                fail_stream_dispatch(
                    &routes,
                    &terminal,
                    STREAM_DISPATCH_TRANSPORT_FAILED,
                    ClientError::TransportFailed,
                );
                return;
            }
        };
        let stream = match &event {
            StreamEvent::Data { stream, .. }
            | StreamEvent::RemoteClosed { stream }
            | StreamEvent::Reset { stream } => *stream,
        };
        let inbox = {
            let mut routes = routes.lock().unwrap();
            if routes.retired.contains(&stream) {
                continue;
            } else if let Some(route) = routes.streams.get(&stream) {
                Some(Arc::clone(&route.inbox))
            } else if routes.streams.len() >= usize::from(MAX_ACTIVE_NAMED_STREAMS) {
                None
            } else {
                let inbox = Arc::new(StreamInbox::new(Arc::clone(&routes.aggregate_bytes)));
                routes.streams.insert(
                    stream,
                    StreamRoute {
                        inbox: Arc::clone(&inbox),
                        registered: false,
                    },
                );
                Some(inbox)
            }
        };
        let Some(inbox) = inbox else {
            fail_stream_dispatch(
                &routes,
                &terminal,
                STREAM_DISPATCH_PROTOCOL_FAILED,
                ClientError::ContractViolation,
            );
            return;
        };
        if inbox.push(Ok(event)).is_err() {
            fail_stream_dispatch(
                &routes,
                &terminal,
                STREAM_DISPATCH_PROTOCOL_FAILED,
                ClientError::ContractViolation,
            );
            return;
        }
    }
}

fn fail_stream_dispatch(
    routes: &StdMutex<StreamRoutes>,
    terminal: &AtomicU8,
    terminal_state: u8,
    error: ClientError,
) {
    terminal.store(terminal_state, Ordering::Release);
    let mut routes = routes.lock().unwrap();
    for (_, route) in std::mem::take(&mut routes.streams) {
        route.inbox.fail(error);
    }
}

fn stream_dispatch_error(state: u8) -> Option<ClientError> {
    match state {
        STREAM_DISPATCH_RUNNING => None,
        STREAM_DISPATCH_TRANSPORT_FAILED => Some(ClientError::TransportFailed),
        _ => Some(ClientError::ContractViolation),
    }
}

struct StreamRegistration {
    dispatcher: Arc<StreamDispatcher>,
    stream: StreamId,
    removed: AtomicBool,
}

impl StreamRegistration {
    fn unregister(&self) {
        if !self.removed.swap(true, Ordering::AcqRel) {
            self.dispatcher.unregister(self.stream);
        }
    }
}

impl Drop for StreamRegistration {
    fn drop(&mut self) {
        self.unregister();
    }
}

pub struct NamedStream {
    driver: SharedDriver,
    id: StreamId,
    state: AtomicU8,
    operation_lock: Mutex<()>,
    receiver: Arc<StreamInbox>,
    registration: StreamRegistration,
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
        driver: SharedDriver,
        dispatcher: &Arc<StreamDispatcher>,
        id: StreamId,
        active: Arc<AtomicU16>,
    ) -> Result<Self, ClientError> {
        let (receiver, registration) = dispatcher.register(id)?;
        Ok(Self {
            driver,
            id,
            state: AtomicU8::new(STREAM_OPEN),
            operation_lock: Mutex::new(()),
            receiver,
            registration,
            changed: Notify::new(),
            permit: StreamPermit {
                active,
                released: AtomicBool::new(false),
            },
        })
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
        match self.receiver.receive().await? {
            StreamEvent::Data { stream, bytes } if stream == self.id => {
                if bytes.is_empty() || bytes.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
                    return Err(ClientError::ContractViolation);
                }
                let consumed =
                    u32::try_from(bytes.len()).map_err(|_| ClientError::ContractViolation)?;
                self.driver
                    .grant_named_stream_credit(self.id, consumed)
                    .await
                    .map_err(|_| ClientError::TransportFailed)?;
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
        self.registration.unregister();
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
        self.registration.unregister();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_inboxes_enforce_the_session_aggregate_bound() {
        let aggregate = Arc::new(AtomicUsize::new(0));
        let mut inboxes = Vec::new();
        for channel in 256..=259 {
            let inbox = Arc::new(StreamInbox::new(Arc::clone(&aggregate)));
            inbox
                .push(Ok(StreamEvent::Data {
                    stream: StreamId::new(channel).unwrap(),
                    bytes: vec![0; 1024 * 1024],
                }))
                .unwrap();
            inboxes.push(inbox);
        }
        let overflow = StreamInbox::new(Arc::clone(&aggregate));
        assert_eq!(
            overflow
                .push(Ok(StreamEvent::Data {
                    stream: StreamId::new(260).unwrap(),
                    bytes: vec![0],
                }))
                .unwrap_err(),
            ClientError::ContractViolation
        );

        inboxes.pop().unwrap().discard();
        overflow
            .push(Ok(StreamEvent::Data {
                stream: StreamId::new(260).unwrap(),
                bytes: vec![0],
            }))
            .unwrap();
    }
}
