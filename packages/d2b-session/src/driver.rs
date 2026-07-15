use std::{
    collections::VecDeque,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use async_trait::async_trait;
use d2b_contracts::v2_component_session::{CloseReason, Remediation, RequestId, SessionErrorCode};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Cancellation, Fragment, OwnedAttachment, OwnedTransport, Result, SessionEngine, SessionError,
    SessionEvent, StreamEvent, StreamId,
};

const DRIVER_COMMAND_CAPACITY: usize = 128;
const DRIVER_EVENT_CAPACITY: usize = 128;

/// Object-safe, clonable control surface for one established ComponentSession.
///
/// Ttrpc frames stay opaque: generated ttrpc code owns framing and correlation,
/// while ComponentSession owns protection, fragmentation, cancellation,
/// attachments, and named-stream multiplexing.
#[async_trait]
pub trait ComponentSessionDriver: Send + Sync {
    fn generation(&self) -> u64;

    /// Registers and sends one outbound ttrpc request. Response correlation
    /// remains with the ttrpc adapter through `receive_ttrpc`.
    async fn start_ttrpc(&self, request_id: RequestId, frame: Vec<u8>) -> Result<()>;

    /// Removes a completed outbound request after the ttrpc adapter has paired
    /// the response frame with its local stream.
    async fn complete_ttrpc(&self, request_id: RequestId) -> Result<bool>;

    async fn cancel(&self, generation: u64, request_id: RequestId) -> Result<()>;

    async fn send_ttrpc(&self, frame: Vec<u8>) -> Result<()>;

    async fn receive_ttrpc(&self) -> Result<Vec<u8>>;

    /// Registers an authenticated inbound request before handler dispatch.
    async fn register_inbound_call(&self, request_id: RequestId) -> Result<Cancellation>;

    /// Removes a normally completed inbound request.
    async fn complete_inbound_call(&self, request_id: RequestId) -> Result<bool>;

    /// Cancels and removes an aborted inbound request.
    async fn remove_inbound_call(&self, request_id: RequestId) -> Result<bool>;

    async fn send_attachments(&self, attachments: Vec<OwnedAttachment>) -> Result<()>;

    async fn receive_attachments(&self) -> Result<Vec<OwnedAttachment>>;

    async fn open_named_stream(
        &self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<()>;

    /// Sends one logical message, fragmenting internally as stream credit
    /// becomes available.
    async fn send_named_stream(&self, stream: StreamId, bytes: Vec<u8>) -> Result<()>;

    async fn receive_named_stream(&self) -> Result<StreamEvent>;

    /// Reports application consumption in logical plaintext bytes.
    async fn grant_named_stream_credit(&self, stream: StreamId, bytes: u32) -> Result<()>;

    async fn close_named_stream(&self, stream: StreamId) -> Result<()>;

    async fn reset_named_stream(&self, stream: StreamId) -> Result<()>;

    async fn drive_keepalive(&self, now: Instant) -> Result<()>;

    async fn receive_control(&self) -> Result<SessionEvent>;

    async fn close(&self, reason: CloseReason, remediation: Remediation) -> Result<()>;
}

#[derive(Clone)]
pub struct SessionDriverHandle {
    commands: mpsc::Sender<DriverCommand>,
    generation: Arc<AtomicU64>,
}

impl fmt::Debug for SessionDriverHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionDriverHandle")
            .field("generation", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl SessionDriverHandle {
    async fn request<R>(
        &self,
        make_command: impl FnOnce(oneshot::Sender<Result<R>>) -> DriverCommand,
    ) -> Result<R> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .send(make_command(reply))
            .await
            .map_err(|_| disconnected())?;
        receive.await.map_err(|_| disconnected())?
    }
}

#[async_trait]
impl ComponentSessionDriver for SessionDriverHandle {
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    async fn start_ttrpc(&self, request_id: RequestId, frame: Vec<u8>) -> Result<()> {
        self.request(|reply| DriverCommand::StartTtrpc {
            request_id,
            frame,
            reply,
        })
        .await
    }

