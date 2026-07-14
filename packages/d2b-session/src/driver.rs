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
    OwnedAttachment, OwnedTransport, Result, SessionEngine, SessionError, SessionEvent,
    StreamEvent, StreamId,
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

    async fn invoke(&self, request_id: RequestId, frame: Vec<u8>) -> Result<Vec<u8>>;

    async fn cancel(&self, generation: u64, request_id: RequestId) -> Result<()>;

    async fn send_ttrpc(&self, frame: Vec<u8>) -> Result<()>;

    async fn receive_ttrpc(&self) -> Result<Vec<u8>>;

    async fn send_attachments(&self, attachments: Vec<OwnedAttachment>) -> Result<()>;

    async fn receive_attachments(&self) -> Result<Vec<OwnedAttachment>>;

    async fn open_named_stream(
        &self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<()>;

    async fn send_named_stream(&self, stream: StreamId, bytes: Vec<u8>) -> Result<()>;

    async fn receive_named_stream(&self) -> Result<StreamEvent>;

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

    async fn invoke(&self, request_id: RequestId, frame: Vec<u8>) -> Result<Vec<u8>> {
        self.request(|reply| DriverCommand::Invoke {
            request_id,
            frame,
            reply,
        })
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
    Invoke {
        request_id: RequestId,
        frame: Vec<u8>,
        reply: Reply<Vec<u8>>,
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

struct PendingInvoke {
    request_id: RequestId,
    frame: Vec<u8>,
    reply: Reply<Vec<u8>>,
}

struct DriverQueues {
    invokes: VecDeque<PendingInvoke>,
    active_invoke: Option<PendingInvoke>,
    ttrpc: EventQueue<Vec<u8>>,
    attachments: EventQueue<Vec<OwnedAttachment>>,
    streams: EventQueue<StreamEvent>,
    control: EventQueue<SessionEvent>,
}

impl DriverQueues {
    fn new() -> Self {
        Self {
            invokes: VecDeque::new(),
            active_invoke: None,
            ttrpc: EventQueue::new(),
            attachments: EventQueue::new(),
            streams: EventQueue::new(),
            control: EventQueue::new(),
        }
    }

    fn enqueue_invoke(&mut self, invoke: PendingInvoke) -> Result<()> {
        if self.invokes.len() >= DRIVER_COMMAND_CAPACITY {
            return Err(backpressure());
        }
        self.invokes.push_back(invoke);
        Ok(())
    }

    fn fail(mut self, error: SessionError) {
        if let Some(active) = self.active_invoke.take() {
            let _ = active.reply.send(Err(error));
        }
        for invoke in self.invokes {
            let _ = invoke.reply.send(Err(error));
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

    fn deliver(&mut self, event: T) -> Result<()> {
        if let Some(waiter) = self.waiters.pop_front() {
            let _ = waiter.send(Ok(event));
        } else {
            if self.events.len() >= DRIVER_EVENT_CAPACITY {
                return Err(backpressure());
            }
            self.events.push_back(event);
        }
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
        if queues.active_invoke.is_none()
            && let Some(invoke) = queues.invokes.pop_front()
        {
            match engine
                .call(invoke.request_id.clone(), invoke.frame.clone())
                .await
            {
                Ok(_) => queues.active_invoke = Some(invoke),
                Err(error) => {
                    let _ = invoke.reply.send(Err(error));
                    continue;
                }
            }
        }

        tokio::select! {
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
                match event.and_then(|event| route_event(&mut engine, &mut queues, event)) {
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
        DriverCommand::Invoke {
            request_id,
            frame,
            reply,
        } => queues.enqueue_invoke(PendingInvoke {
            request_id,
            frame,
            reply,
        })?,
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
            let result = engine.send_named_stream(stream, bytes).await;
            let _ = reply.send(result);
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
            let result = engine.close_named_stream(stream).await;
            let _ = reply.send(result);
        }
        DriverCommand::ResetNamedStream { stream, reply } => {
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

fn route_event<T: OwnedTransport>(
    engine: &mut SessionEngine<T>,
    queues: &mut DriverQueues,
    event: SessionEvent,
) -> Result<()> {
    match event {
        SessionEvent::Ttrpc(frame) => {
            if let Some(invoke) = queues.active_invoke.take() {
                engine.complete_call(&invoke.request_id);
                let _ = invoke.reply.send(Ok(frame));
            } else {
                queues.ttrpc.deliver(frame)?;
            }
        }
        SessionEvent::Attachments(attachments) => queues.attachments.deliver(attachments)?,
        SessionEvent::NamedStream(event) => queues.streams.deliver(event)?,
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
