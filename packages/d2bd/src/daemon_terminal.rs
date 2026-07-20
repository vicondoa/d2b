//! Server-owned ComponentSession terminal streams.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::v2_services::{
    MAX_TERMINAL_CHUNK_BYTES, MIN_NAMED_STREAM_ID, RedactedTerminalFrame, ServerStreamLease,
    StrictWireMessage, TerminalFrameDirection, TerminalStreamValidator, common,
    terminal::{self, terminal_stream_frame},
};
use d2b_session::{Cancellation, ComponentSessionDriver, StreamEvent, StreamId};
use protobuf::{EnumOrUnknown, Message, MessageField};
use tokio::sync::mpsc;

const TERMINAL_EVENT_QUEUE: usize = 32;
const TERMINAL_STREAM_CREDIT: u32 = 256 * 1024;
const TERMINAL_POLL_INTERVAL: Duration = Duration::from_millis(50);
const TERMINAL_SELECTION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalFailure {
    InvalidSelection,
    Unauthorized,
    NotFound,
    Conflict,
    Unavailable,
    ResourceExhausted,
    GenerationMismatch,
    Protocol,
    Internal,
}

impl TerminalFailure {
    pub fn error_kind(self) -> terminal::TerminalErrorKind {
        match self {
            Self::InvalidSelection => {
                terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_INVALID_SELECTION
            }
            Self::Unauthorized => terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_UNAUTHORIZED,
            Self::NotFound => terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_NOT_FOUND,
            Self::Conflict => terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_CONFLICT,
            Self::Unavailable => terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_UNAVAILABLE,
            Self::ResourceExhausted => {
                terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_RESOURCE_EXHAUSTED
            }
            Self::GenerationMismatch => {
                terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_GENERATION_MISMATCH
            }
            Self::Protocol => terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_PROTOCOL,
            Self::Internal => terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_INTERNAL,
        }
    }

    pub fn common_error_kind(self) -> common::ErrorKind {
        match self {
            Self::InvalidSelection | Self::Protocol => {
                common::ErrorKind::ERROR_KIND_INVALID_REQUEST
            }
            Self::Unauthorized => common::ErrorKind::ERROR_KIND_UNAUTHORIZED,
            Self::NotFound => common::ErrorKind::ERROR_KIND_NOT_FOUND,
            Self::Conflict => common::ErrorKind::ERROR_KIND_CONFLICT,
            Self::Unavailable => common::ErrorKind::ERROR_KIND_UNAVAILABLE,
            Self::ResourceExhausted => common::ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED,
            Self::GenerationMismatch => common::ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
            Self::Internal => common::ErrorKind::ERROR_KIND_INTERNAL,
        }
    }