    async fn complete_ttrpc(&self, request_id: RequestId) -> Result<bool> {
        self.request(|reply| DriverCommand::CompleteTtrpc { request_id, reply })
            .await
    }

    async fn cancel(&self, generation: u64, request_id: RequestId) -> Result<()> {
        self.request(|reply| DriverCommand::Cancel {
            generation,
            request_id,
            reply,
        })
        .await
    }

    async fn send_ttrpc(&self, frame: Vec<u8>) -> Result<()> {
        self.request(|reply| DriverCommand::SendTtrpc { frame, reply })
            .await
    }

    async fn receive_ttrpc(&self) -> Result<Vec<u8>> {
        self.request(DriverCommand::ReceiveTtrpc).await
    }

    async fn register_inbound_call(&self, request_id: RequestId) -> Result<Cancellation> {
        self.request(|reply| DriverCommand::RegisterInboundCall { request_id, reply })
            .await
    }

    async fn complete_inbound_call(&self, request_id: RequestId) -> Result<bool> {
        self.request(|reply| DriverCommand::CompleteInboundCall { request_id, reply })
            .await
    }

    async fn remove_inbound_call(&self, request_id: RequestId) -> Result<bool> {
        self.request(|reply| DriverCommand::RemoveInboundCall { request_id, reply })
            .await
    }

    async fn send_attachments(&self, attachments: Vec<OwnedAttachment>) -> Result<()> {
        self.request(|reply| DriverCommand::SendAttachments { attachments, reply })
            .await
    }

    async fn receive_attachments(&self) -> Result<Vec<OwnedAttachment>> {
        self.request(DriverCommand::ReceiveAttachments).await
    }

    async fn open_named_stream(
        &self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<()> {
        self.request(|reply| DriverCommand::OpenNamedStream {
            stream,
            send_credit,
            receive_credit,
            reply,
        })
        .await
    }

    async fn send_named_stream(&self, stream: StreamId, bytes: Vec<u8>) -> Result<()> {
        self.request(|reply| DriverCommand::SendNamedStream {
            stream,
            bytes,
            reply,
        })
        .await
    }

    async fn receive_named_stream(&self) -> Result<StreamEvent> {
        self.request(DriverCommand::ReceiveNamedStream).await
    }

    async fn grant_named_stream_credit(&self, stream: StreamId, bytes: u32) -> Result<()> {
        self.request(|reply| DriverCommand::GrantNamedStreamCredit {
            stream,
            bytes,
            reply,
        })
        .await
    }

    async fn close_named_stream(&self, stream: StreamId) -> Result<()> {
        self.request(|reply| DriverCommand::CloseNamedStream { stream, reply })
            .await
    }

    async fn reset_named_stream(&self, stream: StreamId) -> Result<()> {
        self.request(|reply| DriverCommand::ResetNamedStream { stream, reply })
            .await
    }

    async fn drive_keepalive(&self, now: Instant) -> Result<()> {
        self.request(|reply| DriverCommand::DriveKeepalive { now, reply })
            .await
    }

    async fn receive_control(&self) -> Result<SessionEvent> {
        self.request(DriverCommand::ReceiveControl).await
    }

    async fn close(&self, reason: CloseReason, remediation: Remediation) -> Result<()> {
        self.request(|reply| DriverCommand::Close {
            reason,
            remediation,
            reply,
        })
        .await
    }
}

impl<T: OwnedTransport + 'static> SessionEngine<T> {
    pub fn into_driver(self) -> SessionDriverHandle {
        let generation = Arc::new(AtomicU64::new(self.generation()));
        let (commands, receiver) = mpsc::channel(DRIVER_COMMAND_CAPACITY);
        tokio::spawn(run_driver(self, receiver));
        SessionDriverHandle {
            commands,
            generation,
        }
    }
}

enum DriverCommand {
    StartTtrpc {
        request_id: RequestId,
        frame: Vec<u8>,
        reply: Reply<()>,
    },
    CompleteTtrpc {
        request_id: RequestId,
        reply: Reply<bool>,
    },
    Cancel {
        generation: u64,
        request_id: RequestId,
        reply: Reply<()>,
    },
    SendTtrpc {
        frame: Vec<u8>,
        reply: Reply<()>,
    },
    ReceiveTtrpc(Reply<Vec<u8>>),
    RegisterInboundCall {
        request_id: RequestId,
        reply: Reply<Cancellation>,
    },
    CompleteInboundCall {
        request_id: RequestId,
        reply: Reply<bool>,
    },
    RemoveInboundCall {
        request_id: RequestId,
        reply: Reply<bool>,
    },
    SendAttachments {
        attachments: Vec<OwnedAttachment>,
        reply: Reply<()>,
    },
    ReceiveAttachments(Reply<Vec<OwnedAttachment>>),
    OpenNamedStream {
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
        reply: Reply<()>,
    },
    SendNamedStream {
        stream: StreamId,
        bytes: Vec<u8>,
        reply: Reply<()>,
    },
    ReceiveNamedStream(Reply<StreamEvent>),
    GrantNamedStreamCredit {
        stream: StreamId,
        bytes: u32,
        reply: Reply<()>,
    },
    CloseNamedStream {
        stream: StreamId,
        reply: Reply<()>,
    },
    ResetNamedStream {
        stream: StreamId,
        reply: Reply<()>,
    },
    DriveKeepalive {
        now: Instant,
        reply: Reply<()>,
    },
    ReceiveControl(Reply<SessionEvent>),
    Close {
        reason: CloseReason,
        remediation: Remediation,
        reply: Reply<()>,
    },
}

type Reply<T> = oneshot::Sender<Result<T>>;

struct PendingNamedSend {
    stream: StreamId,
    fragments: VecDeque<Fragment>,
    remaining: usize,
    reply: Reply<()>,
}

struct DriverQueues {
    named_sends: VecDeque<PendingNamedSend>,
    named_send_bytes: usize,
    ttrpc: EventQueue<Vec<u8>>,
    attachments: EventQueue<Vec<OwnedAttachment>>,
    streams: EventQueue<StreamEvent>,
    control: EventQueue<SessionEvent>,
}

impl DriverQueues {
    fn new() -> Self {
        Self {
            named_sends: VecDeque::new(),
            named_send_bytes: 0,
            ttrpc: EventQueue::new(),
            attachments: EventQueue::new(),
            streams: EventQueue::new(),
            control: EventQueue::new(),
        }
    }

    fn can_enqueue_named_send(
        &self,
        stream: StreamId,
        bytes: usize,
        aggregate_limit: usize,
    ) -> Result<()> {
        let aggregate = self
            .named_send_bytes
            .checked_add(bytes)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if bytes == 0
            || aggregate > aggregate_limit
            || self
                .named_sends
                .iter()
                .any(|pending| pending.stream == stream)
        {
            return Err(backpressure());
        }
        Ok(())
    }

    fn enqueue_named_send(&mut self, pending: PendingNamedSend) {
        self.named_send_bytes += pending.remaining;
        self.named_sends.push_back(pending);
    }

    fn cancel_named_send(&mut self, stream: StreamId, error: SessionError) {
        let mut retained = VecDeque::with_capacity(self.named_sends.len());
        while let Some(pending) = self.named_sends.pop_front() {
            if pending.stream == stream {
                self.named_send_bytes = self.named_send_bytes.saturating_sub(pending.remaining);
                let _ = pending.reply.send(Err(error));
            } else {
                retained.push_back(pending);
            }
        }
        self.named_sends = retained;
    }

    fn fail(self, error: SessionError) {
        for pending in self.named_sends {
            let _ = pending.reply.send(Err(error));
        }
        self.ttrpc.fail(error);
        self.attachments.fail(error);
        self.streams.fail(error);
        self.control.fail(error);
    }
}

struct EventQueue<T> {
    events: VecDeque<T>,
    waiters: VecDeque<Reply<T>>,
}

impl<T> EventQueue<T> {
    fn new() -> Self {
        Self {
            events: VecDeque::new(),
            waiters: VecDeque::new(),
        }
    }