    pub fn retry(self) -> common::RetryClass {
        match self {
            Self::Unavailable | Self::ResourceExhausted => {
                common::RetryClass::RETRY_CLASS_AFTER_OBSERVATION
            }
            Self::Conflict => common::RetryClass::RETRY_CLASS_SAME_OPERATION,
            _ => common::RetryClass::RETRY_CLASS_NEVER,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct TerminalBinding {
    pub session_generation: u64,
    pub request_id: [u8; 16],
    pub operation_id: String,
    pub resource_handle: String,
    pub peer_principal: String,
    pub peer_uid: u32,
    pub kind: terminal::TerminalKind,
    pub retained_log: Option<terminal::TerminalRetainedLogRange>,
}

impl fmt::Debug for TerminalBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalBinding")
            .field("session_generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("resource_handle", &"<redacted>")
            .field("peer_principal", &"<redacted>")
            .field("peer_uid", &self.peer_uid)
            .field("kind", &self.kind)
            .finish()
    }
}

pub enum TerminalOpenResult {
    Active {
        started: terminal::TerminalStarted,
        owner: Box<dyn TerminalOwner>,
    },
    ActiveWithoutStarted {
        owner: Box<dyn TerminalOwner>,
    },
    Immediate(Vec<TerminalOwnerEvent>),
    Terminal(terminal::TerminalOutcome),
}

#[async_trait]
pub trait PreparedTerminal: Send + Sync {
    async fn open(
        &self,
        binding: &TerminalBinding,
        selection: terminal::TerminalSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure>;

    async fn abandoned(&self) {}
}

pub enum TerminalCommand {
    Stdin {
        offset: u64,
        data: Vec<u8>,
        eof: bool,
    },
    Resize {
        operation_sequence: u64,
        rows: u32,
        columns: u32,
    },
    Signal {
        operation_sequence: u64,
        signal: terminal::TerminalSignalKind,
    },
    CloseStdin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalFinish {
    Detach,
    Close,
    Cancel,
    Disconnect,
}

pub enum TerminalOwnerEvent {
    Output {
        stream: TerminalOutputStream,
        offset: u64,
        data: Vec<u8>,
        eof: bool,
        dropped_bytes: u64,
        truncated: bool,
    },
    Status {
        status: terminal::TerminalStatusKind,
        next_stdin_offset: u64,
    },
    Outcome(terminal::TerminalOutcome),
    ShellResult(terminal::ShellManagementResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutputStream {
    Stdout,
    Stderr,
}

#[async_trait]
pub trait TerminalOwner: Send {
    async fn command(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure>;

    async fn poll(&mut self) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure>;

    async fn finish(
        &mut self,
        finish: TerminalFinish,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure>;
}

enum RoutedTerminalEvent {
    Data { bytes: Vec<u8> },
    RemoteClosed,
    Reset,
    Cancelled,
    SessionClosed,
}

struct ActiveTerminal {
    binding: TerminalBinding,
    sender: mpsc::Sender<RoutedTerminalEvent>,
    cancellation: Cancellation,
}

struct TerminalTable {
    next_stream: u16,
    active: BTreeMap<u16, ActiveTerminal>,
}

pub struct TerminalSessionManager {
    driver: Arc<dyn ComponentSessionDriver>,
    generation: u64,
    maximum_streams: usize,
    selection_timeout: Duration,
    table: Mutex<TerminalTable>,
}

impl fmt::Debug for TerminalSessionManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let active = self
            .table
            .lock()
            .map(|table| table.active.len())
            .unwrap_or(0);
        formatter
            .debug_struct("TerminalSessionManager")
            .field("generation", &"<redacted>")
            .field("active", &active)
            .field("maximum_streams", &self.maximum_streams)
            .field("selection_timeout", &self.selection_timeout)
            .finish()
    }
}

impl TerminalSessionManager {
    pub fn new(
        driver: Arc<dyn ComponentSessionDriver>,
        maximum_streams: usize,
    ) -> Result<Arc<Self>, TerminalFailure> {
        Self::with_selection_timeout(driver, maximum_streams, TERMINAL_SELECTION_TIMEOUT)
    }

    fn with_selection_timeout(
        driver: Arc<dyn ComponentSessionDriver>,
        maximum_streams: usize,
        selection_timeout: Duration,
    ) -> Result<Arc<Self>, TerminalFailure> {
        let generation = driver.generation();
        if generation == 0 || maximum_streams == 0 || selection_timeout.is_zero() {
            return Err(TerminalFailure::GenerationMismatch);
        }
        Ok(Arc::new(Self {
            driver,
            generation,
            maximum_streams,
            selection_timeout,
            table: Mutex::new(TerminalTable {
                next_stream: MIN_NAMED_STREAM_ID,
                active: BTreeMap::new(),
            }),
        }))
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn active_count(&self) -> usize {
        self.table
            .lock()
            .map(|table| table.active.len())
            .unwrap_or(0)
    }

    pub async fn reserve(
        self: &Arc<Self>,
        binding: TerminalBinding,
        prepared: Arc<dyn PreparedTerminal>,
        cancellation: Cancellation,
    ) -> Result<String, TerminalFailure> {
        if binding.session_generation != self.generation || binding.request_id == [0; 16] {
            return Err(TerminalFailure::GenerationMismatch);
        }
        if cancellation.is_cancelled() {
            prepared.abandoned().await;
            return Err(TerminalFailure::Unavailable);
        }
        let (stream_number, receiver) = {
            let mut table = self.table.lock().map_err(|_| TerminalFailure::Internal)?;
            if table.active.len() >= self.maximum_streams {
                return Err(TerminalFailure::ResourceExhausted);
            }
            if table.active.values().any(|active| {
                active.binding.session_generation == binding.session_generation
                    && active.binding.request_id == binding.request_id
            }) {
                return Err(TerminalFailure::Conflict);
            }
            let stream_number = allocate_stream_number(&mut table)?;
            let (sender, receiver) = mpsc::channel(TERMINAL_EVENT_QUEUE);
            table.active.insert(
                stream_number,
                ActiveTerminal {
                    binding: binding.clone(),
                    sender,
                    cancellation: cancellation.clone(),
                },
            );
            (stream_number, receiver)
        };
        let lease =
            ServerStreamLease::reserve(stream_number).map_err(|_| TerminalFailure::Internal)?;
        let stream = StreamId::new(stream_number).map_err(|_| TerminalFailure::Internal)?;
        if self
            .driver
            .open_named_stream(stream, TERMINAL_STREAM_CREDIT, TERMINAL_STREAM_CREDIT)
            .await
            .is_err()
        {
            self.remove(stream_number);
            prepared.abandoned().await;
            return Err(TerminalFailure::ResourceExhausted);
        }
        if cancellation.is_cancelled() {
            self.remove(stream_number);
            prepared.abandoned().await;
            let _ = self.driver.reset_named_stream(stream).await;
            return Err(TerminalFailure::Unavailable);
        }
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            run_terminal_stream(
                manager,
                stream_number,
                stream,
                binding,
                prepared,
                receiver,
                cancellation,
            )
            .await;
        });
        Ok(lease.name())
    }

    pub fn cancel(&self, generation: u64, request_id: &[u8]) -> CancelTerminalResult {
        if generation != self.generation {
            return CancelTerminalResult::GenerationMismatch;
        }
        let Ok(table) = self.table.lock() else {
            return CancelTerminalResult::Unknown;
        };
        let Some(active) = table
            .active
            .values()
            .find(|active| active.binding.request_id.as_slice() == request_id)
        else {
            return CancelTerminalResult::Unknown;
        };
        if !active.cancellation.cancel() {
            return CancelTerminalResult::AlreadyTerminal;
        }
        let _ = active.sender.try_send(RoutedTerminalEvent::Cancelled);
        CancelTerminalResult::Signalled
    }

    pub async fn run_router(self: Arc<Self>) -> Result<(), TerminalFailure> {
        loop {
            let event = self
                .driver
                .receive_named_stream()
                .await
                .map_err(|_| TerminalFailure::Unavailable)?;
            let (stream_number, routed) = match event {
                StreamEvent::Data { stream, bytes } => (
                    stream.channel().value(),
                    RoutedTerminalEvent::Data { bytes },
                ),
                StreamEvent::RemoteClosed { stream } => {
                    (stream.channel().value(), RoutedTerminalEvent::RemoteClosed)
                }
                StreamEvent::Reset { stream } => {
                    (stream.channel().value(), RoutedTerminalEvent::Reset)
                }
            };
            let sender = self
                .table
                .lock()
                .map_err(|_| TerminalFailure::Internal)?
                .active
                .get(&stream_number)
                .map(|active| active.sender.clone());
            let Some(sender) = sender else {
                let stream = StreamId::new(stream_number).map_err(|_| TerminalFailure::Protocol)?;
                let _ = self.driver.reset_named_stream(stream).await;
                continue;
            };
            if sender.try_send(routed).is_err() {
                let stream = StreamId::new(stream_number).map_err(|_| TerminalFailure::Protocol)?;
                let _ = self.driver.reset_named_stream(stream).await;
                self.remove(stream_number);
                continue;
            }
        }
    }

    pub async fn shutdown(&self) {
        let active = self
            .table
            .lock()
            .map(|mut table| std::mem::take(&mut table.active))
            .unwrap_or_default();
        for (stream_number, active) in active {
            let _ = active.sender.try_send(RoutedTerminalEvent::SessionClosed);
            if let Ok(stream) = StreamId::new(stream_number) {
                let _ = self.driver.reset_named_stream(stream).await;
            }
        }
    }

    fn remove(&self, stream_number: u16) {
        if let Ok(mut table) = self.table.lock() {
            table.active.remove(&stream_number);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelTerminalResult {
    Signalled,
    AlreadyTerminal,
    GenerationMismatch,
    Unknown,
}

fn allocate_stream_number(table: &mut TerminalTable) -> Result<u16, TerminalFailure> {
    let start = table.next_stream.max(MIN_NAMED_STREAM_ID);
    let mut candidate = start;
    loop {
        if !table.active.contains_key(&candidate) {
            table.next_stream = candidate
                .checked_add(1)
                .filter(|next| *next >= MIN_NAMED_STREAM_ID)
                .unwrap_or(MIN_NAMED_STREAM_ID);
            return Ok(candidate);
        }
        candidate = candidate
            .checked_add(1)
            .filter(|next| *next >= MIN_NAMED_STREAM_ID)
            .unwrap_or(MIN_NAMED_STREAM_ID);
        if candidate == start {
            return Err(TerminalFailure::ResourceExhausted);
        }
    }
}

async fn run_terminal_stream(
    manager: Arc<TerminalSessionManager>,
    stream_number: u16,
    stream: StreamId,
    binding: TerminalBinding,
    prepared: Arc<dyn PreparedTerminal>,
    mut receiver: mpsc::Receiver<RoutedTerminalEvent>,
    cancellation: Cancellation,
) {
    let mut validator = match TerminalStreamValidator::new(
        binding.kind,
        binding.session_generation,
        binding.request_id,
        binding.operation_id.clone(),
        binding.resource_handle.clone(),
    ) {
        Ok(validator) => validator,
        Err(_) => {
            manager.remove(stream_number);
            prepared.abandoned().await;
            return;
        }
    };
    if let Some(range) = binding.retained_log.as_ref()
        && validator.bind_retained_log_range(range).is_err()
    {
        manager.remove(stream_number);
        prepared.abandoned().await;
        let _ = manager.driver.reset_named_stream(stream).await;
        return;
    }
    let first = tokio::select! {
        biased;
        () = cancellation.cancelled() => Some(RoutedTerminalEvent::Cancelled),
        event = tokio::time::timeout(manager.selection_timeout, receiver.recv()) => {
            event.ok().flatten()
        },
    };
    let Some(RoutedTerminalEvent::Data { bytes }) = first else {
        prepared.abandoned().await;
        manager.remove(stream_number);
        let _ = manager.driver.reset_named_stream(stream).await;
        return;
    };
    let first_frame = match terminal::TerminalStreamFrame::parse_from_bytes(&bytes) {
        Ok(frame) => frame,
        Err(_) => {
            prepared.abandoned().await;
            manager.remove(stream_number);
            let _ = manager.driver.reset_named_stream(stream).await;
            return;
        }
    };
    if validator
        .accept(TerminalFrameDirection::ClientToServer, &first_frame)
        .is_err()
    {
        tracing::debug!(
            frame = ?RedactedTerminalFrame(&first_frame),
            "terminal selection frame rejected"
        );
        prepared.abandoned().await;
        manager.remove(stream_number);
        let _ = manager.driver.reset_named_stream(stream).await;
        return;
    }
    if grant_consumed_credit(&manager, stream, &validator, bytes.len())
        .await
        .is_err()
    {
        prepared.abandoned().await;
        manager.remove(stream_number);
        let _ = manager.driver.reset_named_stream(stream).await;
        return;
    }
    let Some(terminal_stream_frame::Frame::Select(selection)) = first_frame.frame else {
        prepared.abandoned().await;
        manager.remove(stream_number);
        let _ = manager.driver.reset_named_stream(stream).await;
        return;
    };
    if cancellation.is_cancelled() {
        prepared.abandoned().await;
        manager.remove(stream_number);
        let _ = manager.driver.reset_named_stream(stream).await;
        return;
    }
    let opened = prepared.open(&binding, selection).await;
    let mut server_sequence = 0_u64;
    let mut owner = match opened {
        Ok(TerminalOpenResult::Active { started, owner }) => {
            if send_server_payload(
                &manager,
                stream,
                &binding,
                &mut validator,
                &mut server_sequence,
                terminal_stream_frame::Frame::Started(started),
            )
            .await
            .is_err()
            {
                let mut owner = owner;
                let _ = owner.finish(TerminalFinish::Disconnect).await;
                manager.remove(stream_number);
                let _ = manager.driver.reset_named_stream(stream).await;
                return;
            }
            owner
        }
        Ok(TerminalOpenResult::ActiveWithoutStarted { owner }) => owner,
        Ok(TerminalOpenResult::Immediate(events)) => {
            let result = send_owner_events(
                &manager,
                stream,
                &binding,
                &mut validator,
                &mut server_sequence,
                events,
            )
            .await;
            manager.remove(stream_number);
            if matches!(result, Ok(StreamAction::Terminal)) {
                let _ = close_after_terminal(&manager, stream, &validator).await;
            } else {
                let _ = manager.driver.reset_named_stream(stream).await;
            }
            return;
        }
        Ok(TerminalOpenResult::Terminal(outcome)) => {
            let result = send_server_payload(
                &manager,
                stream,
                &binding,
                &mut validator,
                &mut server_sequence,
                terminal_stream_frame::Frame::Outcome(outcome),
            )
            .await;
            manager.remove(stream_number);
            if result.is_ok() {
                let _ = close_after_terminal(&manager, stream, &validator).await;
            } else {
                let _ = manager.driver.reset_named_stream(stream).await;
            }
            return;
        }
        Err(error) => {
            let _ = send_failed_outcome(
                &manager,
                stream,
                &binding,
                &mut validator,
                &mut server_sequence,
                error,
            )
            .await;
            manager.remove(stream_number);
            let _ = close_after_terminal(&manager, stream, &validator).await;
            return;
        }
    };

    let mut poll = tokio::time::interval(TERMINAL_POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let action = tokio::select! {
            biased;
            () = cancellation.cancelled() => Ok((StreamAction::Finish(TerminalFinish::Cancel), 0)),
            event = receiver.recv() => match event {
                Some(RoutedTerminalEvent::Data { bytes }) => {
                    let consumed = bytes.len();
                    handle_client_message(
                        &manager,
                        stream,
                        &binding,
                        &mut validator,
                        &mut server_sequence,
                        owner.as_mut(),
                        bytes,
                    ).await.map(|action| (action, consumed))
                }
                Some(RoutedTerminalEvent::Cancelled) => Ok((StreamAction::Finish(TerminalFinish::Cancel), 0)),
                Some(RoutedTerminalEvent::RemoteClosed | RoutedTerminalEvent::Reset | RoutedTerminalEvent::SessionClosed) | None => {
                    Ok((StreamAction::Finish(TerminalFinish::Disconnect), 0))
                }
            },
            _ = poll.tick() => {
                match owner.poll().await {
                    Ok(events) => send_owner_events(
                        &manager,
                        stream,
                        &binding,
                        &mut validator,
                        &mut server_sequence,
                        events,
                    ).await.map(|action| (action, 0)),
                    Err(error) => Err(error),
                }
            }
        };
        let action = match action {
            Ok((action, consumed)) if consumed != 0 => {
                match grant_consumed_credit(&manager, stream, &validator, consumed).await {
                    Ok(()) => Ok(action),
                    Err(error) => Err(error),
                }
            }
            Ok((action, _)) => Ok(action),
            Err(error) => Err(error),
        };
        match action {
            Ok(StreamAction::Continue) => {}
            Ok(StreamAction::Terminal) => {
                manager.remove(stream_number);
                if close_after_terminal(&manager, stream, &validator)
                    .await
                    .is_err()
                {
                    let _ = manager.driver.reset_named_stream(stream).await;
                }
                return;
            }
            Ok(StreamAction::Finish(finish)) => {
                let events = owner.finish(finish).await.unwrap_or_else(|error| {
                    vec![TerminalOwnerEvent::Outcome(failed_outcome(error))]
                });
                if finish != TerminalFinish::Disconnect {
                    let sent = send_owner_events(
                        &manager,
                        stream,
                        &binding,
                        &mut validator,
                        &mut server_sequence,
                        events,
                    )
                    .await;
                    if !matches!(sent, Ok(StreamAction::Terminal))
                        || close_after_terminal(&manager, stream, &validator)
                            .await
                            .is_err()
                    {
                        let _ = manager.driver.reset_named_stream(stream).await;
                    }
                } else {
                    let _ = manager.driver.reset_named_stream(stream).await;
                }
                manager.remove(stream_number);
                return;
            }
            Err(error) => {
                let sent = send_failed_outcome(
                    &manager,
                    stream,
                    &binding,
                    &mut validator,
                    &mut server_sequence,
                    error,
                )
                .await;
                manager.remove(stream_number);
                if sent.is_err()
                    || close_after_terminal(&manager, stream, &validator)
                        .await
                        .is_err()
                {
                    let _ = manager.driver.reset_named_stream(stream).await;
                }
                return;
            }
        }
    }
}

async fn close_after_terminal(
    manager: &TerminalSessionManager,
    stream: StreamId,
    validator: &TerminalStreamValidator,
) -> Result<(), TerminalFailure> {
    validator
        .accept_transport_close()
        .map_err(|_| TerminalFailure::Protocol)?;
    manager
        .driver
        .close_named_stream(stream)
        .await
        .map_err(|_| TerminalFailure::Unavailable)
}

enum StreamAction {
    Continue,
    Finish(TerminalFinish),
    Terminal,
}

async fn grant_consumed_credit(
    manager: &TerminalSessionManager,
    stream: StreamId,
    validator: &TerminalStreamValidator,
    bytes: usize,
) -> Result<(), TerminalFailure> {
    let bytes = u32::try_from(bytes).map_err(|_| TerminalFailure::ResourceExhausted)?;
    validator
        .accept_transport_credit(bytes)
        .map_err(|_| TerminalFailure::Protocol)?;
    manager
        .driver
        .grant_named_stream_credit(stream, bytes)
        .await
        .map_err(|_| TerminalFailure::Unavailable)
}

async fn handle_client_message(
    manager: &TerminalSessionManager,
    stream: StreamId,
    binding: &TerminalBinding,
    validator: &mut TerminalStreamValidator,
    server_sequence: &mut u64,
    owner: &mut dyn TerminalOwner,
    bytes: Vec<u8>,
) -> Result<StreamAction, TerminalFailure> {
    let frame = terminal::TerminalStreamFrame::parse_from_bytes(&bytes)
        .map_err(|_| TerminalFailure::Protocol)?;
    if validator
        .accept(TerminalFrameDirection::ClientToServer, &frame)
        .is_err()
    {
        tracing::debug!(
            frame = ?RedactedTerminalFrame(&frame),
            "terminal client frame rejected"
        );
        return Err(TerminalFailure::Protocol);
    }
    let command = match frame.frame.ok_or(TerminalFailure::Protocol)? {
        terminal_stream_frame::Frame::Stdin(stdin) => TerminalCommand::Stdin {
            offset: stdin.offset,
            data: stdin.data,
            eof: stdin.eof,
        },
        terminal_stream_frame::Frame::Resize(resize) => {
            let size = resize.size.into_option().ok_or(TerminalFailure::Protocol)?;
            TerminalCommand::Resize {
                operation_sequence: resize.operation_sequence,
                rows: size.rows,
                columns: size.columns,
            }
        }
        terminal_stream_frame::Frame::Signal(signal) => TerminalCommand::Signal {
            operation_sequence: signal.operation_sequence,
            signal: signal
                .signal
                .enum_value()
                .map_err(|_| TerminalFailure::Protocol)?,
        },
        terminal_stream_frame::Frame::CloseStdin(_) => TerminalCommand::CloseStdin,
        terminal_stream_frame::Frame::Detach(_) => {
            return Ok(StreamAction::Finish(TerminalFinish::Detach));
        }
        terminal_stream_frame::Frame::Close(_) => {
            return Ok(StreamAction::Finish(TerminalFinish::Close));
        }
        terminal_stream_frame::Frame::Cancel(_) => {
            return Ok(StreamAction::Finish(TerminalFinish::Cancel));
        }
        _ => return Err(TerminalFailure::Protocol),
    };
    let events = owner.command(command).await?;
    send_owner_events(manager, stream, binding, validator, server_sequence, events).await
}

async fn send_owner_events(
    manager: &TerminalSessionManager,
    stream: StreamId,
    binding: &TerminalBinding,
    validator: &mut TerminalStreamValidator,
    server_sequence: &mut u64,
    events: Vec<TerminalOwnerEvent>,
) -> Result<StreamAction, TerminalFailure> {
    for event in events {
        let (payload, terminal) = match event {
            TerminalOwnerEvent::Output {
                stream,
                offset,
                data,
                eof,
                dropped_bytes,
                truncated,
            } => {
                if data.len() > MAX_TERMINAL_CHUNK_BYTES {
                    return Err(TerminalFailure::ResourceExhausted);
                }
                let output = terminal::TerminalOutput {
                    offset,
                    data,
                    eof,
                    dropped_bytes,
                    truncated,
                    ..Default::default()
                };
                (
                    match stream {
                        TerminalOutputStream::Stdout => {
                            terminal_stream_frame::Frame::Stdout(output)
                        }
                        TerminalOutputStream::Stderr => {
                            terminal_stream_frame::Frame::Stderr(output)
                        }
                    },
                    false,
                )
            }
            TerminalOwnerEvent::Status {
                status,
                next_stdin_offset,
            } => (
                terminal_stream_frame::Frame::Status(terminal::TerminalStatus {
                    status: EnumOrUnknown::new(status),
                    next_stdin_offset,
                    ..Default::default()
                }),
                false,
            ),
            TerminalOwnerEvent::Outcome(outcome) => {
                (terminal_stream_frame::Frame::Outcome(outcome), true)
            }
            TerminalOwnerEvent::ShellResult(result) => {
                (terminal_stream_frame::Frame::ShellResult(result), false)
            }
        };
        send_server_payload(
            manager,
            stream,
            binding,
            validator,
            server_sequence,
            payload,
        )
        .await?;
        if terminal {
            return Ok(StreamAction::Terminal);
        }
    }
    Ok(StreamAction::Continue)
}

async fn send_server_payload(
    manager: &TerminalSessionManager,
    stream: StreamId,
    binding: &TerminalBinding,
    validator: &mut TerminalStreamValidator,
    sequence: &mut u64,
    payload: terminal_stream_frame::Frame,
) -> Result<(), TerminalFailure> {
    let frame = terminal::TerminalStreamFrame {
        session_generation: binding.session_generation,
        request_id: binding.request_id.to_vec(),
        sequence: *sequence,
        operation_id: binding.operation_id.clone(),
        resource_handle: binding.resource_handle.clone(),
        frame: Some(payload),
        ..Default::default()
    };
    validator
        .accept(TerminalFrameDirection::ServerToClient, &frame)
        .map_err(|_| TerminalFailure::Protocol)?;
    let encoded = frame
        .write_to_bytes()
        .map_err(|_| TerminalFailure::Protocol)?;
    manager
        .driver
        .send_named_stream(stream, encoded)
        .await
        .map_err(|_| TerminalFailure::Unavailable)?;
    *sequence = sequence
        .checked_add(1)
        .ok_or(TerminalFailure::ResourceExhausted)?;
    Ok(())
}

async fn send_failed_outcome(
    manager: &TerminalSessionManager,
    stream: StreamId,
    binding: &TerminalBinding,
    validator: &mut TerminalStreamValidator,
    sequence: &mut u64,
    failure: TerminalFailure,
) -> Result<(), TerminalFailure> {
    send_server_payload(
        manager,
        stream,
        binding,
        validator,
        sequence,
        terminal_stream_frame::Frame::Outcome(failed_outcome(failure)),
    )
    .await
}

pub fn failed_outcome(failure: TerminalFailure) -> terminal::TerminalOutcome {
    terminal::TerminalOutcome {
        outcome: Some(terminal::terminal_outcome::Outcome::Failed(
            terminal::TerminalFailed {
                error: EnumOrUnknown::new(failure.error_kind()),
                retry: EnumOrUnknown::new(failure.retry()),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

pub fn cancelled_outcome() -> terminal::TerminalOutcome {
    terminal::TerminalOutcome {
        outcome: Some(terminal::terminal_outcome::Outcome::Cancelled(
            terminal::TerminalCancelled::default(),
        )),
        ..Default::default()
    }
}

pub fn closed_outcome() -> terminal::TerminalOutcome {
    terminal::TerminalOutcome {
        outcome: Some(terminal::terminal_outcome::Outcome::Closed(
            terminal::TerminalClosed::default(),
        )),
        ..Default::default()
    }
}

pub fn detached_outcome() -> terminal::TerminalOutcome {
    terminal::TerminalOutcome {
        outcome: Some(terminal::terminal_outcome::Outcome::Detached(
            terminal::TerminalDetached::default(),
        )),
        ..Default::default()
    }
}

pub fn exited_outcome(exit_code: i32) -> terminal::TerminalOutcome {
    terminal::TerminalOutcome {
        outcome: Some(terminal::terminal_outcome::Outcome::Exited(
            terminal::TerminalExited {
                exit_code,
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

pub fn signaled_outcome(signal: u32) -> terminal::TerminalOutcome {
    terminal::TerminalOutcome {
        outcome: Some(terminal::terminal_outcome::Outcome::Signaled(
            terminal::TerminalSignaled {
                signal,
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

pub fn error_envelope(failure: TerminalFailure) -> common::ErrorEnvelope {
    common::ErrorEnvelope {
        kind: EnumOrUnknown::new(failure.common_error_kind()),
        retry: EnumOrUnknown::new(failure.retry()),
        ..Default::default()
    }
}

pub fn new_terminal_resource_handle() -> Result<String, TerminalFailure> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| TerminalFailure::Internal)?;
    let encoded = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("terminal-{encoded}"))
}

pub fn terminal_open_failure_response(
    request: &terminal::TerminalOpenRequest,
    generation: u64,
    failure: TerminalFailure,
) -> terminal::TerminalOpenResponse {
    let request_id = request
        .metadata
        .as_ref()
        .map(|metadata| metadata.request_id.clone())
        .unwrap_or_else(|| vec![1; 16]);
    terminal::TerminalOpenResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_FAILED),
        operation_id: request.operation_id.clone(),
        session_generation: generation,
        request_id,
        error: MessageField::some(error_envelope(failure)),
        ..Default::default()
    }
}

pub fn terminal_open_success_response(
    request: &terminal::TerminalOpenRequest,
    generation: u64,
    stream_id: String,
    resource_handle: String,
) -> Result<terminal::TerminalOpenResponse, TerminalFailure> {
    let metadata = request.metadata.as_ref().ok_or(TerminalFailure::Protocol)?;
    let response = terminal::TerminalOpenResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
        operation_id: request.operation_id.clone(),
        stream_id,
        resource_handle,
        session_generation: generation,
        request_id: metadata.request_id.clone(),
        ..Default::default()
    };
    response
        .validate_wire(false)
        .map_err(|_| TerminalFailure::Protocol)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use d2b_contracts::v2_component_session::{RequestId, SessionErrorCode};
    use d2b_session::{Cancellation, OwnedAttachment, RequestRegistry, SessionError, SessionEvent};

    struct FakeDriver {
        generation: u64,
        fail_open: AtomicBool,
        opened: Mutex<Vec<u16>>,
        reset: Mutex<Vec<u16>>,
        incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<StreamEvent>>,
        sent: mpsc::UnboundedSender<Vec<u8>>,
    }

    impl FakeDriver {
        fn new(
            generation: u64,
        ) -> (
            Arc<Self>,
            mpsc::UnboundedSender<StreamEvent>,
            mpsc::UnboundedReceiver<Vec<u8>>,
        ) {
            let (incoming_tx, incoming) = mpsc::unbounded_channel();
            let (sent, sent_rx) = mpsc::unbounded_channel();
            (
                Arc::new(Self {
                    generation,
                    fail_open: AtomicBool::new(false),
                    opened: Mutex::new(Vec::new()),
                    reset: Mutex::new(Vec::new()),
                    incoming: tokio::sync::Mutex::new(incoming),
                    sent,
                }),
                incoming_tx,
                sent_rx,
            )
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for FakeDriver {
        fn generation(&self) -> u64 {
            self.generation
        }

        async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_ttrpc(&self, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn register_inbound_call(
            &self,
            request_id: RequestId,
        ) -> d2b_session::Result<Cancellation> {
            RequestRegistry::new(self.generation)?.register(request_id)
        }

        async fn complete_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn remove_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
            Ok(Vec::new())
        }

        async fn open_named_stream(
            &self,
            stream: StreamId,
            _: u32,
            _: u32,
        ) -> d2b_session::Result<()> {
            if self.fail_open.load(Ordering::Acquire) {
                return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
            }
            self.opened.lock().unwrap().push(stream.channel().value());
            Ok(())
        }

        async fn send_named_stream(&self, _: StreamId, bytes: Vec<u8>) -> d2b_session::Result<()> {
            self.sent
                .send(bytes)
                .map_err(|_| SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
            self.incoming
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn reset_named_stream(&self, stream: StreamId) -> d2b_session::Result<()> {
            self.reset.lock().unwrap().push(stream.channel().value());
            Ok(())
        }

        async fn drive_keepalive(&self, _: std::time::Instant) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn close(
            &self,
            _: d2b_contracts::v2_component_session::CloseReason,
            _: d2b_contracts::v2_component_session::Remediation,
        ) -> d2b_session::Result<()> {
            Ok(())
        }
    }

    struct FakePrepared {
        abandoned: Arc<AtomicUsize>,
        opened: Arc<AtomicUsize>,
        finished: Arc<Mutex<Vec<TerminalFinish>>>,
        poll: Arc<Mutex<VecDeque<TerminalOwnerEvent>>>,
    }

    #[async_trait]
    impl PreparedTerminal for FakePrepared {
        async fn open(
            &self,
            _: &TerminalBinding,
            _: terminal::TerminalSelection,
        ) -> Result<TerminalOpenResult, TerminalFailure> {
            self.opened.fetch_add(1, Ordering::AcqRel);
            Ok(TerminalOpenResult::Active {
                started: terminal::TerminalStarted {
                    kind: EnumOrUnknown::new(terminal::TerminalKind::TERMINAL_KIND_EXEC),
                    tty: false,
                    ..Default::default()
                },
                owner: Box::new(FakeOwner {
                    finished: Arc::clone(&self.finished),
                    poll: Arc::clone(&self.poll),
                }),
            })
        }

        async fn abandoned(&self) {
            self.abandoned.fetch_add(1, Ordering::AcqRel);
        }
    }

    struct FakeOwner {
        finished: Arc<Mutex<Vec<TerminalFinish>>>,
        poll: Arc<Mutex<VecDeque<TerminalOwnerEvent>>>,
    }

    #[async_trait]
    impl TerminalOwner for FakeOwner {
        async fn command(
            &mut self,
            command: TerminalCommand,
        ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
            match command {
                TerminalCommand::Stdin { offset, data, .. } => {
                    Ok(vec![TerminalOwnerEvent::Status {
                        status: terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED,
                        next_stdin_offset: offset + data.len() as u64,
                    }])
                }
                TerminalCommand::Resize { .. }
                | TerminalCommand::Signal { .. }
                | TerminalCommand::CloseStdin => Ok(vec![TerminalOwnerEvent::Status {
                    status: terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_CONTROL_APPLIED,
                    next_stdin_offset: 0,
                }]),
            }
        }

        async fn poll(&mut self) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
            Ok(self.poll.lock().unwrap().pop_front().into_iter().collect())
        }

        async fn finish(
            &mut self,
            finish: TerminalFinish,
        ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
            self.finished.lock().unwrap().push(finish);
            let outcome = match finish {
                TerminalFinish::Cancel => cancelled_outcome(),
                TerminalFinish::Detach => detached_outcome(),
                TerminalFinish::Close | TerminalFinish::Disconnect => closed_outcome(),
            };
            Ok(vec![TerminalOwnerEvent::Outcome(outcome)])
        }
    }

    fn binding(generation: u64) -> TerminalBinding {
        TerminalBinding {
            session_generation: generation,
            request_id: [4; 16],
            operation_id: "operation-1".to_owned(),
            resource_handle: "terminal-resource".to_owned(),
            peer_principal: "local-admin".to_owned(),
            peer_uid: 1000,
            kind: terminal::TerminalKind::TERMINAL_KIND_EXEC,
            retained_log: None,
        }
    }

    fn binding_with_request(generation: u64, request_byte: u8) -> TerminalBinding {
        TerminalBinding {
            request_id: [request_byte; 16],
            operation_id: format!("operation-{request_byte}"),
            ..binding(generation)
        }
    }

    fn cancellation(generation: u64, request_byte: u8) -> Cancellation {
        RequestRegistry::new(generation)
            .unwrap()
            .register(RequestId::new(vec![request_byte; 16]).unwrap())
            .unwrap()
    }

    fn prepared() -> (
        Arc<FakePrepared>,
        Arc<AtomicUsize>,
        Arc<Mutex<Vec<TerminalFinish>>>,
    ) {
        let abandoned = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(Mutex::new(Vec::new()));
        (
            Arc::new(FakePrepared {
                abandoned: Arc::clone(&abandoned),
                opened: Arc::new(AtomicUsize::new(0)),
                finished: Arc::clone(&finished),
                poll: Arc::new(Mutex::new(VecDeque::new())),
            }),
            abandoned,
            finished,
        )
    }

    fn selection_frame(generation: u64) -> terminal::TerminalStreamFrame {
        let exec = terminal::ExecSelection {
            authority: EnumOrUnknown::new(terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY),
            selection: Some(terminal::exec_selection::Selection::Arbitrary(
                terminal::ArbitraryExecSelection {
                    argv: vec![b"true".to_vec()],
                    ..Default::default()
                },
            )),
            tty: false,
            detached: false,
            ..Default::default()
        };
        terminal::TerminalStreamFrame {
            session_generation: generation,
            request_id: vec![4; 16],
            sequence: 0,
            operation_id: "operation-1".to_owned(),
            resource_handle: "terminal-resource".to_owned(),
            frame: Some(terminal_stream_frame::Frame::Select(
                terminal::TerminalSelection {
                    selection: Some(terminal::terminal_selection::Selection::Exec(exec)),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    fn stdin_frame(generation: u64) -> terminal::TerminalStreamFrame {
        terminal::TerminalStreamFrame {
            session_generation: generation,
            request_id: vec![4; 16],
            sequence: 1,
            operation_id: "operation-1".to_owned(),
            resource_handle: "terminal-resource".to_owned(),
            frame: Some(terminal_stream_frame::Frame::Stdin(
                terminal::TerminalStdin {
                    offset: 0,
                    data: b"input".to_vec(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    async fn next_frame(
        sent: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> terminal::TerminalStreamFrame {
        let bytes = tokio::time::timeout(Duration::from_secs(1), sent.recv())
            .await
            .expect("server frame timeout")
            .expect("server frame channel");
        terminal::TerminalStreamFrame::parse_from_bytes(&bytes).expect("parse server frame")
    }

    #[tokio::test]
    async fn stream_ids_are_allocated_only_by_the_server() {
        let (driver, _, _) = FakeDriver::new(41);
        let manager = TerminalSessionManager::new(driver.clone(), 4).unwrap();
        let (prepared, _, _) = prepared();
        let first = manager
            .reserve(binding(41), prepared.clone(), cancellation(41, 4))
            .await
            .unwrap();
        let second = manager
            .reserve(binding_with_request(41, 5), prepared, cancellation(41, 5))
            .await
            .unwrap();
        assert_eq!(first, "stream-256");
        assert_eq!(second, "stream-257");
        assert_eq!(*driver.opened.lock().unwrap(), [256, 257]);
    }

    #[tokio::test]
    async fn stream_count_is_bounded_before_driver_allocation() {
        let (driver, _, _) = FakeDriver::new(40);
        let manager = TerminalSessionManager::new(driver.clone(), 1).unwrap();
        let (prepared, _, _) = prepared();
        manager
            .reserve(binding(40), prepared.clone(), cancellation(40, 4))
            .await
            .unwrap();
        assert_eq!(
            manager
                .reserve(binding(40), prepared, cancellation(40, 4))
                .await
                .unwrap_err(),
            TerminalFailure::ResourceExhausted
        );
        assert_eq!(driver.opened.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn duplicate_request_binding_is_rejected_before_driver_allocation() {
        let (driver, _, _) = FakeDriver::new(39);
        let manager = TerminalSessionManager::new(driver.clone(), 2).unwrap();
        let (prepared, _, _) = prepared();
        manager
            .reserve(binding(39), prepared.clone(), cancellation(39, 4))
            .await
            .unwrap();
        assert_eq!(
            manager
                .reserve(binding(39), prepared, cancellation(39, 4))
                .await
                .unwrap_err(),
            TerminalFailure::Conflict
        );
        assert_eq!(driver.opened.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn open_failure_returns_no_dangling_stream() {
        let (driver, _, _) = FakeDriver::new(42);
        driver.fail_open.store(true, Ordering::Release);
        let manager = TerminalSessionManager::new(driver, 4).unwrap();
        let (prepared, abandoned, _) = prepared();
        assert_eq!(
            manager
                .reserve(binding(42), prepared, cancellation(42, 4))
                .await
                .unwrap_err(),
            TerminalFailure::ResourceExhausted
        );
        assert_eq!(manager.active_count(), 0);
        assert_eq!(abandoned.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn shared_cancellation_closes_reservation_during_unary_handoff() {
        let (driver, _, _) = FakeDriver::new(48);
        let manager = TerminalSessionManager::new(driver, 4).unwrap();
        let (prepared, abandoned, _) = prepared();
        let cancellation = cancellation(48, 4);
        manager
            .reserve(binding(48), prepared.clone(), cancellation.clone())
            .await
            .unwrap();

        assert_eq!(
            manager.cancel(48, &[4; 16]),
            CancelTerminalResult::Signalled
        );
        assert!(cancellation.is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.active_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(abandoned.load(Ordering::Acquire), 1);
        assert_eq!(prepared.opened.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn selection_timeout_abandons_without_owner_effects() {
        let (driver, _, _) = FakeDriver::new(49);
        let manager = TerminalSessionManager::with_selection_timeout(
            driver.clone(),
            4,
            Duration::from_millis(10),
        )
        .unwrap();
        let (prepared, abandoned, _) = prepared();
        manager
            .reserve(binding(49), prepared.clone(), cancellation(49, 4))
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.active_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(prepared.opened.load(Ordering::Acquire), 0);
        assert_eq!(abandoned.load(Ordering::Acquire), 1);
        assert_eq!(driver.reset.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn real_bridge_runs_bidirectional_lifecycle_and_cancel_teardown() {
        let (driver, incoming, mut sent) = FakeDriver::new(43);
        let manager = TerminalSessionManager::new(driver, 4).unwrap();
        let (prepared, _, finished) = prepared();
        let stream_name = manager
            .reserve(binding(43), prepared, cancellation(43, 4))
            .await
            .unwrap();
        let stream = StreamId::new(
            d2b_contracts::v2_services::parse_server_stream_name(&stream_name).unwrap(),
        )
        .unwrap();
        let router = tokio::spawn(Arc::clone(&manager).run_router());

        incoming
            .send(StreamEvent::Data {
                stream,
                bytes: selection_frame(43).write_to_bytes().unwrap(),
            })
            .unwrap();
        assert!(next_frame(&mut sent).await.has_started());

        incoming
            .send(StreamEvent::Data {
                stream,
                bytes: stdin_frame(43).write_to_bytes().unwrap(),
            })
            .unwrap();
        let status = next_frame(&mut sent).await;
        assert_eq!(
            status.status().status.enum_value().unwrap(),
            terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED
        );

        assert_eq!(
            manager.cancel(43, &[4; 16]),
            CancelTerminalResult::Signalled
        );
        let outcome = next_frame(&mut sent).await;
        assert!(matches!(
            outcome.outcome().outcome,
            Some(terminal::terminal_outcome::Outcome::Cancelled(_))
        ));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if manager.active_count() == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(*finished.lock().unwrap(), [TerminalFinish::Cancel]);
        router.abort();
    }

    #[tokio::test]
    async fn generation_mismatch_resets_before_owner_setup() {
        let (driver, incoming, _sent) = FakeDriver::new(44);
        let manager = TerminalSessionManager::new(driver.clone(), 4).unwrap();
        let (prepared, abandoned, finished) = prepared();
        let stream_name = manager
            .reserve(binding(44), prepared, cancellation(44, 4))
            .await
            .unwrap();
        let stream = StreamId::new(
            d2b_contracts::v2_services::parse_server_stream_name(&stream_name).unwrap(),
        )
        .unwrap();
        let router = tokio::spawn(Arc::clone(&manager).run_router());
        incoming
            .send(StreamEvent::Data {
                stream,
                bytes: selection_frame(45).write_to_bytes().unwrap(),
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !driver.reset.lock().unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(abandoned.load(Ordering::Acquire), 1);
        assert!(finished.lock().unwrap().is_empty());
        router.abort();
    }

    #[tokio::test]
    async fn remote_disconnect_tears_down_the_exact_owner() {
        let (driver, incoming, mut sent) = FakeDriver::new(46);
        let manager = TerminalSessionManager::new(driver, 4).unwrap();
        let (prepared, _, finished) = prepared();
        let stream_name = manager
            .reserve(binding(46), prepared, cancellation(46, 4))
            .await
            .unwrap();
        let stream = StreamId::new(
            d2b_contracts::v2_services::parse_server_stream_name(&stream_name).unwrap(),
        )
        .unwrap();
        let router = tokio::spawn(Arc::clone(&manager).run_router());
        incoming
            .send(StreamEvent::Data {
                stream,
                bytes: selection_frame(46).write_to_bytes().unwrap(),
            })
            .unwrap();
        assert!(next_frame(&mut sent).await.has_started());
        incoming.send(StreamEvent::RemoteClosed { stream }).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !finished.lock().unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(*finished.lock().unwrap(), [TerminalFinish::Disconnect]);
        router.abort();
    }

    #[tokio::test]
    async fn oversized_backend_output_fails_with_one_typed_outcome() {
        let (driver, incoming, mut sent) = FakeDriver::new(47);
        let manager = TerminalSessionManager::new(driver, 4).unwrap();
        let abandoned = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(Mutex::new(Vec::new()));
        let poll = Arc::new(Mutex::new(VecDeque::from([TerminalOwnerEvent::Output {
            stream: TerminalOutputStream::Stdout,
            offset: 0,
            data: vec![0; MAX_TERMINAL_CHUNK_BYTES + 1],
            eof: false,
            dropped_bytes: 0,
            truncated: false,
        }])));
        let prepared = Arc::new(FakePrepared {
            abandoned,
            opened: Arc::new(AtomicUsize::new(0)),
            finished,
            poll,
        });
        let stream_name = manager
            .reserve(binding(47), prepared, cancellation(47, 4))
            .await
            .unwrap();
        let stream = StreamId::new(
            d2b_contracts::v2_services::parse_server_stream_name(&stream_name).unwrap(),
        )
        .unwrap();
        let router = tokio::spawn(Arc::clone(&manager).run_router());
        incoming
            .send(StreamEvent::Data {
                stream,
                bytes: selection_frame(47).write_to_bytes().unwrap(),
            })
            .unwrap();
        assert!(next_frame(&mut sent).await.has_started());
        let outcome = next_frame(&mut sent).await;
        let Some(terminal::terminal_outcome::Outcome::Failed(failed)) =
            outcome.outcome().outcome.as_ref()
        else {
            panic!("expected failed terminal outcome");
        };
        assert_eq!(
            failed.error.enum_value().unwrap(),
            terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_RESOURCE_EXHAUSTED
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), sent.recv())
                .await
                .is_err()
        );
        router.abort();
    }

    #[test]
    fn redacted_diagnostics_hide_terminal_bytes_and_bindings() {
        let frame = terminal::TerminalStreamFrame {
            session_generation: 55,
            request_id: vec![9; 16],
            sequence: 1,
            frame: Some(terminal_stream_frame::Frame::Stdin(
                terminal::TerminalStdin {
                    offset: 0,
                    data: b"sensitive-terminal-bytes".to_vec(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let rendered = format!("{:?}", RedactedTerminalFrame(&frame));
        assert!(!rendered.contains("sensitive-terminal-bytes"));
        assert!(!rendered.contains("55"));
        assert!(!rendered.contains("[9"));
    }

    #[test]
    fn typed_open_failure_never_advertises_a_stream() {
        let request = terminal::TerminalOpenRequest {
            metadata: MessageField::some(common::RequestMetadata {
                request_id: vec![3; 16],
                correlation_id: "correlation-1".to_owned(),
                trace_id: vec![4; 16],
                idempotency_key: vec![5; 16],
                issued_at_unix_ms: 1,
                expires_at_unix_ms: 2,
                session_generation: 48,
                ..Default::default()
            }),
            scope: MessageField::some(common::IdentityScope {
                realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                ..Default::default()
            }),
            resource_id: "workload-1".to_owned(),
            operation_id: "operation-1".to_owned(),
            request_digest: vec![6; 32],
            ..Default::default()
        };
        let response = terminal_open_failure_response(&request, 48, TerminalFailure::Unavailable);
        d2b_contracts::v2_services::validate_terminal_open_response_for_request(
            &request, &response,
        )
        .expect("strict typed failure");
        assert!(response.stream_id.is_empty());
        assert!(response.error.is_some());
    }
}