    fn receive(&mut self, waiter: Reply<T>) -> Result<()> {
        if let Some(event) = self.events.pop_front() {
            let _ = waiter.send(Ok(event));
        } else {
            if self.waiters.len() >= DRIVER_COMMAND_CAPACITY {
                return Err(backpressure());
            }
            self.waiters.push_back(waiter);
        }
        Ok(())
    }

    fn deliver(&mut self, mut event: T) -> Result<()> {
        while let Some(waiter) = self.waiters.pop_front() {
            match waiter.send(Ok(event)) {
                Ok(()) => return Ok(()),
                Err(Ok(returned)) => event = returned,
                Err(Err(_)) => {
                    return Err(SessionError::new(SessionErrorCode::InternalInvariant));
                }
            }
        }
        if self.events.len() >= DRIVER_EVENT_CAPACITY {
            return Err(backpressure());
        }
        self.events.push_back(event);
        Ok(())
    }

    fn fail(self, error: SessionError) {
        for waiter in self.waiters {
            let _ = waiter.send(Err(error));
        }
    }
}

async fn run_driver<T: OwnedTransport>(
    mut engine: SessionEngine<T>,
    mut commands: mpsc::Receiver<DriverCommand>,
) {
    let mut queues = DriverQueues::new();
    let result = loop {
        match pump_named_stream(&mut engine, &mut queues).await {
            Ok(true) => {
                tokio::task::yield_now().await;
                continue;
            }
            Ok(false) => {}
            Err(error) => break Err(error),
        }
        tokio::select! {
            biased;
            command = commands.recv() => {
                let Some(command) = command else {
                    break Err(disconnected());
                };
                match handle_command(&mut engine, &mut queues, command).await {
                    Ok(DriverAction::Continue) => {}
                    Ok(DriverAction::Close) => break Ok(()),
                    Err(error) => break Err(error),
                }
            }
            event = engine.receive() => {
                match event.and_then(|event| route_event(&mut queues, event)) {
                    Ok(()) => {}
                    Err(error) => break Err(error),
                }
            }
        }
    };

    let error = result.err().unwrap_or_else(disconnected);
    queues.fail(error);
}

enum DriverAction {
    Continue,
    Close,
}

async fn handle_command<T: OwnedTransport>(
    engine: &mut SessionEngine<T>,
    queues: &mut DriverQueues,
    command: DriverCommand,
) -> Result<DriverAction> {
    match command {
        DriverCommand::StartTtrpc {
            request_id,
            frame,
            reply,
        } => {
            let result = engine.call(request_id, frame).await.map(|_| ());
            let _ = reply.send(result);
        }
        DriverCommand::CompleteTtrpc { request_id, reply } => {
            let _ = reply.send(Ok(engine.complete_call(&request_id)));
        }
        DriverCommand::Cancel {
            generation,
            request_id,
            reply,
        } => {
            let result = if generation == engine.generation() {
                engine.cancel_call(&request_id).await
            } else {
                Err(SessionError::new(SessionErrorCode::GenerationMismatch))
            };
            let _ = reply.send(result);
        }
        DriverCommand::SendTtrpc { frame, reply } => {
            let result = engine.send_ttrpc(frame).await;
            let _ = reply.send(result);
        }
        DriverCommand::ReceiveTtrpc(reply) => queues.ttrpc.receive(reply)?,
        DriverCommand::RegisterInboundCall { request_id, reply } => {
            let _ = reply.send(engine.register_inbound_call(request_id));
        }
        DriverCommand::CompleteInboundCall { request_id, reply } => {
            let _ = reply.send(Ok(engine.complete_inbound_call(&request_id)));
        }
        DriverCommand::RemoveInboundCall { request_id, reply } => {
            let _ = reply.send(Ok(engine.remove_inbound_call(&request_id)));
        }
        DriverCommand::SendAttachments { attachments, reply } => {
            let result = engine.send_attachments(attachments).await;
            let _ = reply.send(result);
        }
        DriverCommand::ReceiveAttachments(reply) => queues.attachments.receive(reply)?,
        DriverCommand::OpenNamedStream {
            stream,
            send_credit,
            receive_credit,
            reply,
        } => {
            let _ = reply.send(engine.open_named_stream(stream, send_credit, receive_credit));
        }
        DriverCommand::SendNamedStream {
            stream,
            bytes,
            reply,
        } => {
            let len = bytes.len();
            if let Err(error) =
                queues.can_enqueue_named_send(stream, len, engine.aggregate_named_stream_limit())
            {
                let _ = reply.send(Err(error));
            } else {
                match engine.fragment_named_stream(stream, bytes) {
                    Ok(fragments) => queues.enqueue_named_send(PendingNamedSend {
                        stream,
                        fragments,
                        remaining: len,
                        reply,
                    }),
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
        }
        DriverCommand::ReceiveNamedStream(reply) => queues.streams.receive(reply)?,
        DriverCommand::GrantNamedStreamCredit {
            stream,
            bytes,
            reply,
        } => {
            let result = engine.grant_named_stream_credit(stream, bytes).await;
            let _ = reply.send(result);
        }
        DriverCommand::CloseNamedStream { stream, reply } => {
            queues.cancel_named_send(stream, SessionError::new(SessionErrorCode::Cancelled));
            let result = engine.close_named_stream(stream).await;
            let _ = reply.send(result);
        }
        DriverCommand::ResetNamedStream { stream, reply } => {
            queues.cancel_named_send(stream, SessionError::new(SessionErrorCode::Cancelled));
            let result = engine.reset_named_stream(stream).await;
            let _ = reply.send(result);
        }
        DriverCommand::DriveKeepalive { now, reply } => {
            let result = engine.drive_keepalive(now).await;
            let _ = reply.send(result);
        }
        DriverCommand::ReceiveControl(reply) => queues.control.receive(reply)?,
        DriverCommand::Close {
            reason,
            remediation,
            reply,
        } => {
            let result = engine.close(reason, remediation).await;
            let closed = result.is_ok();
            let _ = reply.send(result);
            if closed {
                return Ok(DriverAction::Close);
            }
        }
    }
    Ok(DriverAction::Continue)
}

async fn pump_named_stream<T: OwnedTransport>(
    engine: &mut SessionEngine<T>,
    queues: &mut DriverQueues,
) -> Result<bool> {
    let attempts = queues.named_sends.len();
    for _ in 0..attempts {
        let Some(mut pending) = queues.named_sends.pop_front() else {
            return Ok(false);
        };
        let Some(fragment) = pending.fragments.front() else {
            let _ = pending.reply.send(Ok(()));
            continue;
        };
        let fragment_len = fragment.as_bytes().len();
        let fragment_credit = u32::try_from(fragment_len)
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if engine
            .named_stream_send_credit(pending.stream)
            .is_none_or(|credit| credit < fragment_credit)
        {
            queues.named_sends.push_back(pending);
            continue;
        }

        let fragment = pending
            .fragments
            .pop_front()
            .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
        engine
            .send_named_stream_fragment(pending.stream, fragment)
            .await?;
        pending.remaining = pending
            .remaining
            .checked_sub(fragment_len)
            .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
        queues.named_send_bytes = queues
            .named_send_bytes
            .checked_sub(fragment_len)
            .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
        if pending.fragments.is_empty() {
            let _ = pending.reply.send(Ok(()));
        } else {
            queues.named_sends.push_back(pending);
        }
        return Ok(true);
    }
    Ok(false)
}

fn route_event(queues: &mut DriverQueues, event: SessionEvent) -> Result<()> {
    match event {
        SessionEvent::Ttrpc(frame) => {
            queues.ttrpc.deliver(frame)?;
        }
        SessionEvent::Attachments(attachments) => queues.attachments.deliver(attachments)?,
        SessionEvent::NamedStream(event) => {
            if let StreamEvent::Reset { stream } = &event {
                queues.cancel_named_send(*stream, SessionError::new(SessionErrorCode::Cancelled));
            }
            queues.streams.deliver(event)?;
        }
        event => queues.control.deliver(event)?,
    }
    Ok(())
}

fn disconnected() -> SessionError {
    SessionError::new(SessionErrorCode::SessionDisconnected)
}

fn backpressure() -> SessionError {
    SessionError::new(SessionErrorCode::QueueBackpressure)
}
