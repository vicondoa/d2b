//! In-process exec session table + per-session worker for `d2b vm exec`.
//!
//! The daemon owns a long-lived, authenticated Process named-stream client per
//! exec session. The CLI establishes the resource owner through the
//! admin-gated `public.sock` route, then sends one correlated named-stream
//! frame per [`ExecOp`]. A dedicated worker thread (current-thread tokio
//! runtime) owns the authenticated client, the target-local process resource,
//! the authoritative stdin offset, and the monotone control sequence; it is
//! reached over a bounded sync command channel.
//!
//! Concurrency contract (no head-of-line blocking): long-poll ops
//! (`ReadOutput`, `Wait`) are spawned onto the worker runtime so the worker
//! keeps servicing fast control ops (`WriteStdin`, `Signal`, `Resize`,
//! `Close`) while a poll is in flight. Fast ops are handled inline because
//! they mutate shared session state (stdin offset, control sequence).
//!
//! Teardown contract (non-detached): when the owner connection drops, the
//! command channel closes, the worker explicitly cancels/resets the named
//! stream and target-local process before returning. The runtime is then
//! dropped, so no attachment or process remains owned by a disconnected
//! caller.

use std::sync::{
    Arc, Mutex,
    atomic::AtomicU64,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Weak,
};

use async_trait::async_trait;

use d2b_contracts_control::guest_proto as pb;
use d2b_contracts_control::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
use d2b_contracts_control::public_wire::{
    EXEC_MAX_CHUNK_BYTES, ExecCloseResult, ExecControlResult, ExecOp, ExecOpResponse,
    ExecReadOutputResult, ExecStartResult, ExecStream, ExecTerminalStatus, ExecWaitResult,
    ExecWriteStdinResult, NamedProcessStreamErrorKind, NamedProcessStreamRequest,
    NamedProcessStreamRequestFrame, NamedProcessStreamResponse, NamedProcessStreamResponseFrame,
};
use d2b_core::base64_codec;
use d2b_session::{ComponentSessionDriver, StreamEvent, StreamId};
use protobuf::{EnumOrUnknown, MessageField};
use tokio::sync::{mpsc, oneshot};

use crate::guest_control_health::GuestControlHealthError;
use crate::terminal_session::{OutputStreamSel, TerminalBackend, TerminalKind};
use crate::terminal_session::{ReadOutputOutcome, WaitOutcome, WriteStdinOutcome};

/// Closed enum of per-op proxy failures. Each maps to a redaction-safe slug;
/// the daemon never attaches argv, env, output bytes, or a guest-supplied
/// string to the failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecOpError {
    Transport,
    Auth,
    StaleSession,
    Protocol,
    Timeout,
    OldGeneration,
    Capability,
    DetachedUnavailable,
    /// Guest-reported deterministic op error (a closed slug).
    Guest(GuestOpError),
}

/// Closed enum of deterministic guest-reported op errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestOpError {
    OffsetMismatch,
    StdinClosed,
    StdinNotOpen,
    StdinBackpressure,
    ExecNotFound,
    ExecAlreadyExited,
    ExecExpired,
    ControlSeqMismatch,
    RateLimited,
    MaxChunkExceeded,
    InvalidProgram,
    Protocol,
    Other,
}

impl GuestOpError {
    pub fn slug(self) -> &'static str {
        match self {
            Self::OffsetMismatch => "stdin-offset-mismatch",
            Self::StdinClosed => "stdin-closed",
            Self::StdinNotOpen => "stdin-not-open",
            Self::StdinBackpressure => "stdin-backpressure",
            Self::ExecNotFound => "exec-not-found",
            Self::ExecAlreadyExited => "exec-already-exited",
            Self::ExecExpired => "exec-expired",
            Self::ControlSeqMismatch => "control-seq-mismatch",
            Self::RateLimited => "rate-limited",
            Self::MaxChunkExceeded => "max-chunk-exceeded",
            Self::InvalidProgram => "invalid-program",
            Self::Protocol => "guest-protocol-error",
            Self::Other => "guest-error",
        }
    }
}

impl ExecOpError {
    /// Redaction-safe slug for the public error envelope and audit fields.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Transport => "guest-control-transport-unavailable",
            Self::Auth => "guest-control-auth-failed",
            Self::StaleSession => "stale-session",
            Self::Protocol => "guest-control-protocol-error",
            Self::Timeout => "guest-control-timeout",
            Self::OldGeneration => "guest-control-unavailable-old-generation",
            Self::Capability => "guest-control-capability-unavailable",
            Self::DetachedUnavailable => "guest-control-exec-detached-unavailable",
            Self::Guest(inner) => inner.slug(),
        }
    }

    /// Closed-enum `error_kind` metric label (hard allowlist).
    pub fn metric_kind(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Auth => "auth",
            Self::StaleSession => "auth",
            Self::Protocol => "protocol",
            Self::Timeout => "timeout",
            Self::OldGeneration => "old-generation",
            Self::Capability => "capability",
            Self::DetachedUnavailable => "capability",
            Self::Guest(_) => "guest",
        }
    }
}

/// Closed enum of session-establishment failures (connect + auth + cap-gate +
/// `ExecCreate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecEstablishError {
    Transport,
    Auth,
    Protocol,
    Timeout,
    OldGeneration,
    Capability,
    /// Guest accepted the handshake but rejected the create (e.g. exec
    /// disabled, root denied, unsupported mode).
    Guest(GuestOpError),
}

impl ExecEstablishError {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Transport => "guest-control-transport-unavailable",
            Self::Auth => "guest-control-auth-failed",
            Self::Protocol => "guest-control-protocol-error",
            Self::Timeout => "guest-control-timeout",
            Self::OldGeneration => "guest-control-unavailable-old-generation",
            Self::Capability => "guest-control-capability-unavailable",
            Self::Guest(inner) => inner.slug(),
        }
    }

    pub fn metric_kind(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Auth => "auth",
            Self::Protocol => "protocol",
            Self::Timeout => "timeout",
            Self::OldGeneration => "old-generation",
            Self::Capability => "capability",
            Self::Guest(_) => "guest",
        }
    }
}

/// Per-op absolute deadlines. Each op draws a FRESH deadline: the
/// one-shot establishment budget is exhausted by the time the first op runs,
/// so reusing it would immediately time out.
#[derive(Debug, Clone, Copy)]
pub struct ExecOpDeadlines {
    /// Fast control ops (`WriteStdin`, `Signal`, `Resize`, `Close`).
    pub control: Duration,
    /// Upper bound on a single long-poll (`ReadOutput`, `Wait`) op; the guest
    /// `timeout_ms` is clamped to this so a malicious client cannot pin the
    /// worker indefinitely.
    pub poll_cap: Duration,
    /// Slack added to a long-poll's transport deadline above the guest
    /// `timeout_ms` so the guest's own bounded wait fires first.
    pub poll_slack: Duration,
}

impl Default for ExecOpDeadlines {
    fn default() -> Self {
        Self {
            control: Duration::from_secs(5),
            poll_cap: Duration::from_secs(30),
            poll_slack: Duration::from_secs(2),
        }
    }
}

/// Establishment spec resolved from a validated [`ExecOp::Start`]. `Debug` is
/// redacted so a stray `{:?}` can never leak argv / env keys+values / cwd.
#[derive(Clone, PartialEq, Eq)]
pub struct ExecStartSpec {
    pub vm: String,
    /// Optional opaque idempotency key forwarded as guest request metadata.
    /// It is never argv and must not appear in Debug output.
    pub request_id: Option<String>,
    pub argv: Vec<String>,
    pub tty: bool,
    pub detached: bool,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
    pub term_size: Option<(u32, u32)>,
}

impl std::fmt::Debug for ExecStartSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecStartSpec")
            .field("vm", &self.vm)
            .field("has_request_id", &self.request_id.is_some())
            .field("tty", &self.tty)
            .field("detached", &self.detached)
            .field("argv_len", &self.argv.len())
            .field("env_len", &self.env.len())
            .field("has_cwd", &self.cwd.is_some())
            .field("term_size", &self.term_size)
            .finish()
    }
}

/// Session info reported back to the owner on a successful establish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecSessionInfo {
    pub tty: bool,
    pub stdout_offset: u64,
    pub stderr_offset: u64,
}

/// Negotiated per-session capability + shape snapshot, cached at establish so
/// each proxied op can be gated fail-closed BEFORE it reaches the guest.
/// A guest that did not advertise the cap an op needs (or a non-tty
/// session asked to resize) is rejected with a typed redacted `Capability`
/// error instead of silently proxying the op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegotiatedCaps {
    /// The session was created with a PTY (`-it`). Required for `Resize`.
    pub tty: bool,
    /// Guest advertised `Signals`. Required for `Signal`.
    pub signals: bool,
    /// Guest advertised `TtyResize`. Required (with `tty`) for `Resize`.
    pub tty_resize: bool,
    /// Guest advertised `ExecLogs` (the output cap). Required for `ReadOutput`.
    pub output: bool,
}

impl NegotiatedCaps {
    /// All capabilities present - used by tests that exercise the happy path.
    #[cfg(test)]
    pub fn all() -> Self {
        Self {
            tty: true,
            signals: true,
            tty_resize: true,
            output: true,
        }
    }
}

/// A freshly established session: the authenticated client, the info echoed to
/// the owner, and the initial control sequence from `ExecCreate`.
pub struct Established {
    pub client: Arc<dyn ExecGuestClient>,
    pub info: ExecSessionInfo,
    pub control_seq: u64,
    pub caps: NegotiatedCaps,
}

/// Exec adapter over the shared terminal backend seam. Production implementation
/// wraps a ttRPC client; tests inject a fake.
#[async_trait]
pub trait ExecGuestClient: TerminalBackend<Error = ExecOpError> {}

impl<T> ExecGuestClient for T where T: TerminalBackend<Error = ExecOpError> {}

/// ComponentSession named-stream implementation of the terminal backend.
///
/// The stream is admitted by the authenticated session before this value is
/// constructed. Resource identity and caller authorization therefore stay in
/// the Resource API/session layers; this adapter only moves bounded typed
/// control messages and returns redacted protocol failures.
const MAX_PENDING_NAMED_RESPONSES: usize = 64;

pub struct ComponentSessionExecClient<D> {
    inner: Arc<ComponentSessionExecClientInner<D>>,
}

struct ComponentSessionExecClientInner<D> {
    driver: Arc<D>,
    stream: StreamId,
    closed: AtomicBool,
    next_request_id: AtomicU64,
    waiters: Mutex<BTreeMap<u64, oneshot::Sender<Result<NamedProcessStreamResponse, ExecOpError>>>>,
    late_responses: Mutex<BTreeMap<u64, NamedProcessStreamResponse>>,
    pending_credit: Mutex<u32>,
    send_lock: tokio::sync::Mutex<()>,
    demux_abort: Mutex<Option<tokio::task::AbortHandle>>,
    reset_sent: AtomicBool,
}

impl<D> std::fmt::Debug for ComponentSessionExecClient<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComponentSessionExecClient")
            .field("stream", &"<redacted>")
            .field("closed", &self.inner.closed.load(Ordering::Acquire))
            .finish()
    }
}

impl<D> ComponentSessionExecClient<D>
where
    D: ComponentSessionDriver + 'static,
{
    /// Open the fixed process named stream on an authenticated session.
    pub async fn open(
        driver: D,
        stream_number: u16,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<Self, ExecOpError> {
        let stream = StreamId::new(stream_number).map_err(|_| ExecOpError::Protocol)?;
        driver
            .open_named_stream(stream, send_credit, receive_credit)
            .await
            .map_err(|_| ExecOpError::Transport)?;
        let inner = Arc::new(ComponentSessionExecClientInner {
            driver: Arc::new(driver),
            stream,
            closed: AtomicBool::new(false),
            next_request_id: AtomicU64::new(1),
            waiters: Mutex::new(BTreeMap::new()),
            late_responses: Mutex::new(BTreeMap::new()),
            pending_credit: Mutex::new(0),
            send_lock: tokio::sync::Mutex::new(()),
            demux_abort: Mutex::new(None),
            reset_sent: AtomicBool::new(false),
        });
        let task = tokio::spawn(named_stream_demux(Arc::downgrade(&inner)));
        *inner
            .demux_abort
            .lock()
            .map_err(|_| ExecOpError::Protocol)? = Some(task.abort_handle());
        Ok(Self { inner })
    }

    /// Cancel/reset the named stream after owner disconnect or timeout.
    pub async fn cancel(&self) -> Result<(), ExecOpError> {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.stop_demux();
        self.inner.fail_waiters(ExecOpError::Transport);
        self.inner
            .late_responses
            .lock()
            .map_err(|_| ExecOpError::Protocol)?
            .clear();
        *self
            .inner
            .pending_credit
            .lock()
            .map_err(|_| ExecOpError::Protocol)? = 0;
        if self.inner.reset_sent.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.inner
            .driver
            .reset_named_stream(self.inner.stream)
            .await
            .map_err(|_| ExecOpError::Transport)
    }

    /// Acknowledge that the last received response reached its downstream
    /// consumer and release its ComponentSession receive credit.
    pub async fn acknowledge_received(&self) -> Result<(), ExecOpError> {
        self.flush_pending_credit().await
    }

    /// Close the named stream after a clean owner detach.
    pub async fn close_stream(&self) -> Result<(), ExecOpError> {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.inner.stop_demux();
        self.inner.fail_waiters(ExecOpError::Transport);
        self.flush_pending_credit().await?;
        self.inner
            .driver
            .close_named_stream(self.inner.stream)
            .await
            .map_err(|_| ExecOpError::Transport)?;
        self.inner
            .late_responses
            .lock()
            .map_err(|_| ExecOpError::Protocol)?
            .clear();
        Ok(())
    }

    async fn request(
        &self,
        request: NamedProcessStreamRequest,
        timeout: Duration,
    ) -> Result<NamedProcessStreamResponse, ExecOpError> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(ExecOpError::StaleSession);
        }
        self.flush_pending_credit().await?;
        let request_id = self.inner.next_request_id.fetch_add(1, Ordering::AcqRel);
        if request_id == 0 {
            return Err(ExecOpError::Protocol);
        }
        let frame = serde_json::to_vec(&NamedProcessStreamRequestFrame::new(request_id, request))
            .map_err(|_| ExecOpError::Protocol)?;
        if frame.len()
            > d2b_contracts_zone_session::v3::component_session::MAX_LOGICAL_MESSAGE_BYTES as usize
        {
            return Err(ExecOpError::Protocol);
        }
        let (reply, receive) = oneshot::channel();
        if let Some(response) = self
            .inner
            .late_responses
            .lock()
            .map_err(|_| ExecOpError::Protocol)?
            .remove(&request_id)
        {
            return Ok(response);
        }
        {
            let mut waiters = self
                .inner
                .waiters
                .lock()
                .map_err(|_| ExecOpError::Protocol)?;
            if waiters.len() >= MAX_PENDING_NAMED_RESPONSES {
                return Err(ExecOpError::Guest(GuestOpError::StdinBackpressure));
            }
            waiters.insert(request_id, reply);
        }
        let send_result = {
            let _send_lock = self.inner.send_lock.lock().await;
            self.inner
                .driver
                .send_named_stream(self.inner.stream, frame)
                .await
        };
        if send_result.is_err() {
            self.inner
                .waiters
                .lock()
                .map_err(|_| ExecOpError::Protocol)?
                .remove(&request_id);
            return Err(ExecOpError::Transport);
        }
        match tokio::time::timeout(timeout, receive).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => Err(ExecOpError::Transport),
            Err(_) => {
                self.inner
                    .waiters
                    .lock()
                    .map_err(|_| ExecOpError::Protocol)?
                    .remove(&request_id);
                Err(ExecOpError::Timeout)
            }
        }
    }

    async fn flush_pending_credit(&self) -> Result<(), ExecOpError> {
        let credit = {
            let mut pending = self
                .inner
                .pending_credit
                .lock()
                .map_err(|_| ExecOpError::Protocol)?;
            std::mem::take(&mut *pending)
        };
        if credit == 0 {
            return Ok(());
        }
        self.inner
            .driver
            .grant_named_stream_credit(self.inner.stream, credit)
            .await
            .map_err(|_| ExecOpError::Transport)
    }

    fn map_error(kind: NamedProcessStreamErrorKind) -> ExecOpError {
        match kind {
            NamedProcessStreamErrorKind::Authorization => ExecOpError::Auth,
            NamedProcessStreamErrorKind::StaleSession => ExecOpError::StaleSession,
            NamedProcessStreamErrorKind::NotFound => ExecOpError::Guest(GuestOpError::ExecNotFound),
            NamedProcessStreamErrorKind::Backpressure => {
                ExecOpError::Guest(GuestOpError::StdinBackpressure)
            }
            NamedProcessStreamErrorKind::Protocol => ExecOpError::Protocol,
            NamedProcessStreamErrorKind::Timeout => ExecOpError::Timeout,
            NamedProcessStreamErrorKind::Disconnected => ExecOpError::Transport,
        }
    }

    fn response_error(
        response: NamedProcessStreamResponse,
    ) -> Result<NamedProcessStreamResponse, ExecOpError> {
        match response {
            NamedProcessStreamResponse::Error(error) => Err(Self::map_error(error.kind)),
            response => Ok(response),
        }
    }

    fn terminal_kind(status: &ExecTerminalStatus) -> Option<TerminalKind> {
        match status {
            ExecTerminalStatus::Exited { code } => Some(TerminalKind::Exited(*code)),
            ExecTerminalStatus::Signaled { signal } => Some(TerminalKind::Signaled(*signal)),
            ExecTerminalStatus::Error { .. } => Some(TerminalKind::Error("process-stream-error")),
        }
    }
}

impl<D> Drop for ComponentSessionExecClient<D> {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.stop_demux();
        self.inner.fail_waiters(ExecOpError::Transport);
    }
}

impl<D> ComponentSessionExecClientInner<D> {
    fn stop_demux(&self) {
        if let Ok(mut abort) = self.demux_abort.lock()
            && let Some(abort) = abort.take()
        {
            abort.abort();
        }
    }

    fn fail_waiters(&self, error: ExecOpError) {
        if let Ok(mut waiters) = self.waiters.lock() {
            for (_, waiter) in std::mem::take(&mut *waiters) {
                let _ = waiter.send(Err(error));
            }
        }
    }

    fn fail_closed(&self, error: ExecOpError) {
        self.closed.store(true, Ordering::Release);
        self.fail_waiters(error);
    }
}

async fn named_stream_demux<D>(weak: Weak<ComponentSessionExecClientInner<D>>)
where
    D: ComponentSessionDriver + 'static,
{
    loop {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let event = inner.driver.receive_named_stream().await;
        match event {
            Ok(StreamEvent::Data { stream, bytes }) if stream == inner.stream => {
                let frame: NamedProcessStreamResponseFrame = match serde_json::from_slice(&bytes) {
                    Ok(frame) => frame,
                    Err(_) => {
                        inner.fail_closed(ExecOpError::Protocol);
                        return;
                    }
                };
                if frame.request_id == 0 {
                    inner.fail_closed(ExecOpError::Protocol);
                    return;
                }
                let credit = match u32::try_from(bytes.len()) {
                    Ok(credit) => credit,
                    Err(_) => {
                        inner.fail_closed(ExecOpError::Protocol);
                        return;
                    }
                };
                let credit_result = match inner.pending_credit.lock() {
                    Ok(mut pending) => match pending.checked_add(credit) {
                        Some(next) => {
                            *pending = next;
                            Ok(())
                        }
                        None => Err(ExecOpError::Protocol),
                    },
                    Err(_) => Err(ExecOpError::Protocol),
                };
                match credit_result {
                    Ok(()) => {}
                    Err(error) => {
                        inner.fail_closed(error);
                        return;
                    }
                }
                let waiter = inner
                    .waiters
                    .lock()
                    .ok()
                    .and_then(|mut waiters| waiters.remove(&frame.request_id));
                if let Some(waiter) = waiter {
                    let _ = waiter.send(Ok(frame.response));
                    continue;
                }
                let mut late = match inner.late_responses.lock() {
                    Ok(late) => late,
                    Err(_) => {
                        inner.fail_closed(ExecOpError::Protocol);
                        return;
                    }
                };
                if late.len() >= MAX_PENDING_NAMED_RESPONSES {
                    inner.fail_closed(ExecOpError::Guest(GuestOpError::StdinBackpressure));
                    return;
                }
                if late.contains_key(&frame.request_id) {
                    inner.fail_closed(ExecOpError::Protocol);
                    return;
                }
                late.insert(frame.request_id, frame.response);
            }
            Ok(StreamEvent::RemoteClosed { stream } | StreamEvent::Reset { stream })
                if stream == inner.stream =>
            {
                inner.fail_closed(ExecOpError::Transport);
                return;
            }
            Ok(_) => {}
            Err(_) => {
                inner.fail_closed(ExecOpError::Transport);
                return;
            }
        }
    }
}

#[async_trait]
impl<D> TerminalBackend for ComponentSessionExecClient<D>
where
    D: ComponentSessionDriver + 'static,
{
    type Error = ExecOpError;

    async fn write_stdin(
        &self,
        offset: u64,
        data: Vec<u8>,
        eof: bool,
        timeout: Duration,
    ) -> Result<WriteStdinOutcome, Self::Error> {
        let data_len = u64::try_from(data.len()).map_err(|_| ExecOpError::Protocol)?;
        if data.is_empty() || data_len > EXEC_MAX_CHUNK_BYTES {
            return Err(ExecOpError::Guest(GuestOpError::MaxChunkExceeded));
        }
        let response = Self::response_error(
            self.request(
                NamedProcessStreamRequest::Stdin {
                    offset,
                    chunk_base64: base64_codec::encode(&data),
                    eof,
                },
                timeout,
            )
            .await?,
        )?;
        match response {
            NamedProcessStreamResponse::Stdin(result) => {
                if result.accepted_len > data_len
                    || result.next_offset < offset
                    || result.next_offset - offset != result.accepted_len
                {
                    return Err(ExecOpError::Protocol);
                }
                Ok(WriteStdinOutcome {
                    accepted_len: result.accepted_len,
                    next_offset: result.next_offset,
                    backpressured: result.backpressured,
                    stdin_closed: result.stdin_closed,
                })
            }
            _ => Err(ExecOpError::Protocol),
        }
    }

    async fn read_output(
        &self,
        stream: OutputStreamSel,
        offset: u64,
        max_len: u64,
        wait: bool,
        timeout_ms: u64,
        timeout: Duration,
    ) -> Result<ReadOutputOutcome, Self::Error> {
        if max_len == 0 || max_len > EXEC_MAX_CHUNK_BYTES {
            return Err(ExecOpError::Guest(GuestOpError::MaxChunkExceeded));
        }
        let response = Self::response_error(
            self.request(
                NamedProcessStreamRequest::Read {
                    stream: match stream {
                        OutputStreamSel::Stdout => ExecStream::Stdout,
                        OutputStreamSel::Stderr => ExecStream::Stderr,
                    },
                    offset,
                    max_len,
                    wait,
                    timeout_ms,
                },
                timeout,
            )
            .await?,
        )?;
        match response {
            NamedProcessStreamResponse::Output(result) => {
                let data =
                    base64_codec::decode(&result.data_base64).map_err(|_| ExecOpError::Protocol)?;
                let data_len = u64::try_from(data.len()).map_err(|_| ExecOpError::Protocol)?;
                if data_len > max_len
                    || result.next_offset < offset
                    || result.next_offset - offset != data_len
                {
                    return Err(ExecOpError::Protocol);
                }
                Ok(ReadOutputOutcome {
                    data,
                    next_offset: result.next_offset,
                    eof: result.eof,
                    dropped_bytes: result.dropped_bytes,
                    truncated: result.truncated,
                    timed_out: result.timed_out,
                })
            }
            NamedProcessStreamResponse::Terminal(_) => Ok(ReadOutputOutcome {
                data: Vec::new(),
                next_offset: offset,
                eof: true,
                dropped_bytes: 0,
                truncated: false,
                timed_out: false,
            }),
            _ => Err(ExecOpError::Protocol),
        }
    }

    async fn signal(
        &self,
        control_seq: u64,
        signo: u32,
        timeout: Duration,
    ) -> Result<(), Self::Error> {
        if !matches!(signo, 1 | 2 | 3 | 9 | 10 | 12 | 15 | 18 | 19 | 20 | 28) {
            return Err(ExecOpError::Protocol);
        }
        let response = Self::response_error(
            self.request(
                NamedProcessStreamRequest::Signal { control_seq, signo },
                timeout,
            )
            .await?,
        )?;
        match response {
            NamedProcessStreamResponse::Delivered(_) => Ok(()),
            _ => Err(ExecOpError::Protocol),
        }
    }

    async fn resize(
        &self,
        control_seq: u64,
        rows: u32,
        cols: u32,
        timeout: Duration,
    ) -> Result<(), Self::Error> {
        if rows == 0 || cols == 0 || rows > u32::from(u16::MAX) || cols > u32::from(u16::MAX) {
            return Err(ExecOpError::Protocol);
        }
        let response = Self::response_error(
            self.request(
                NamedProcessStreamRequest::Resize {
                    control_seq,
                    rows,
                    cols,
                },
                timeout,
            )
            .await?,
        )?;
        match response {
            NamedProcessStreamResponse::Delivered(_) => Ok(()),
            _ => Err(ExecOpError::Protocol),
        }
    }

    async fn wait(&self, timeout_ms: u64, timeout: Duration) -> Result<WaitOutcome, Self::Error> {
        let response = Self::response_error(
            self.request(NamedProcessStreamRequest::Wait { timeout_ms }, timeout)
                .await?,
        )?;
        match response {
            NamedProcessStreamResponse::Wait(result) => Ok(WaitOutcome {
                running: result.running,
                terminal: result
                    .terminal_status
                    .as_ref()
                    .and_then(Self::terminal_kind),
            }),
            NamedProcessStreamResponse::Terminal(status) => Ok(WaitOutcome {
                running: false,
                terminal: Self::terminal_kind(&status),
            }),
            _ => Err(ExecOpError::Protocol),
        }
    }

    async fn close_stdin(&self, offset: u64, timeout: Duration) -> Result<(), Self::Error> {
        let response = Self::response_error(
            self.request(NamedProcessStreamRequest::CloseStdin { offset }, timeout)
                .await?,
        )?;
        match response {
            NamedProcessStreamResponse::Closed(_) => Ok(()),
            _ => Err(ExecOpError::Protocol),
        }
    }

    async fn cancel(&self, _control_seq: u64, timeout: Duration) -> Result<(), Self::Error> {
        tokio::time::timeout(timeout, ComponentSessionExecClient::cancel(self))
            .await
            .map_err(|_| ExecOpError::Timeout)?
    }
}

/// Establishment seam: connect + authenticate + cap-gate + `ExecCreate`.
#[async_trait]
pub trait ExecGuestConnector: Send + Sync {
    async fn establish(&self, spec: &ExecStartSpec) -> Result<Established, ExecEstablishError>;
}

/// One command shuttled from the owner connection to the session worker.
pub struct WorkerCommand {
    pub op: ExecOp,
    pub reply: oneshot::Sender<Result<ExecOpResponse, ExecOpError>>,
}

/// Establish reply shuttled back to the owner before the op loop begins.
pub type EstablishReply = Result<ExecSessionInfo, ExecEstablishError>;

/// Owner-socket teardown seam for the terminal-cleanup reaper.
/// `reap` forces the owner connection's reader to unblock (e.g. by shutting
/// down the socket) so the session slot is released after the command has gone
/// terminal and the cleanup TTL elapsed. It MUST be idempotent and MUST NOT be
/// called while the command is still live.
pub trait OwnerReaper: Send + Sync {
    fn reap(&self);
}

/// A no-op owner reaper for unit tests / callers that drive teardown directly.
pub struct NoopReaper;

impl OwnerReaper for NoopReaper {
    fn reap(&self) {}
}

/// Default terminal-cleanup grace: after the guest command goes
/// terminal, a stalled owner that never closes its connection is reaped after
/// this long so it cannot pin a session slot indefinitely. Generous enough for
/// a well-behaved CLI to read the terminal status and close first. The reaper
/// never kills a LIVE command - cleanup only arms once `Wait` returns terminal.
pub const EXEC_TERMINAL_CLEANUP_TTL: Duration = Duration::from_secs(10);

/// Records when the guest command first went terminal and decides - against an
/// injected [`Clock`] - whether the terminal-cleanup TTL has since elapsed.
/// Pure and fake-clock testable; the worker arms a real timer that consults
/// [`TerminalReaper::due`].
pub struct TerminalReaper {
    clock: Arc<dyn Clock>,
    ttl: Duration,
    terminal_at: Mutex<Option<Instant>>,
}

impl TerminalReaper {
    pub fn new(clock: Arc<dyn Clock>, ttl: Duration) -> Self {
        Self {
            clock,
            ttl,
            terminal_at: Mutex::new(None),
        }
    }

    /// Record the first terminal observation. Idempotent: a later call keeps
    /// the original instant so the TTL is always measured from when the command
    /// FIRST went terminal. Returns `true` only on the transition.
    pub fn mark_terminal(&self) -> bool {
        let mut at = self.terminal_at.lock().expect("terminal reaper poisoned");
        if at.is_none() {
            *at = Some(self.clock.now());
            true
        } else {
            false
        }
    }

    /// Whether the command has been observed terminal at least once.
    pub fn is_terminal(&self) -> bool {
        self.terminal_at
            .lock()
            .expect("terminal reaper poisoned")
            .is_some()
    }

    /// True once the command is terminal AND the TTL has elapsed since.
    pub fn due(&self) -> bool {
        match *self.terminal_at.lock().expect("terminal reaper poisoned") {
            Some(at) => self.clock.now().saturating_duration_since(at) >= self.ttl,
            None => false,
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

/// Inputs to [`spawn_session_worker`].
pub struct WorkerSpawn {
    pub connector: Arc<dyn ExecGuestConnector>,
    pub spec: ExecStartSpec,
    pub deadlines: ExecOpDeadlines,
    pub establish_tx: oneshot::Sender<EstablishReply>,
    pub control_rx: mpsc::Receiver<WorkerCommand>,
    /// Terminal-cleanup grace before the reaper releases a stalled owner's slot.
    pub terminal_ttl: Duration,
    /// Clock for the terminal-cleanup TTL (production: [`SystemClock`]).
    pub clock: Arc<dyn Clock>,
    /// Owner-socket teardown seam fired by the terminal-cleanup reaper.
    pub owner_reaper: Arc<dyn OwnerReaper>,
}

/// Spawn a session worker on its own OS thread with a dedicated current-thread
/// tokio runtime. The worker establishes the session, reports the result over
/// `establish_tx`, then services `WorkerCommand`s until the channel closes.
/// Dropping the sender (owner disconnect) returns the worker, drops the
/// runtime, and drops every client clone - prompting the guest teardown.
pub fn spawn_session_worker(spawn: WorkerSpawn) -> JoinHandle<()> {
    let WorkerSpawn {
        connector,
        spec,
        deadlines,
        establish_tx,
        control_rx,
        terminal_ttl,
        clock,
        owner_reaper,
    } = spawn;
    std::thread::Builder::new()
        .name("d2b-exec".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => {
                    let _ = establish_tx.send(Err(ExecEstablishError::Transport));
                    return;
                }
            };
            runtime.block_on(worker_main(
                connector,
                spec,
                deadlines,
                establish_tx,
                control_rx,
                Arc::new(TerminalReaper::new(clock, terminal_ttl)),
                owner_reaper,
            ));
        })
        .expect("spawn exec session worker thread")
}

async fn worker_main(
    connector: Arc<dyn ExecGuestConnector>,
    spec: ExecStartSpec,
    deadlines: ExecOpDeadlines,
    establish_tx: oneshot::Sender<EstablishReply>,
    mut control_rx: mpsc::Receiver<WorkerCommand>,
    reaper: Arc<TerminalReaper>,
    owner_reaper: Arc<dyn OwnerReaper>,
) {
    let established = match connector.establish(&spec).await {
        Ok(established) => established,
        Err(error) => {
            let _ = establish_tx.send(Err(error));
            return;
        }
    };
    let Established {
        client,
        info,
        control_seq,
        caps,
    } = established;
    if establish_tx.send(Ok(info)).is_err() {
        // Owner vanished before the establish reply landed. Returning here
        // must still reset/cancel the guest process before the client drops.
        let _ = client
            .cancel(control_seq.saturating_add(1), deadlines.control)
            .await;
        return;
    }

    let mut state = WorkerState {
        client,
        deadlines,
        next_stdin_offset: 0,
        control_seq,
        last_write: None,
        stdin_closed: false,
        control_replay: std::collections::VecDeque::new(),
        caps,
    };

    while let Some(WorkerCommand { op, reply }) = control_rx.recv().await {
        match op {
            ExecOp::ReadOutput(_) | ExecOp::Wait(_) => {
                // Fail closed before spawning a `ReadOutput` long-poll if the
                // guest never advertised the output (`ExecLogs`) cap.
                // `Wait` is the terminal-status poll and needs no output cap.
                if matches!(op, ExecOp::ReadOutput(_)) && !state.caps.output {
                    let _ = reply.send(Err(ExecOpError::Capability));
                    continue;
                }
                // Long-polls are spawned so the worker keeps servicing fast
                // control ops while a poll is in flight (no head-of-line
                // blocking). They touch no shared mutable session state.
                let client = Arc::clone(&state.client);
                let deadlines = state.deadlines;
                let reaper = Arc::clone(&reaper);
                let owner_reaper = Arc::clone(&owner_reaper);
                tokio::spawn(async move {
                    let is_wait = matches!(op, ExecOp::Wait(_));
                    let result = run_long_poll(client.as_ref(), op, deadlines).await;
                    // Record terminal state when `Wait` first reports terminal,
                    // then arm the terminal-cleanup reaper. The reaper
                    // only releases the slot AFTER the command is terminal; it
                    // never kills a live command.
                    if is_wait
                        && let Ok(ExecOpResponse::Wait(wait)) = &result
                        && wait.terminal_status.is_some()
                        && reaper.mark_terminal()
                    {
                        arm_terminal_reap(reaper, owner_reaper);
                    }
                    let _ = reply.send(result);
                });
            }
            other => {
                let result = state.handle_inline(other).await;
                let _ = reply.send(result);
            }
        }
    }
    // `control_rx` closed -> owner disconnected. Explicitly cancel/reset the
    // established process before dropping the client so a named stream and
    // its target-local process cannot outlive their owner.
    let _ = state
        .client
        .cancel(state.control_seq.saturating_add(1), state.deadlines.control)
        .await;
}

/// Arm the terminal-cleanup timer: after the TTL elapses, if the command
/// is still terminal (the owner never closed), reap the owner socket so the
/// session slot is released. If the owner closes first the worker is torn down
/// and this task is aborted with the runtime, so the reaper never fires.
fn arm_terminal_reap(reaper: Arc<TerminalReaper>, owner_reaper: Arc<dyn OwnerReaper>) {
    let ttl = reaper.ttl();
    tokio::spawn(async move {
        tokio::time::sleep(ttl).await;
        if reaper.due() {
            owner_reaper.reap();
        }
    });
}

/// Bounded replay cache depth for control ops (Signal/Resize). A retried
/// control op (same client `opId`) replays the cached ack instead of being
/// re-delivered to the guest, so a lost reply never causes a duplicate
/// signal/resize. Interactive sessions issue very few control ops, so a small
/// ring is sufficient.
const CONTROL_REPLAY_CAP: usize = 16;

struct WorkerState {
    client: Arc<dyn ExecGuestClient>,
    deadlines: ExecOpDeadlines,
    next_stdin_offset: u64,
    control_seq: u64,
    last_write: Option<(u64, ExecWriteStdinResult)>,
    stdin_closed: bool,
    // Idempotency ring for control ops keyed by the client-assigned `opId`.
    // `opId == 0` is never cached (legacy / no-dedup).
    control_replay: std::collections::VecDeque<(u64, ExecOpResponse)>,
    // Negotiated caps + session shape for fail-closed per-op gating.
    caps: NegotiatedCaps,
}

impl WorkerState {
    /// Return a cached control-op ack for a previously-served `opId`, if any.
    fn cached_control(&self, op_id: u64) -> Option<ExecOpResponse> {
        if op_id == 0 {
            return None;
        }
        self.control_replay
            .iter()
            .find(|(id, _)| *id == op_id)
            .map(|(_, resp)| resp.clone())
    }

    /// Record a control-op ack so an idempotent retry replays it. `opId == 0`
    /// is not cached.
    fn remember_control(&mut self, op_id: u64, resp: ExecOpResponse) {
        if op_id == 0 {
            return;
        }
        if self.control_replay.len() >= CONTROL_REPLAY_CAP {
            self.control_replay.pop_front();
        }
        self.control_replay.push_back((op_id, resp));
    }
}

impl WorkerState {
    async fn handle_inline(&mut self, op: ExecOp) -> Result<ExecOpResponse, ExecOpError> {
        match op {
            ExecOp::WriteStdin(args) => {
                let data =
                    base64_codec::decode(&args.chunk_base64).map_err(|_| ExecOpError::Protocol)?;
                if data.len() as u64 > EXEC_MAX_CHUNK_BYTES {
                    return Err(ExecOpError::Guest(GuestOpError::MaxChunkExceeded));
                }
                // Idempotent retry of the most recent write at the same offset.
                if let Some((offset, cached)) = &self.last_write
                    && *offset == args.offset
                {
                    return Ok(ExecOpResponse::WriteStdin(cached.clone()));
                }
                if args.offset != self.next_stdin_offset {
                    return Err(ExecOpError::Guest(GuestOpError::OffsetMismatch));
                }
                if self.stdin_closed {
                    return Err(ExecOpError::Guest(GuestOpError::StdinClosed));
                }
                let timeout = self.deadlines.control;
                let outcome = self
                    .client
                    .write_stdin(args.offset, data, args.eof, timeout)
                    .await?;
                self.next_stdin_offset = outcome.next_offset;
                if outcome.stdin_closed {
                    self.stdin_closed = true;
                }
                let result = ExecWriteStdinResult {
                    accepted_len: outcome.accepted_len,
                    next_offset: outcome.next_offset,
                    backpressured: outcome.backpressured,
                    stdin_closed: outcome.stdin_closed,
                };
                // Only cache writes that made progress or closed stdin. A
                // zero-progress (backpressured) write must NOT be replay-cached:
                // its offset never advances, so caching it would pin the session
                // at perpetual backpressure even after the guest budget recovers
                // and the CLI retries the same offset.
                if result.accepted_len > 0 || result.stdin_closed {
                    self.last_write = Some((args.offset, result.clone()));
                }
                Ok(ExecOpResponse::WriteStdin(result))
            }
            ExecOp::Signal(args) => {
                // Fail closed if the guest never advertised the Signals cap.
                if !self.caps.signals {
                    return Err(ExecOpError::Capability);
                }
                if let Some(cached) = self.cached_control(args.op_id) {
                    return Ok(cached);
                }
                self.control_seq = self.control_seq.saturating_add(1);
                let timeout = self.deadlines.control;
                self.client
                    .signal(self.control_seq, args.signo, timeout)
                    .await?;
                let resp = ExecOpResponse::Signal(ExecControlResult { delivered: true });
                self.remember_control(args.op_id, resp.clone());
                Ok(resp)
            }
            ExecOp::Resize(args) => {
                // Resize requires a PTY session AND the guest TtyResize cap; a
                // non-tty session or a guest missing the cap fails closed.
                if !self.caps.tty || !self.caps.tty_resize {
                    return Err(ExecOpError::Capability);
                }
                if let Some(cached) = self.cached_control(args.op_id) {
                    return Ok(cached);
                }
                self.control_seq = self.control_seq.saturating_add(1);
                let timeout = self.deadlines.control;
                self.client
                    .resize(self.control_seq, args.rows, args.cols, timeout)
                    .await?;
                let resp = ExecOpResponse::Resize(ExecControlResult { delivered: true });
                self.remember_control(args.op_id, resp.clone());
                Ok(resp)
            }
            ExecOp::Close(_) => {
                if self.stdin_closed {
                    return Ok(ExecOpResponse::Close(ExecCloseResult {
                        stdin_closed: true,
                    }));
                }
                let timeout = self.deadlines.control;
                // A close on a session whose stdin the process already shut is
                // idempotent: treat a not-open/closed guest error as success.
                match self
                    .client
                    .close_stdin(self.next_stdin_offset, timeout)
                    .await
                {
                    Ok(()) => {}
                    Err(ExecOpError::Guest(
                        GuestOpError::StdinClosed | GuestOpError::StdinNotOpen,
                    )) => {}
                    Err(error) => return Err(error),
                }
                self.stdin_closed = true;
                Ok(ExecOpResponse::Close(ExecCloseResult {
                    stdin_closed: true,
                }))
            }
            ExecOp::Start(_) => Err(ExecOpError::Protocol),
            ExecOp::List(_) | ExecOp::Logs(_) | ExecOp::Status(_) | ExecOp::Kill(_) => {
                Err(ExecOpError::Protocol)
            }
            ExecOp::ReadOutput(_) | ExecOp::Wait(_) => unreachable!("long-polls are spawned"),
        }
    }
}

async fn run_long_poll(
    client: &dyn ExecGuestClient,
    op: ExecOp,
    deadlines: ExecOpDeadlines,
) -> Result<ExecOpResponse, ExecOpError> {
    match op {
        ExecOp::ReadOutput(args) => {
            let stream = match args.stream {
                ExecStream::Stdout => OutputStreamSel::Stdout,
                ExecStream::Stderr => OutputStreamSel::Stderr,
            };
            let max_len = args.max_len.min(EXEC_MAX_CHUNK_BYTES);
            let poll_cap_ms = deadlines.poll_cap.as_millis().min(u64::MAX as u128) as u64;
            let timeout_ms = if args.wait {
                args.timeout_ms.min(poll_cap_ms)
            } else {
                0
            };
            let op_deadline = Duration::from_millis(timeout_ms) + deadlines.poll_slack;
            let outcome = client
                .read_output(
                    stream,
                    args.offset,
                    max_len,
                    args.wait,
                    timeout_ms,
                    op_deadline,
                )
                .await?;
            Ok(ExecOpResponse::ReadOutput(ExecReadOutputResult {
                data_base64: base64_codec::encode(&outcome.data),
                next_offset: outcome.next_offset,
                eof: outcome.eof,
                dropped_bytes: outcome.dropped_bytes,
                truncated: outcome.truncated,
                timed_out: outcome.timed_out,
            }))
        }
        ExecOp::Wait(args) => {
            let poll_cap_ms = deadlines.poll_cap.as_millis().min(u64::MAX as u128) as u64;
            let timeout_ms = args.timeout_ms.min(poll_cap_ms);
            let op_deadline = Duration::from_millis(timeout_ms) + deadlines.poll_slack;
            let outcome = client.wait(timeout_ms, op_deadline).await?;
            Ok(ExecOpResponse::Wait(ExecWaitResult {
                running: outcome.running,
                terminal_status: outcome.terminal.map(map_terminal),
            }))
        }
        _ => unreachable!("only ReadOutput/Wait are long-polls"),
    }
}

fn map_terminal(kind: TerminalKind) -> ExecTerminalStatus {
    match kind {
        TerminalKind::Exited(code) => ExecTerminalStatus::Exited { code },
        TerminalKind::Signaled(signal) => ExecTerminalStatus::Signaled { signal },
        TerminalKind::Error(slug) => ExecTerminalStatus::Error {
            slug: slug.to_owned(),
        },
    }
}

/// Build the `Start` op response from the established session + handle.
pub fn start_response(handle: &str, info: &ExecSessionInfo) -> ExecOpResponse {
    ExecOpResponse::Start(ExecStartResult {
        session: handle.to_owned(),
        tty: info.tty,
        stdout_offset: info.stdout_offset,
        stderr_offset: info.stderr_offset,
    })
}

// ---------------------------------------------------------------------------
// Session table: global / per-uid / per-vm caps + opaque handles.
// ---------------------------------------------------------------------------

/// Monotonic clock seam so the Start rate limiter can be driven deterministically
/// in tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// Production clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Concurrent-session caps. Reservation is fail-closed and happens BEFORE any
/// connect / auth / `ExecCreate`, so a cap breach never spends a guest round
/// trip.
#[derive(Debug, Clone, Copy)]
pub struct ExecSessionCaps {
    pub global: usize,
    pub per_uid: usize,
    pub per_vm: usize,
    /// Max `Start`s per `start_window` per uid (DoS rate limit).
    pub start_burst: usize,
    pub start_window: Duration,
}

impl Default for ExecSessionCaps {
    fn default() -> Self {
        Self {
            global: 64,
            per_uid: 16,
            per_vm: 8,
            start_burst: 32,
            start_window: Duration::from_secs(10),
        }
    }
}

/// Why a session slot could not be reserved. Every variant releases nothing
/// (no slot was taken) and maps to a redaction-safe slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionReserveError {
    GlobalCap,
    PerUidCap,
    PerVmCap,
    RateLimited,
    HandleExhausted,
}

impl SessionReserveError {
    pub fn slug(self) -> &'static str {
        match self {
            Self::GlobalCap => "exec-session-global-cap",
            Self::PerUidCap => "exec-session-per-uid-cap",
            Self::PerVmCap => "exec-session-per-vm-cap",
            Self::RateLimited => "exec-session-rate-limited",
            Self::HandleExhausted => "exec-session-handle-exhausted",
        }
    }
}

#[derive(Debug, Clone)]
struct SessionMeta {
    uid: u32,
    vm: String,
}

struct TableInner {
    sessions: HashMap<String, SessionMeta>,
    /// Per-uid recent Start timestamps for the sliding-window rate limit.
    starts: HashMap<u32, Vec<Instant>>,
}

/// In-process exec session table. Held in `ServerState` behind an `Arc`.
pub struct SessionTable {
    caps: ExecSessionCaps,
    clock: Arc<dyn Clock>,
    inner: Mutex<TableInner>,
}

impl std::fmt::Debug for SessionTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionTable")
            .field("caps", &self.caps)
            .field("live", &self.len())
            .finish()
    }
}

const HANDLE_RETRY_LIMIT: usize = 8;

impl SessionTable {
    pub fn new(caps: ExecSessionCaps) -> Self {
        Self::with_clock(caps, Arc::new(SystemClock))
    }

    pub fn with_clock(caps: ExecSessionCaps, clock: Arc<dyn Clock>) -> Self {
        Self {
            caps,
            clock,
            inner: Mutex::new(TableInner {
                sessions: HashMap::new(),
                starts: HashMap::new(),
            }),
        }
    }

    pub fn caps(&self) -> ExecSessionCaps {
        self.caps
    }

    /// Reserve a slot, generating an opaque handle with the OS CSPRNG. Caps and
    /// the Start rate limit are enforced fail-closed before the handle is
    /// minted; the returned guard releases the slot on drop.
    pub fn reserve(
        self: &Arc<Self>,
        uid: u32,
        vm: &str,
    ) -> Result<SessionSlot, SessionReserveError> {
        self.reserve_with(uid, vm, default_handle_bytes)
    }

    /// Reserve with an injectable 16-byte handle generator (test seam for the
    /// collision path).
    pub fn reserve_with(
        self: &Arc<Self>,
        uid: u32,
        vm: &str,
        mut r#gen: impl FnMut() -> Option<[u8; 16]>,
    ) -> Result<SessionSlot, SessionReserveError> {
        let mut inner = self.inner.lock().expect("exec session table poisoned");
        self.enforce_start_rate(&mut inner, uid)?;
        if inner.sessions.len() >= self.caps.global {
            return Err(SessionReserveError::GlobalCap);
        }
        if inner
            .sessions
            .values()
            .filter(|meta| meta.uid == uid)
            .count()
            >= self.caps.per_uid
        {
            return Err(SessionReserveError::PerUidCap);
        }
        if inner.sessions.values().filter(|meta| meta.vm == vm).count() >= self.caps.per_vm {
            return Err(SessionReserveError::PerVmCap);
        }
        let mut handle = None;
        for _ in 0..HANDLE_RETRY_LIMIT {
            let candidate = match r#gen() {
                Some(bytes) => hex_encode(&bytes),
                None => return Err(SessionReserveError::HandleExhausted),
            };
            if !inner.sessions.contains_key(&candidate) {
                handle = Some(candidate);
                break;
            }
        }
        let handle = handle.ok_or(SessionReserveError::HandleExhausted)?;
        inner.sessions.insert(
            handle.clone(),
            SessionMeta {
                uid,
                vm: vm.to_owned(),
            },
        );
        // Record the Start for the rate window only after a successful reserve.
        inner.starts.entry(uid).or_default().push(self.clock.now());
        Ok(SessionSlot {
            handle,
            uid,
            vm: vm.to_owned(),
            table: Arc::clone(self),
            released: false,
        })
    }

    fn enforce_start_rate(
        &self,
        inner: &mut TableInner,
        uid: u32,
    ) -> Result<(), SessionReserveError> {
        let now = self.clock.now();
        let window = self.caps.start_window;
        let entry = inner.starts.entry(uid).or_default();
        entry.retain(|stamp| now.duration_since(*stamp) < window);
        if entry.len() >= self.caps.start_burst {
            return Err(SessionReserveError::RateLimited);
        }
        Ok(())
    }

    /// True iff `handle` is live AND bound to `uid` (peer-uid binding check).
    pub fn owned_by(&self, handle: &str, uid: u32) -> bool {
        let inner = self.inner.lock().expect("exec session table poisoned");
        inner
            .sessions
            .get(handle)
            .map(|meta| meta.uid == uid)
            .unwrap_or(false)
    }

    /// Live session count (test/observability helper).
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("exec session table poisoned")
            .sessions
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn release(&self, handle: &str) {
        let mut inner = self.inner.lock().expect("exec session table poisoned");
        inner.sessions.remove(handle);
    }
}

/// RAII guard for a reserved session slot. Dropping it releases the slot
/// (every failure path drops the guard, so the slot is always released).
/// `Debug` is redacted so a stray `{:?}` can never leak the unguessable
/// session handle capability token; only the leak-safe uid / vm /
/// released fields are observable.
pub struct SessionSlot {
    handle: String,
    uid: u32,
    vm: String,
    table: Arc<SessionTable>,
    released: bool,
}

impl std::fmt::Debug for SessionSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionSlot")
            .field("uid", &self.uid)
            .field("vm", &self.vm)
            .field("released", &self.released)
            .finish_non_exhaustive()
    }
}

impl SessionSlot {
    pub fn handle(&self) -> &str {
        &self.handle
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn vm(&self) -> &str {
        &self.vm
    }
}

impl Drop for SessionSlot {
    fn drop(&mut self) {
        if !self.released {
            self.released = true;
            self.table.release(&self.handle);
        }
    }
}

fn default_handle_bytes() -> Option<[u8; 16]> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).ok()?;
    Some(bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    out
}

// ===========================================================================
// Tests (hermetic matrices: session-table adversarial, worker lifecycle
// + teardown, no-head-of-line concurrency, backpressure/offset/idempotency,
// and fake-clock rate limiting). All fakes are injected; no live transport.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_zone_session::v3::component_session::{
        AttachmentPolicy, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass,
        ServicePackage, TransportBinding, TransportClass,
    };
    use d2b_session::{
        HandshakeCredentials, OwnedTransport, SessionEngine, TransportDescriptor, TransportError,
        TransportPacket, TransportReader, TransportWriter,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct NamedDriverState {
        sent: Mutex<Vec<Vec<u8>>>,
        events: Mutex<VecDeque<d2b_session::Result<StreamEvent>>>,
        opened: Mutex<Vec<(StreamId, u32, u32)>>,
        granted: AtomicUsize,
        resets: AtomicUsize,
        event_ready: Notify,
    }

    #[derive(Clone, Default)]
    struct NamedDriver {
        state: Arc<NamedDriverState>,
    }

    impl NamedDriver {
        fn push_event(&self, event: d2b_session::Result<StreamEvent>) {
            self.state.events.lock().unwrap().push_back(event);
            self.state.event_ready.notify_one();
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for NamedDriver {
        fn generation(&self) -> u64 {
            1
        }

        async fn start_ttrpc(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
            _frame: Vec<u8>,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn complete_ttrpc(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn cancel(
            &self,
            _generation: u64,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_ttrpc(&self, _frame: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            Err(d2b_session::SessionError::new(
                d2b_contracts_zone_session::v3::component_session::SessionErrorCode::InternalInvariant,
            ))
        }

        async fn register_inbound_call(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<d2b_session::Cancellation> {
            panic!("named-stream test driver does not accept inbound calls")
        }

        async fn mark_inbound_dispatched(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn complete_inbound_call(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn remove_inbound_call(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn send_attachments(
            &self,
            _attachments: Vec<d2b_session::OwnedAttachment>,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_attachments(
            &self,
        ) -> d2b_session::Result<Vec<d2b_session::OwnedAttachment>> {
            Ok(Vec::new())
        }

        async fn open_named_stream(
            &self,
            stream: StreamId,
            send_credit: u32,
            receive_credit: u32,
        ) -> d2b_session::Result<()> {
            self.state
                .opened
                .lock()
                .unwrap()
                .push((stream, send_credit, receive_credit));
            Ok(())
        }

        async fn send_named_stream(
            &self,
            _stream: StreamId,
            bytes: Vec<u8>,
        ) -> d2b_session::Result<()> {
            self.state.sent.lock().unwrap().push(bytes);
            Ok(())
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
            loop {
                if let Some(event) = self.state.events.lock().unwrap().pop_front() {
                    return event;
                }
                self.state.event_ready.notified().await;
            }
        }

        async fn grant_named_stream_credit(
            &self,
            _stream: StreamId,
            bytes: u32,
        ) -> d2b_session::Result<()> {
            self.state
                .granted
                .fetch_add(bytes as usize, Ordering::AcqRel);
            Ok(())
        }

        async fn close_named_stream(&self, _stream: StreamId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn reset_named_stream(&self, _stream: StreamId) -> d2b_session::Result<()> {
            self.state.resets.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        async fn drive_keepalive(&self, _now: Instant) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_control(&self) -> d2b_session::Result<d2b_session::SessionEvent> {
            Err(d2b_session::SessionError::new(
                d2b_contracts_zone_session::v3::component_session::SessionErrorCode::InternalInvariant,
            ))
        }

        async fn close(
            &self,
            _reason: d2b_contracts_zone_session::v3::component_session::CloseReason,
            _remediation: d2b_contracts_zone_session::v3::component_session::Remediation,
        ) -> d2b_session::Result<()> {
            Ok(())
        }
    }

    fn named_event(
        stream: StreamId,
        response: NamedProcessStreamResponse,
    ) -> d2b_session::Result<StreamEvent> {
        named_event_with_id(stream, 1, response)
    }

    fn named_event_with_id(
        stream: StreamId,
        request_id: u64,
        response: NamedProcessStreamResponse,
    ) -> d2b_session::Result<StreamEvent> {
        Ok(StreamEvent::Data {
            stream,
            bytes: serde_json::to_vec(&NamedProcessStreamResponseFrame::new(request_id, response))
                .unwrap(),
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn component_session_exec_client_uses_one_authenticated_named_stream() {
        let stream = StreamId::new(0x100).unwrap();
        let driver = NamedDriver::default();
        driver.push_event(named_event(
            stream,
            NamedProcessStreamResponse::Stdin(ExecWriteStdinResult {
                accepted_len: 2,
                next_offset: 2,
                backpressured: false,
                stdin_closed: false,
            }),
        ));
        let client = ComponentSessionExecClient::open(driver.clone(), 0x100, 1024, 1024)
            .await
            .unwrap();
        let result = client
            .write_stdin(0, b"hi".to_vec(), false, Duration::from_secs(1))
            .await
            .unwrap();
        client.acknowledge_received().await.unwrap();
        assert_eq!(result.next_offset, 2);
        assert_eq!(driver.state.opened.lock().unwrap().len(), 1);
        assert!(driver.state.granted.load(Ordering::Acquire) > 0);
        assert_eq!(driver.state.sent.lock().unwrap().len(), 1);
        let request: NamedProcessStreamRequestFrame =
            serde_json::from_slice(&driver.state.sent.lock().unwrap()[0]).unwrap();
        assert_eq!(request.request_id, 1);
        assert!(matches!(
            request.request,
            NamedProcessStreamRequest::Stdin { .. }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn component_session_exec_client_releases_credit_after_output_consumption() {
        let stream = StreamId::new(0x101).unwrap();
        let driver = NamedDriver::default();
        let output = NamedProcessStreamResponse::Output(ExecReadOutputResult {
            data_base64: base64_codec::encode(b"output"),
            next_offset: 6,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        });
        driver.push_event(named_event(stream, output));
        let client = ComponentSessionExecClient::open(driver.clone(), 0x101, 1024, 1024)
            .await
            .unwrap();
        let result = client
            .read_output(
                OutputStreamSel::Stdout,
                0,
                64,
                false,
                0,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(result.data, b"output");
        assert_eq!(driver.state.granted.load(Ordering::Acquire), 0);
        client.acknowledge_received().await.unwrap();
        assert!(driver.state.granted.load(Ordering::Acquire) > 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn component_session_exec_client_cancel_resets_the_named_stream_once() {
        let driver = NamedDriver::default();
        let client = ComponentSessionExecClient::open(driver.clone(), 0x105, 1024, 1024)
            .await
            .unwrap();
        client.cancel().await.unwrap();
        client.cancel().await.unwrap();
        assert_eq!(driver.state.resets.load(Ordering::Acquire), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn component_session_exec_client_correlates_out_of_order_responses() {
        let stream = StreamId::new(0x102).unwrap();
        let driver = NamedDriver::default();
        let reply = |next_offset| {
            NamedProcessStreamResponse::Stdin(ExecWriteStdinResult {
                accepted_len: 1,
                next_offset,
                backpressured: false,
                stdin_closed: false,
            })
        };
        driver.push_event(named_event_with_id(stream, 2, reply(2)));
        driver.push_event(named_event_with_id(stream, 1, reply(1)));
        let client = ComponentSessionExecClient::open(driver, 0x102, 1024, 1024)
            .await
            .unwrap();
        let (first, second) = tokio::join!(
            client.write_stdin(0, b"a".to_vec(), false, Duration::from_secs(1)),
            client.write_stdin(1, b"b".to_vec(), false, Duration::from_secs(1)),
        );
        assert_eq!(first.unwrap().next_offset, 1);
        assert_eq!(second.unwrap().next_offset, 2);
    }

    struct DriverTestTransport {
        sender: mpsc::Sender<TransportPacket>,
        receiver: Option<mpsc::Receiver<TransportPacket>>,
        descriptor: TransportDescriptor,
    }

    struct DriverTestReader {
        receiver: mpsc::Receiver<TransportPacket>,
    }

    struct DriverTestWriter {
        sender: mpsc::Sender<TransportPacket>,
    }

    #[async_trait]
    impl OwnedTransport for DriverTestTransport {
        fn descriptor(&self) -> TransportDescriptor {
            self.descriptor
        }

        fn into_split(mut self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
            (
                Box::new(DriverTestReader {
                    receiver: self.receiver.take().expect("driver test reader"),
                }),
                Box::new(DriverTestWriter {
                    sender: self.sender.clone(),
                }),
            )
        }

        async fn receive(
            &mut self,
            protected_limit: usize,
        ) -> Result<TransportPacket, TransportError> {
            let receiver = self.receiver.as_mut().expect("handshake receiver");
            let packet = receiver.recv().await.ok_or(TransportError::Disconnected)?;
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

    #[async_trait]
    impl TransportReader for DriverTestReader {
        async fn receive(
            &mut self,
            protected_limit: usize,
        ) -> Result<TransportPacket, TransportError> {
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
    }

    #[async_trait]
    impl TransportWriter for DriverTestWriter {
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

    fn driver_test_policy() -> EndpointPolicy {
        EndpointPolicy {
            purpose: EndpointPurpose::LocalLifecycle,
            purpose_class: PurposeClass::Local,
            initiator_role: EndpointRole::ZoneController,
            responder_role: EndpointRole::Component,
            service: ServicePackage::ResourceV3,
            schema_fingerprint: [0x11; 32],
            noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
            limits: LimitProfile::local_default(),
            transport_binding: TransportBinding {
                transport: TransportClass::UnixSeqpacket,
                locality: Locality::HostLocal,
                channel_binding: [0x22; 32],
                identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
            },
            reconnect_generation: 1,
            attachment_policy: AttachmentPolicy::disabled(),
        }
    }

    fn driver_test_transport_pair() -> (DriverTestTransport, DriverTestTransport) {
        let (left_sender, right_receiver) = mpsc::channel(128);
        let (right_sender, left_receiver) = mpsc::channel(128);
        let descriptor = TransportDescriptor {
            class: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            packet_atomic: true,
            supports_attachments: false,
        };
        (
            DriverTestTransport {
                sender: left_sender,
                receiver: Some(left_receiver),
                descriptor,
            },
            DriverTestTransport {
                sender: right_sender,
                receiver: Some(right_receiver),
                descriptor,
            },
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn component_session_exec_client_demuxes_concurrent_controls_on_real_driver_handle() {
        let (initiator_transport, responder_transport) = driver_test_transport_pair();
        let policy = driver_test_policy();
        let now = Instant::now();
        let (initiator, responder) = tokio::join!(
            SessionEngine::establish_initiator(
                initiator_transport,
                policy.clone(),
                HandshakeCredentials::Nn,
                now,
            ),
            SessionEngine::establish_responder(
                responder_transport,
                policy,
                HandshakeCredentials::Nn,
                now,
            ),
        );
        let initiator = initiator.expect("initiator session");
        let responder = responder.expect("responder session");
        let initiator = initiator.into_driver();
        let responder = responder.into_driver();
        let stream = StreamId::new(0x104).unwrap();
        responder
            .open_named_stream(stream, 64 * 1024, 64 * 1024)
            .await
            .unwrap();

        let wait_gate = Arc::new(Notify::new());
        let peer_gate = Arc::clone(&wait_gate);
        let wait_seen = Arc::new(Notify::new());
        let peer_wait_seen = Arc::clone(&wait_seen);
        let peer = tokio::spawn(async move {
            while let Ok(event) = responder.receive_named_stream().await {
                let StreamEvent::Data { stream, bytes } = event else {
                    continue;
                };
                let frame: NamedProcessStreamRequestFrame =
                    serde_json::from_slice(&bytes).expect("request frame");
                if let NamedProcessStreamRequest::Wait { .. } = frame.request {
                    let responder = responder.clone();
                    let peer_gate = Arc::clone(&peer_gate);
                    peer_wait_seen.notify_one();
                    tokio::spawn(async move {
                        peer_gate.notified().await;
                        let response = NamedProcessStreamResponseFrame::new(
                            frame.request_id,
                            NamedProcessStreamResponse::Wait(ExecWaitResult {
                                running: true,
                                terminal_status: None,
                            }),
                        );
                        responder
                            .send_named_stream(stream, serde_json::to_vec(&response).unwrap())
                            .await
                            .expect("wait response frame");
                    });
                    continue;
                }
                let response = match frame.request {
                    NamedProcessStreamRequest::Resize { .. } => {
                        NamedProcessStreamResponse::Delivered(ExecControlResult { delivered: true })
                    }
                    _ => NamedProcessStreamResponse::Error(
                        d2b_contracts_control::public_wire::NamedProcessStreamError {
                            kind: NamedProcessStreamErrorKind::Protocol,
                        },
                    ),
                };
                let response = NamedProcessStreamResponseFrame::new(frame.request_id, response);
                responder
                    .send_named_stream(stream, serde_json::to_vec(&response).unwrap())
                    .await
                    .expect("response frame");
            }
        });

        let client = Arc::new(
            ComponentSessionExecClient::open(initiator, 0x104, 64 * 1024, 64 * 1024)
                .await
                .unwrap(),
        );
        let wait_client = Arc::clone(&client);
        let wait =
            tokio::spawn(async move { wait_client.wait(1_000, Duration::from_secs(2)).await });
        tokio::time::timeout(Duration::from_secs(1), wait_seen.notified())
            .await
            .expect("peer did not receive the long poll");
        let resize_client = Arc::clone(&client);
        let resize = tokio::spawn(async move {
            resize_client
                .resize(1, 24, 80, Duration::from_secs(2))
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_secs(1), resize)
                .await
                .expect("resize was head-of-line blocked")
                .unwrap()
                .is_ok()
        );
        wait_gate.notify_one();
        assert!(wait.await.unwrap().unwrap().running);
        client.cancel().await.unwrap();
        peer.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn component_session_exec_client_rejects_unallowlisted_controls() {
        let driver = NamedDriver::default();
        let client = ComponentSessionExecClient::open(driver.clone(), 0x103, 1024, 1024)
            .await
            .unwrap();
        assert_eq!(
            client
                .signal(1, 4, Duration::from_secs(1))
                .await
                .unwrap_err(),
            ExecOpError::Protocol
        );
        assert_eq!(
            client
                .resize(1, u32::from(u16::MAX) + 1, 80, Duration::from_secs(1))
                .await
                .unwrap_err(),
            ExecOpError::Protocol
        );
        assert!(driver.state.sent.lock().unwrap().is_empty());
    }

    use d2b_contracts_control::public_wire::{
        ExecCloseArgs, ExecReadOutputArgs, ExecResizeArgs, ExecSignalArgs, ExecStream,
        ExecWaitArgs, ExecWriteStdinArgs,
    };

    #[test]
    fn exec_start_spec_debug_redacts_argv_env_cwd() {
        // A stray `{:?}` on the resolved establishment spec must never
        // leak argv, env keys/values, or cwd; only the VM name, shape, and
        // counts are observable.
        const SECRET_ARGV: &str = "SENTINEL_ARGV_dspc";
        const SECRET_KEY: &str = "SENTINEL_ENV_KEY_dspc";
        const SECRET_VAL: &str = "SENTINEL_ENV_VAL_dspc";
        const SECRET_CWD: &str = "SENTINEL_CWD_dspc";
        const SECRET_REQUEST_ID: &str = "SENTINEL_REQUEST_ID_dspc";
        let spec = ExecStartSpec {
            vm: "corp-vm".to_owned(),
            request_id: Some(SECRET_REQUEST_ID.to_owned()),
            argv: vec!["sh".to_owned(), SECRET_ARGV.to_owned()],
            tty: true,
            detached: false,
            env: vec![(SECRET_KEY.to_owned(), SECRET_VAL.to_owned())],
            cwd: Some(SECRET_CWD.to_owned()),
            term_size: Some((24, 80)),
        };
        let rendered = format!("{spec:?}");
        for secret in [
            SECRET_ARGV,
            SECRET_KEY,
            SECRET_VAL,
            SECRET_CWD,
            SECRET_REQUEST_ID,
        ] {
            assert!(
                !rendered.contains(secret),
                "ExecStartSpec Debug leaked {secret}: {rendered}"
            );
        }
        assert!(rendered.contains("corp-vm"), "vm name is observable");
        assert!(rendered.contains("argv_len"), "argv length is observable");
        assert!(rendered.contains("env_len"), "env length is observable");
    }

    // ---- Fake clock (drives the Start rate-limit window deterministically) --

    struct FakeClock {
        now: Mutex<Instant>,
    }

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                now: Mutex::new(Instant::now()),
            })
        }
        fn advance(&self, by: Duration) {
            let mut now = self.now.lock().unwrap();
            *now += by;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
    }

    // ---- Fake guest client ----------------------------------------------------

    #[derive(Default)]
    struct FakeShared {
        write_calls: AtomicUsize,
        close_calls: AtomicUsize,
        cancel_calls: AtomicUsize,
        signal_calls: AtomicUsize,
        resize_calls: AtomicUsize,
        read_calls: AtomicUsize,
        // Per-op transport deadline recorded in call order, tagged by op kind.
        // Lets a test assert each op draws a FRESH per-op deadline rather than
        // sharing one cumulative session budget.
        op_timeouts: Mutex<Vec<(&'static str, Duration)>>,
    }

    struct FakeClient {
        alive: Arc<AtomicUsize>,
        shared: Arc<FakeShared>,
        write_outcome: WriteStdinOutcome,
        stdout_reads: Mutex<VecDeque<ReadOutputOutcome>>,
        stderr_reads: Mutex<VecDeque<ReadOutputOutcome>>,
        waits: Mutex<VecDeque<WaitOutcome>>,
        read_gate: Option<Arc<tokio::sync::Notify>>,
    }

    impl Drop for FakeClient {
        fn drop(&mut self) {
            self.alive.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl TerminalBackend for FakeClient {
        type Error = ExecOpError;

        async fn write_stdin(
            &self,
            _offset: u64,
            _data: Vec<u8>,
            _eof: bool,
            timeout: Duration,
        ) -> Result<WriteStdinOutcome, ExecOpError> {
            self.shared.write_calls.fetch_add(1, Ordering::SeqCst);
            self.shared
                .op_timeouts
                .lock()
                .unwrap()
                .push(("write", timeout));
            Ok(self.write_outcome.clone())
        }

        async fn read_output(
            &self,
            stream: OutputStreamSel,
            _offset: u64,
            _max_len: u64,
            _wait: bool,
            _timeout_ms: u64,
            timeout: Duration,
        ) -> Result<ReadOutputOutcome, ExecOpError> {
            self.shared.read_calls.fetch_add(1, Ordering::SeqCst);
            self.shared
                .op_timeouts
                .lock()
                .unwrap()
                .push(("read", timeout));
            if let Some(gate) = &self.read_gate {
                gate.notified().await;
            }
            let queue = match stream {
                OutputStreamSel::Stdout => &self.stdout_reads,
                OutputStreamSel::Stderr => &self.stderr_reads,
            };
            let outcome = queue.lock().unwrap().pop_front();
            Ok(outcome.unwrap_or(ReadOutputOutcome {
                data: Vec::new(),
                next_offset: 0,
                eof: true,
                dropped_bytes: 0,
                truncated: false,
                timed_out: false,
            }))
        }

        async fn signal(
            &self,
            _control_seq: u64,
            _signo: u32,
            timeout: Duration,
        ) -> Result<(), ExecOpError> {
            self.shared.signal_calls.fetch_add(1, Ordering::SeqCst);
            self.shared
                .op_timeouts
                .lock()
                .unwrap()
                .push(("signal", timeout));
            Ok(())
        }

        async fn resize(
            &self,
            _control_seq: u64,
            _rows: u32,
            _cols: u32,
            timeout: Duration,
        ) -> Result<(), ExecOpError> {
            self.shared.resize_calls.fetch_add(1, Ordering::SeqCst);
            self.shared
                .op_timeouts
                .lock()
                .unwrap()
                .push(("resize", timeout));
            Ok(())
        }

        async fn wait(
            &self,
            _timeout_ms: u64,
            timeout: Duration,
        ) -> Result<WaitOutcome, ExecOpError> {
            self.shared
                .op_timeouts
                .lock()
                .unwrap()
                .push(("wait", timeout));
            let outcome = self.waits.lock().unwrap().pop_front();
            Ok(outcome.unwrap_or(WaitOutcome {
                running: false,
                terminal: Some(TerminalKind::Exited(0)),
            }))
        }

        async fn close_stdin(&self, _offset: u64, timeout: Duration) -> Result<(), ExecOpError> {
            self.shared.close_calls.fetch_add(1, Ordering::SeqCst);
            self.shared
                .op_timeouts
                .lock()
                .unwrap()
                .push(("close", timeout));
            Ok(())
        }

        async fn cancel(&self, _control_seq: u64, timeout: Duration) -> Result<(), ExecOpError> {
            self.shared.cancel_calls.fetch_add(1, Ordering::SeqCst);
            self.shared
                .op_timeouts
                .lock()
                .unwrap()
                .push(("cancel", timeout));
            Ok(())
        }
    }

    // ---- Fake connector (establish once from a builder closure) ----------------

    type Builder = Box<dyn FnOnce() -> Established + Send>;

    struct FakeConnector {
        builder: Mutex<Option<Builder>>,
        error: Option<ExecEstablishError>,
    }

    impl FakeConnector {
        fn ok(builder: Builder) -> Arc<Self> {
            Arc::new(Self {
                builder: Mutex::new(Some(builder)),
                error: None,
            })
        }
        fn failing(error: ExecEstablishError) -> Arc<Self> {
            Arc::new(Self {
                builder: Mutex::new(None),
                error: Some(error),
            })
        }
    }

    #[async_trait]
    impl ExecGuestConnector for FakeConnector {
        async fn establish(
            &self,
            _spec: &ExecStartSpec,
        ) -> Result<Established, ExecEstablishError> {
            if let Some(error) = self.error {
                return Err(error);
            }
            let builder = self.builder.lock().unwrap().take().expect("establish once");
            Ok(builder())
        }
    }

    fn spec() -> ExecStartSpec {
        ExecStartSpec {
            vm: "work".to_owned(),
            request_id: None,
            argv: vec!["true".to_owned()],
            tty: false,
            detached: false,
            env: Vec::new(),
            cwd: None,
            term_size: None,
        }
    }

    fn established(client: Arc<dyn ExecGuestClient>) -> Established {
        established_with_caps(client, NegotiatedCaps::all())
    }

    fn established_with_caps(
        client: Arc<dyn ExecGuestClient>,
        caps: NegotiatedCaps,
    ) -> Established {
        Established {
            client,
            info: ExecSessionInfo {
                tty: false,
                stdout_offset: 0,
                stderr_offset: 0,
            },
            control_seq: 0,
            caps,
        }
    }

    /// Drive one op through a worker over the sync command channel exactly like
    /// the owner connection does (blocking_send + blocking_recv).
    fn send_op(
        tx: &mpsc::Sender<WorkerCommand>,
        op: ExecOp,
    ) -> Result<ExecOpResponse, ExecOpError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.blocking_send(WorkerCommand {
            op,
            reply: reply_tx,
        })
        .expect("worker accepts command");
        reply_rx.blocking_recv().expect("worker replies")
    }

    fn start_worker(
        connector: Arc<dyn ExecGuestConnector>,
    ) -> (mpsc::Sender<WorkerCommand>, JoinHandle<()>, EstablishReply) {
        start_worker_with_deadlines(connector, ExecOpDeadlines::default())
    }

    fn start_worker_with_deadlines(
        connector: Arc<dyn ExecGuestConnector>,
        deadlines: ExecOpDeadlines,
    ) -> (mpsc::Sender<WorkerCommand>, JoinHandle<()>, EstablishReply) {
        let (control_tx, control_rx) = mpsc::channel(16);
        let (establish_tx, establish_rx) = oneshot::channel();
        let worker = spawn_session_worker(WorkerSpawn {
            connector,
            spec: spec(),
            deadlines,
            establish_tx,
            control_rx,
            terminal_ttl: EXEC_TERMINAL_CLEANUP_TTL,
            clock: Arc::new(SystemClock),
            owner_reaper: Arc::new(NoopReaper),
        });
        let reply = establish_rx.blocking_recv().expect("establish reply");
        (control_tx, worker, reply)
    }

    // ---- (a) disconnect lifecycle / teardown ----------------------------------

    #[test]
    fn dropping_owner_channel_drops_the_authenticated_client() {
        let alive = Arc::new(AtomicUsize::new(0));
        let alive_for_builder = Arc::clone(&alive);
        let shared = Arc::new(FakeShared::default());
        let shared_for_builder = Arc::clone(&shared);
        let builder: Builder = Box::new(move || {
            alive_for_builder.fetch_add(1, Ordering::SeqCst);
            established(Arc::new(FakeClient {
                alive: alive_for_builder,
                shared: shared_for_builder,
                write_outcome: WriteStdinOutcome {
                    accepted_len: 0,
                    next_offset: 0,
                    backpressured: false,
                    stdin_closed: false,
                },
                stdout_reads: Mutex::new(VecDeque::new()),
                stderr_reads: Mutex::new(VecDeque::new()),
                waits: Mutex::new(VecDeque::new()),
                read_gate: None,
            }))
        });
        let (control_tx, worker, reply) = start_worker(FakeConnector::ok(builder));
        assert!(reply.is_ok());
        assert_eq!(
            alive.load(Ordering::SeqCst),
            1,
            "client alive after establish"
        );

        // Owner disconnects: drop the channel, join the worker.
        drop(control_tx);
        worker.join().expect("worker joins");
        assert_eq!(
            alive.load(Ordering::SeqCst),
            0,
            "client dropped on teardown (prompts guest close_connection)"
        );
        assert_eq!(
            shared.cancel_calls.load(Ordering::SeqCst),
            1,
            "owner disconnect must cancel the established process"
        );
    }

    #[test]
    fn dropping_channel_mid_long_poll_aborts_and_drops_client() {
        let alive = Arc::new(AtomicUsize::new(0));
        let alive_for_builder = Arc::clone(&alive);
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_builder = Arc::clone(&gate);
        let shared = Arc::new(FakeShared::default());
        let shared_for_builder = Arc::clone(&shared);
        let builder: Builder = Box::new(move || {
            alive_for_builder.fetch_add(1, Ordering::SeqCst);
            established(Arc::new(FakeClient {
                alive: alive_for_builder,
                shared: shared_for_builder,
                write_outcome: WriteStdinOutcome {
                    accepted_len: 0,
                    next_offset: 0,
                    backpressured: false,
                    stdin_closed: false,
                },
                stdout_reads: Mutex::new(VecDeque::new()),
                stderr_reads: Mutex::new(VecDeque::new()),
                waits: Mutex::new(VecDeque::new()),
                // Never released: the long-poll parks forever until teardown.
                read_gate: Some(gate_for_builder),
            }))
        });
        let (control_tx, worker, reply) = start_worker(FakeConnector::ok(builder));
        assert!(reply.is_ok());

        // Fire a long-poll that will park on the gate, then tear down without
        // ever releasing it. The runtime drop must abort the parked task and
        // drop its client clone.
        let (reply_tx, _reply_rx) = oneshot::channel();
        control_tx
            .blocking_send(WorkerCommand {
                op: ExecOp::ReadOutput(ExecReadOutputArgs {
                    session: "h".to_owned(),
                    stream: ExecStream::Stdout,
                    offset: 0,
                    max_len: 1024,
                    wait: true,
                    timeout_ms: 60_000,
                }),
                reply: reply_tx,
            })
            .expect("send long-poll");

        drop(control_tx);
        let _ = gate; // keep the notify alive; it must not keep the client alive
        worker.join().expect("worker joins");
        assert_eq!(
            alive.load(Ordering::SeqCst),
            0,
            "parked long-poll's client clone dropped at runtime teardown"
        );
        assert_eq!(
            shared.cancel_calls.load(Ordering::SeqCst),
            1,
            "disconnect must cancel even while a long poll is active"
        );
    }

    #[test]
    fn establish_failure_reports_error_and_joins_clean() {
        let connector = FakeConnector::failing(ExecEstablishError::OldGeneration);
        let (control_tx, worker, reply) = start_worker(connector);
        assert_eq!(reply, Err(ExecEstablishError::OldGeneration));
        drop(control_tx);
        worker.join().expect("worker joins after establish failure");
    }

    // ---- (i) no head-of-line: fast op serviced while a long-poll is parked -----

    #[test]
    fn fast_control_op_completes_while_long_poll_is_parked() {
        let shared = Arc::new(FakeShared::default());
        let shared_for_builder = Arc::clone(&shared);
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_builder = Arc::clone(&gate);
        let alive = Arc::new(AtomicUsize::new(0));
        let alive_for_builder = Arc::clone(&alive);
        let mut stdout_reads = VecDeque::new();
        stdout_reads.push_back(ReadOutputOutcome {
            data: b"late".to_vec(),
            next_offset: 4,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        });
        let builder: Builder = Box::new(move || {
            alive_for_builder.fetch_add(1, Ordering::SeqCst);
            established(Arc::new(FakeClient {
                alive: alive_for_builder,
                shared: shared_for_builder,
                write_outcome: WriteStdinOutcome {
                    accepted_len: 0,
                    next_offset: 0,
                    backpressured: false,
                    stdin_closed: false,
                },
                stdout_reads: Mutex::new(stdout_reads),
                stderr_reads: Mutex::new(VecDeque::new()),
                waits: Mutex::new(VecDeque::new()),
                read_gate: Some(gate_for_builder),
            }))
        });
        let (control_tx, worker, reply) = start_worker(FakeConnector::ok(builder));
        assert!(reply.is_ok());

        // 1. Enqueue a long-poll that parks on the gate (reply held, not read).
        let (poll_reply_tx, poll_reply_rx) = oneshot::channel();
        control_tx
            .blocking_send(WorkerCommand {
                op: ExecOp::ReadOutput(ExecReadOutputArgs {
                    session: "h".to_owned(),
                    stream: ExecStream::Stdout,
                    offset: 0,
                    max_len: 1024,
                    wait: true,
                    timeout_ms: 60_000,
                }),
                reply: poll_reply_tx,
            })
            .expect("send long-poll");

        // 2. A fast Signal must complete promptly even though the poll parks.
        let signal = send_op(
            &control_tx,
            ExecOp::Signal(ExecSignalArgs {
                session: "h".to_owned(),
                signo: 2,
                op_id: 0,
            }),
        );
        assert!(matches!(signal, Ok(ExecOpResponse::Signal(_))));
        assert_eq!(shared.signal_calls.load(Ordering::SeqCst), 1);

        // 3. Release the gate; the long-poll now resolves with its data.
        gate.notify_one();
        let poll = poll_reply_rx.blocking_recv().expect("poll resolves");
        match poll {
            Ok(ExecOpResponse::ReadOutput(result)) => {
                assert_eq!(base64_codec::decode(&result.data_base64).unwrap(), b"late");
            }
            other => panic!("expected ReadOutput, got {other:?}"),
        }

        drop(control_tx);
        worker.join().expect("worker joins");
    }

    // ---- (e) backpressure / offset / idempotency ------------------------------

    fn write_op(offset: u64, chunk: &[u8]) -> ExecOp {
        ExecOp::WriteStdin(ExecWriteStdinArgs {
            session: "h".to_owned(),
            offset,
            chunk_base64: base64_codec::encode(chunk),
            eof: false,
        })
    }

    fn backpressure_worker(
        write_outcome: WriteStdinOutcome,
    ) -> (mpsc::Sender<WorkerCommand>, JoinHandle<()>, Arc<FakeShared>) {
        backpressure_worker_with_deadlines(write_outcome, ExecOpDeadlines::default())
    }

    fn backpressure_worker_with_deadlines(
        write_outcome: WriteStdinOutcome,
        deadlines: ExecOpDeadlines,
    ) -> (mpsc::Sender<WorkerCommand>, JoinHandle<()>, Arc<FakeShared>) {
        let shared = Arc::new(FakeShared::default());
        let shared_for_builder = Arc::clone(&shared);
        let builder: Builder = Box::new(move || {
            established(Arc::new(FakeClient {
                alive: Arc::new(AtomicUsize::new(1)),
                shared: shared_for_builder,
                write_outcome,
                stdout_reads: Mutex::new(VecDeque::new()),
                stderr_reads: Mutex::new(VecDeque::new()),
                waits: Mutex::new(VecDeque::new()),
                read_gate: None,
            }))
        });
        let (control_tx, worker, reply) =
            start_worker_with_deadlines(FakeConnector::ok(builder), deadlines);
        assert!(reply.is_ok());
        (control_tx, worker, shared)
    }

    /// A worker whose session advertises exactly `caps`, for per-op fail-closed
    /// gating tests. The fake client records whether each op reached it.
    fn gated_worker(
        caps: NegotiatedCaps,
    ) -> (mpsc::Sender<WorkerCommand>, JoinHandle<()>, Arc<FakeShared>) {
        let shared = Arc::new(FakeShared::default());
        let shared_for_builder = Arc::clone(&shared);
        let builder: Builder = Box::new(move || {
            established_with_caps(
                Arc::new(FakeClient {
                    alive: Arc::new(AtomicUsize::new(1)),
                    shared: shared_for_builder,
                    write_outcome: WriteStdinOutcome {
                        accepted_len: 0,
                        next_offset: 0,
                        backpressured: false,
                        stdin_closed: false,
                    },
                    stdout_reads: Mutex::new(VecDeque::new()),
                    stderr_reads: Mutex::new(VecDeque::new()),
                    waits: Mutex::new(VecDeque::new()),
                    read_gate: None,
                }),
                caps,
            )
        });
        let (control_tx, worker, reply) = start_worker(FakeConnector::ok(builder));
        assert!(reply.is_ok());
        (control_tx, worker, shared)
    }

    #[test]
    fn signal_without_signals_cap_fails_closed() {
        let (tx, worker, shared) = gated_worker(NegotiatedCaps {
            tty: false,
            signals: false,
            tty_resize: false,
            output: true,
        });
        let err = send_op(
            &tx,
            ExecOp::Signal(ExecSignalArgs {
                session: "h".to_owned(),
                signo: 2,
                op_id: 0,
            }),
        )
        .expect_err("missing Signals cap fails closed");
        assert_eq!(err, ExecOpError::Capability);
        assert_eq!(
            shared.signal_calls.load(Ordering::SeqCst),
            0,
            "signal must never reach the guest without the cap"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn resize_on_non_tty_session_fails_closed() {
        let (tx, worker, shared) = gated_worker(NegotiatedCaps {
            tty: false,
            signals: true,
            tty_resize: true,
            output: true,
        });
        let err = send_op(
            &tx,
            ExecOp::Resize(ExecResizeArgs {
                session: "h".to_owned(),
                rows: 40,
                cols: 120,
                op_id: 0,
            }),
        )
        .expect_err("resize on a non-tty session fails closed");
        assert_eq!(err, ExecOpError::Capability);
        assert_eq!(shared.resize_calls.load(Ordering::SeqCst), 0);
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn resize_without_tty_resize_cap_fails_closed() {
        let (tx, worker, shared) = gated_worker(NegotiatedCaps {
            tty: true,
            signals: true,
            tty_resize: false,
            output: true,
        });
        let err = send_op(
            &tx,
            ExecOp::Resize(ExecResizeArgs {
                session: "h".to_owned(),
                rows: 40,
                cols: 120,
                op_id: 0,
            }),
        )
        .expect_err("missing TtyResize cap fails closed");
        assert_eq!(err, ExecOpError::Capability);
        assert_eq!(shared.resize_calls.load(Ordering::SeqCst), 0);
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn read_output_without_output_cap_fails_closed() {
        let (tx, worker, shared) = gated_worker(NegotiatedCaps {
            tty: false,
            signals: true,
            tty_resize: false,
            output: false,
        });
        let err = send_op(
            &tx,
            ExecOp::ReadOutput(ExecReadOutputArgs {
                session: "h".to_owned(),
                stream: ExecStream::Stdout,
                offset: 0,
                max_len: 1024,
                wait: false,
                timeout_ms: 0,
            }),
        )
        .expect_err("missing ExecLogs/output cap fails closed");
        assert_eq!(err, ExecOpError::Capability);
        assert_eq!(
            shared.read_calls.load(Ordering::SeqCst),
            0,
            "ReadOutput must never reach the guest without the output cap"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn partial_write_reports_accepted_len_and_advances_offset() {
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 3,
            next_offset: 3,
            backpressured: false,
            stdin_closed: false,
        });
        let resp = send_op(&tx, write_op(0, b"abcdef")).expect("write ok");
        match resp {
            ExecOpResponse::WriteStdin(result) => {
                assert_eq!(result.accepted_len, 3);
                assert_eq!(result.next_offset, 3);
            }
            other => panic!("expected WriteStdin, got {other:?}"),
        }
        assert_eq!(shared.write_calls.load(Ordering::SeqCst), 1);
        drop(tx);
        worker.join().unwrap();
    }

    /// Each proxied op draws a FRESH per-op transport deadline. A long-lived
    /// session does NOT share one cumulative budget that shrinks as the session
    /// ages, so a session whose total elapsed time exceeds a single op deadline
    /// still serves later ops with a full deadline.
    ///
    /// This is a regression GUARD against re-introducing a shared
    /// absolute deadline / `AttemptBudget` in the deadline-selection path. The
    /// session is genuinely AGED past one op deadline (real wall-clock elapses
    /// between ops via `sleep`), then we assert the recorded per-op `timeout`
    /// the worker hands the guest is STILL the full control deadline. A shared
    /// absolute deadline minted at establish would compute `remaining =
    /// deadline - now` and record a value shrunk by the elapsed age (here
    /// saturating toward zero), failing the equality below. The recorded
    /// `timeout` is the real output of the deadline-selection seam
    /// (`handle_inline`/`run_long_poll`), not a separate test-only value.
    #[test]
    fn each_proxied_op_draws_a_fresh_per_op_deadline_not_a_shared_session_budget() {
        // Deliberately tiny control deadline so the inter-op aging sleep is
        // comfortably LONGER than a single op deadline: an absolute-deadline
        // regression would record ~0 for the later ops, not the full 60ms.
        let deadlines = ExecOpDeadlines {
            control: Duration::from_millis(60),
            ..ExecOpDeadlines::default()
        };
        let age = deadlines.control * 3;
        let (tx, worker, shared) = backpressure_worker_with_deadlines(
            WriteStdinOutcome {
                accepted_len: 3,
                next_offset: 3,
                backpressured: false,
                stdin_closed: false,
            },
            deadlines,
        );

        // Drive a sequence of ops on ONE long-lived session, AGING the session
        // by real wall-clock time (> one op deadline) between each op.
        send_op(&tx, write_op(0, b"abc")).expect("first write");
        std::thread::sleep(age);
        send_op(
            &tx,
            ExecOp::Signal(ExecSignalArgs {
                session: "h".to_owned(),
                signo: 2,
                op_id: 0,
            }),
        )
        .expect("signal");
        std::thread::sleep(age);
        send_op(&tx, write_op(3, b"def")).expect("second write");
        std::thread::sleep(age);
        send_op(
            &tx,
            ExecOp::Wait(ExecWaitArgs {
                session: "h".to_owned(),
                timeout_ms: 1_000,
            }),
        )
        .expect("wait");

        drop(tx);
        worker.join().unwrap();

        let recorded = shared.op_timeouts.lock().unwrap().clone();

        // Every control op got the FULL fresh control deadline, even though the
        // session has aged well past one op deadline between each...
        let control: Vec<Duration> = recorded
            .iter()
            .filter(|(kind, _)| *kind == "write" || *kind == "signal")
            .map(|(_, d)| *d)
            .collect();
        assert!(
            control.len() >= 3,
            "expected >=3 control ops, got {control:?}"
        );
        for d in &control {
            assert_eq!(
                *d, deadlines.control,
                "each control op must draw a fresh full control deadline even \
                 after the session has aged past one op deadline (a shared \
                 absolute deadline would have shrunk this)"
            );
        }
        // ...and the LAST control op's deadline equals the FIRST: no shared
        // cumulative budget that decays as the session ages.
        assert_eq!(
            control.first(),
            control.last(),
            "a later op must not inherit a shrunk remaining-budget deadline"
        );

        // The long-poll Wait draws its own fresh poll-based deadline, computed
        // from THIS op's timeout_ms - not from session start, and not shrunk by
        // the accumulated session age.
        let wait = recorded
            .iter()
            .find(|(kind, _)| *kind == "wait")
            .map(|(_, d)| *d)
            .expect("wait op recorded a deadline");
        assert_eq!(wait, Duration::from_millis(1_000) + deadlines.poll_slack);
    }

    #[test]
    fn duplicate_write_at_same_offset_is_idempotent_without_reissuing() {
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 3,
            next_offset: 3,
            backpressured: false,
            stdin_closed: false,
        });
        let _ = send_op(&tx, write_op(0, b"abc")).expect("write ok");
        // A retry at the SAME offset returns the cached result and must NOT
        // call the transport again.
        let resp = send_op(&tx, write_op(0, b"abc")).expect("retry ok");
        assert!(matches!(resp, ExecOpResponse::WriteStdin(_)));
        assert_eq!(
            shared.write_calls.load(Ordering::SeqCst),
            1,
            "idempotent retry must not reissue the write"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn write_at_wrong_offset_is_rejected_as_offset_mismatch() {
        let (tx, worker, _shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 3,
            next_offset: 3,
            backpressured: false,
            stdin_closed: false,
        });
        let err = send_op(&tx, write_op(99, b"abc")).expect_err("offset mismatch");
        assert_eq!(err, ExecOpError::Guest(GuestOpError::OffsetMismatch));
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn zero_accepted_write_surfaces_backpressure() {
        let (tx, worker, _shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: true,
            stdin_closed: false,
        });
        let resp = send_op(&tx, write_op(0, b"abc")).expect("write ok");
        match resp {
            ExecOpResponse::WriteStdin(result) => {
                assert_eq!(result.accepted_len, 0);
                assert!(result.backpressured);
            }
            other => panic!("expected WriteStdin, got {other:?}"),
        }
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn zero_progress_write_is_not_replay_cached() {
        // A zero-accepted (backpressured) write must NOT be replay-cached: its
        // offset never advances, so a retry at the same offset must re-issue to
        // the guest (observing recovered budget), not return a stale cached zero
        // forever.
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: true,
            stdin_closed: false,
        });
        let _ = send_op(&tx, write_op(0, b"abc")).expect("write ok");
        let _ = send_op(&tx, write_op(0, b"abc")).expect("retry ok");
        assert_eq!(
            shared.write_calls.load(Ordering::SeqCst),
            2,
            "zero-progress write must re-issue on retry, not serve a cached zero"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn control_op_retry_with_same_op_id_replays_cached_ack() {
        // A Signal retried with the SAME client opId must replay the original
        // ack WITHOUT re-delivering the signal to the guest (idempotency).
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: false,
            stdin_closed: false,
        });
        let sig = ExecOp::Signal(ExecSignalArgs {
            session: "h".to_owned(),
            signo: 2,
            op_id: 7,
        });
        let r1 = send_op(&tx, sig.clone()).expect("signal ok");
        assert!(matches!(r1, ExecOpResponse::Signal(_)));
        let r2 = send_op(&tx, sig).expect("signal retry ok");
        assert!(matches!(r2, ExecOpResponse::Signal(_)));
        assert_eq!(
            shared.signal_calls.load(Ordering::SeqCst),
            1,
            "retried Signal with same opId must not re-deliver"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn control_op_without_op_id_is_never_deduped() {
        // opId == 0 means "no dedup": two Signals with op_id 0 both deliver.
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: false,
            stdin_closed: false,
        });
        let sig = ExecOp::Signal(ExecSignalArgs {
            session: "h".to_owned(),
            signo: 2,
            op_id: 0,
        });
        let _ = send_op(&tx, sig.clone()).expect("signal ok");
        let _ = send_op(&tx, sig).expect("signal again");
        assert_eq!(
            shared.signal_calls.load(Ordering::SeqCst),
            2,
            "opId 0 must never be deduped"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn resize_retry_with_same_op_id_replays_cached_ack() {
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: false,
            stdin_closed: false,
        });
        let resize = ExecOp::Resize(ExecResizeArgs {
            session: "h".to_owned(),
            rows: 40,
            cols: 120,
            op_id: 11,
        });
        let _ = send_op(&tx, resize.clone()).expect("resize ok");
        let _ = send_op(&tx, resize).expect("resize retry ok");
        assert_eq!(
            shared.resize_calls.load(Ordering::SeqCst),
            1,
            "retried Resize with same opId must not re-deliver"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn oversized_chunk_is_rejected_before_the_transport() {
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: false,
            stdin_closed: false,
        });
        let big = vec![0_u8; (EXEC_MAX_CHUNK_BYTES + 1) as usize];
        let err = send_op(&tx, write_op(0, &big)).expect_err("too big");
        assert_eq!(err, ExecOpError::Guest(GuestOpError::MaxChunkExceeded));
        assert_eq!(
            shared.write_calls.load(Ordering::SeqCst),
            0,
            "oversized chunk must never reach the transport"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn close_is_idempotent_and_issued_once() {
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: false,
            stdin_closed: false,
        });
        let close = ExecOp::Close(ExecCloseArgs {
            session: "h".to_owned(),
        });
        let r1 = send_op(&tx, close.clone()).expect("close ok");
        assert!(matches!(r1, ExecOpResponse::Close(_)));
        let r2 = send_op(&tx, close).expect("close idempotent");
        assert!(matches!(r2, ExecOpResponse::Close(_)));
        assert_eq!(
            shared.close_calls.load(Ordering::SeqCst),
            1,
            "second close must be a no-op on the transport"
        );
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn stdout_and_stderr_reads_are_separated_with_flags_passed_through() {
        let shared = Arc::new(FakeShared::default());
        let shared_for_builder = Arc::clone(&shared);
        let mut stdout_reads = VecDeque::new();
        stdout_reads.push_back(ReadOutputOutcome {
            data: b"OUT".to_vec(),
            next_offset: 3,
            eof: false,
            dropped_bytes: 7,
            truncated: true,
            timed_out: false,
        });
        let mut stderr_reads = VecDeque::new();
        stderr_reads.push_back(ReadOutputOutcome {
            data: b"ERR".to_vec(),
            next_offset: 3,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        });
        let builder: Builder = Box::new(move || {
            established(Arc::new(FakeClient {
                alive: Arc::new(AtomicUsize::new(1)),
                shared: shared_for_builder,
                write_outcome: WriteStdinOutcome {
                    accepted_len: 0,
                    next_offset: 0,
                    backpressured: false,
                    stdin_closed: false,
                },
                stdout_reads: Mutex::new(stdout_reads),
                stderr_reads: Mutex::new(stderr_reads),
                waits: Mutex::new(VecDeque::new()),
                read_gate: None,
            }))
        });
        let (tx, worker, reply) = start_worker(FakeConnector::ok(builder));
        assert!(reply.is_ok());

        let out = send_op(
            &tx,
            ExecOp::ReadOutput(ExecReadOutputArgs {
                session: "h".to_owned(),
                stream: ExecStream::Stdout,
                offset: 0,
                max_len: 1024,
                wait: false,
                timeout_ms: 0,
            }),
        )
        .expect("stdout read");
        match out {
            ExecOpResponse::ReadOutput(result) => {
                assert_eq!(base64_codec::decode(&result.data_base64).unwrap(), b"OUT");
                assert_eq!(result.dropped_bytes, 7);
                assert!(result.truncated);
            }
            other => panic!("expected ReadOutput, got {other:?}"),
        }

        let err = send_op(
            &tx,
            ExecOp::ReadOutput(ExecReadOutputArgs {
                session: "h".to_owned(),
                stream: ExecStream::Stderr,
                offset: 0,
                max_len: 1024,
                wait: false,
                timeout_ms: 0,
            }),
        )
        .expect("stderr read");
        match err {
            ExecOpResponse::ReadOutput(result) => {
                assert_eq!(base64_codec::decode(&result.data_base64).unwrap(), b"ERR");
            }
            other => panic!("expected ReadOutput, got {other:?}"),
        }

        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn resize_is_serviced_inline() {
        let (tx, worker, shared) = backpressure_worker(WriteStdinOutcome {
            accepted_len: 0,
            next_offset: 0,
            backpressured: false,
            stdin_closed: false,
        });
        let resp = send_op(
            &tx,
            ExecOp::Resize(ExecResizeArgs {
                session: "h".to_owned(),
                rows: 40,
                cols: 120,
                op_id: 0,
            }),
        )
        .expect("resize ok");
        assert!(matches!(resp, ExecOpResponse::Resize(_)));
        assert_eq!(shared.resize_calls.load(Ordering::SeqCst), 1);
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn wait_timeout_then_terminal_keeps_polling() {
        let shared = Arc::new(FakeShared::default());
        let shared_for_builder = Arc::clone(&shared);
        let mut waits = VecDeque::new();
        waits.push_back(WaitOutcome {
            running: true,
            terminal: None,
        });
        waits.push_back(WaitOutcome {
            running: false,
            terminal: Some(TerminalKind::Exited(7)),
        });
        let builder: Builder = Box::new(move || {
            established(Arc::new(FakeClient {
                alive: Arc::new(AtomicUsize::new(1)),
                shared: shared_for_builder,
                write_outcome: WriteStdinOutcome {
                    accepted_len: 0,
                    next_offset: 0,
                    backpressured: false,
                    stdin_closed: false,
                },
                stdout_reads: Mutex::new(VecDeque::new()),
                stderr_reads: Mutex::new(VecDeque::new()),
                waits: Mutex::new(waits),
                read_gate: None,
            }))
        });
        let (tx, worker, reply) = start_worker(FakeConnector::ok(builder));
        assert!(reply.is_ok());

        let wait_op = ExecOp::Wait(ExecWaitArgs {
            session: "h".to_owned(),
            timeout_ms: 50,
        });
        let first = send_op(&tx, wait_op.clone()).expect("first wait");
        match first {
            ExecOpResponse::Wait(result) => {
                assert!(result.running);
                assert!(result.terminal_status.is_none());
            }
            other => panic!("expected Wait, got {other:?}"),
        }
        let second = send_op(&tx, wait_op).expect("second wait");
        match second {
            ExecOpResponse::Wait(result) => {
                assert_eq!(
                    result.terminal_status,
                    Some(ExecTerminalStatus::Exited { code: 7 })
                );
            }
            other => panic!("expected Wait, got {other:?}"),
        }
        drop(tx);
        worker.join().unwrap();
    }

    // ---- (b) session-table adversarial ----------------------------------------

    fn caps(global: usize, per_uid: usize, per_vm: usize) -> ExecSessionCaps {
        ExecSessionCaps {
            global,
            per_uid,
            per_vm,
            start_burst: 1024,
            start_window: Duration::from_secs(10),
        }
    }

    #[test]
    fn per_vm_cap_is_enforced_and_released_on_drop() {
        let table = Arc::new(SessionTable::new(caps(8, 8, 1)));
        let slot = table.reserve(1, "work").expect("first slot");
        let err = table.reserve(1, "work").expect_err("second blocked");
        assert_eq!(err, SessionReserveError::PerVmCap);
        assert_eq!(table.len(), 1);
        drop(slot);
        assert_eq!(table.len(), 0);
        // The slot released, so a fresh reserve succeeds again.
        let _slot = table.reserve(1, "work").expect("reserve after release");
    }

    #[test]
    fn per_uid_and_global_caps_are_enforced() {
        // per-uid cap (global high enough not to mask it).
        let uid_table = Arc::new(SessionTable::new(caps(8, 2, 8)));
        let _a = uid_table.reserve(5, "va").expect("a");
        let _b = uid_table.reserve(5, "vb").expect("b");
        let uid_err = uid_table.reserve(5, "vc").expect_err("per-uid");
        assert_eq!(uid_err, SessionReserveError::PerUidCap);
        // A different uid is unaffected by another uid's per-uid cap.
        let _other = uid_table.reserve(6, "vd").expect("other uid ok");

        // global cap, checked before per-uid: two live sessions exhaust it
        // even across distinct uids/vms.
        let global_table = Arc::new(SessionTable::new(caps(2, 8, 8)));
        let _x = global_table.reserve(5, "va").expect("x");
        let _y = global_table.reserve(6, "vb").expect("y");
        let global_err = global_table.reserve(7, "vc").expect_err("global");
        assert_eq!(global_err, SessionReserveError::GlobalCap);
    }

    #[test]
    fn handle_collision_and_exhaustion_fail_closed_without_leaking_a_slot() {
        let table = Arc::new(SessionTable::new(caps(8, 8, 8)));
        // A generator that always returns the SAME bytes: the first reserve
        // succeeds, the second collides every retry → HandleExhausted.
        let fixed = [7_u8; 16];
        let _first = table
            .reserve_with(1, "work", || Some(fixed))
            .expect("first mints handle");
        let collide = table
            .reserve_with(1, "work", || Some(fixed))
            .expect_err("collision");
        assert_eq!(collide, SessionReserveError::HandleExhausted);
        // A generator that cannot produce entropy fails closed too.
        let exhausted = table
            .reserve_with(2, "work", || None)
            .expect_err("no entropy");
        assert_eq!(exhausted, SessionReserveError::HandleExhausted);
        // Neither failure leaked a slot: only the first reserve is live.
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn owned_by_binds_handle_to_reserving_uid() {
        let table = Arc::new(SessionTable::new(caps(8, 8, 8)));
        let slot = table.reserve(7, "work").expect("slot");
        let handle = slot.handle().to_owned();
        assert!(table.owned_by(&handle, 7));
        assert!(!table.owned_by(&handle, 8), "wrong peer uid is rejected");
        assert!(!table.owned_by("deadbeef", 7), "unknown handle is rejected");
        drop(slot);
        assert!(
            !table.owned_by(&handle, 7),
            "released handle is not reusable / lookupable"
        );
    }

    #[test]
    fn session_slot_debug_redacts_the_handle() {
        // A stray `{:?}` on the reserved-slot guard must never leak the
        // unguessable session handle; only uid / vm / released are observable.
        let table = Arc::new(SessionTable::new(caps(8, 8, 8)));
        let slot = table.reserve(7, "corp-vm").expect("slot");
        let handle = slot.handle().to_owned();
        let rendered = format!("{slot:?}");
        assert!(
            !rendered.contains(&handle),
            "SessionSlot Debug leaked the handle {handle}: {rendered}"
        );
        assert!(rendered.contains("corp-vm"), "vm name is observable");
        assert!(rendered.contains("uid"), "uid is observable");
    }

    #[test]
    fn read_output_outcome_debug_redacts_output_bytes() {
        // A stray `{:?}` on a `ReadOutput` outcome must never render
        // the guest output bytes; only the length + framing flags are shown.
        const SECRET_OUTPUT: &[u8] = b"SENTINEL_STDOUT_rood";
        let outcome = ReadOutputOutcome {
            data: SECRET_OUTPUT.to_vec(),
            next_offset: 20,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        };
        let rendered = format!("{outcome:?}");
        assert!(
            !rendered.contains("SENTINEL_STDOUT_rood"),
            "ReadOutputOutcome Debug leaked output bytes: {rendered}"
        );
        assert!(rendered.contains("data_len"), "output length is observable");
    }

    // ---- (j) fake-clock rate limit --------------------------------------------

    #[test]
    fn start_rate_limit_uses_the_clock_window() {
        let clock = FakeClock::new();
        let table = Arc::new(SessionTable::with_clock(
            ExecSessionCaps {
                global: 64,
                per_uid: 64,
                per_vm: 64,
                start_burst: 2,
                start_window: Duration::from_secs(10),
            },
            Arc::clone(&clock) as Arc<dyn Clock>,
        ));
        // Two starts in the window are allowed; the third is rate limited.
        let _a = table.reserve(1, "va").expect("start 1");
        let _b = table.reserve(1, "vb").expect("start 2");
        let limited = table.reserve(1, "vc").expect_err("rate limited");
        assert_eq!(limited, SessionReserveError::RateLimited);

        // Advance past the window: the sliding window forgets the old starts.
        clock.advance(Duration::from_secs(11));
        let _c = table.reserve(1, "vd").expect("start after window");
    }

    // ---- (f) terminal-cleanup reaper --------------------------------

    #[test]
    fn terminal_reaper_is_not_due_before_a_terminal_observation() {
        let clock = FakeClock::new();
        let reaper = TerminalReaper::new(
            Arc::clone(&clock) as Arc<dyn Clock>,
            Duration::from_secs(10),
        );
        assert!(!reaper.is_terminal());
        // Time passing without a terminal observation never makes it due.
        clock.advance(Duration::from_secs(3600));
        assert!(!reaper.due());
    }

    #[test]
    fn terminal_reaper_becomes_due_only_after_the_ttl_elapses() {
        let clock = FakeClock::new();
        let reaper = TerminalReaper::new(
            Arc::clone(&clock) as Arc<dyn Clock>,
            Duration::from_secs(10),
        );
        assert!(reaper.mark_terminal(), "first mark is the transition");
        assert!(reaper.is_terminal());
        // Before the TTL: not due.
        clock.advance(Duration::from_secs(9));
        assert!(!reaper.due());
        // A second mark must NOT move the deadline forward.
        assert!(!reaper.mark_terminal(), "mark is idempotent");
        clock.advance(Duration::from_secs(1));
        assert!(reaper.due(), "due once the TTL elapses from first terminal");
    }

    /// A recording owner reaper for the worker integration test.
    struct RecordingReaper {
        reaped: Arc<AtomicUsize>,
    }

    impl OwnerReaper for RecordingReaper {
        fn reap(&self) {
            self.reaped.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn worker_reaps_a_stalled_owner_after_the_command_goes_terminal() {
        let reaped = Arc::new(AtomicUsize::new(0));
        let shared = Arc::new(FakeShared::default());
        let alive = Arc::new(AtomicUsize::new(0));
        let alive_for_builder = Arc::clone(&alive);
        let shared_for_builder = Arc::clone(&shared);
        let builder: Builder = Box::new(move || {
            alive_for_builder.fetch_add(1, Ordering::SeqCst);
            let mut waits = VecDeque::new();
            waits.push_back(WaitOutcome {
                running: false,
                terminal: Some(TerminalKind::Exited(0)),
            });
            established(Arc::new(FakeClient {
                alive: alive_for_builder,
                shared: shared_for_builder,
                write_outcome: WriteStdinOutcome {
                    accepted_len: 0,
                    next_offset: 0,
                    backpressured: false,
                    stdin_closed: false,
                },
                stdout_reads: Mutex::new(VecDeque::new()),
                stderr_reads: Mutex::new(VecDeque::new()),
                waits: Mutex::new(waits),
                read_gate: None,
            }))
        });

        let (control_tx, control_rx) = mpsc::channel(16);
        let (establish_tx, establish_rx) = oneshot::channel();
        let reaped_for_worker = Arc::clone(&reaped);
        // A tiny terminal TTL so the test does not sleep long; the DECISION is
        // covered by the fake-clock unit tests above.
        let worker = spawn_session_worker(WorkerSpawn {
            connector: FakeConnector::ok(builder),
            spec: spec(),
            deadlines: ExecOpDeadlines::default(),
            establish_tx,
            control_rx,
            terminal_ttl: Duration::from_millis(50),
            clock: Arc::new(SystemClock),
            owner_reaper: Arc::new(RecordingReaper {
                reaped: reaped_for_worker,
            }),
        });
        establish_rx
            .blocking_recv()
            .expect("establish")
            .expect("ok");

        // Drive a Wait that returns terminal. The owner then STALLS (never drops
        // the channel), modelling a stuck CLI that pins the slot.
        let response = send_op(
            &control_tx,
            ExecOp::Wait(ExecWaitArgs {
                session: "h".to_owned(),
                timeout_ms: 0,
            }),
        )
        .expect("wait ok");
        assert!(matches!(response, ExecOpResponse::Wait(_)));

        // The reaper must fire after the TTL even though the owner never closed.
        let mut reaped_seen = false;
        for _ in 0..100 {
            if reaped.load(Ordering::SeqCst) > 0 {
                reaped_seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            reaped_seen,
            "terminal-cleanup reaper did not fire for a stalled owner"
        );

        drop(control_tx);
        worker.join().expect("worker joins");
    }
}

pub fn gate_capabilities(
    capabilities: &[EnumOrUnknown<pb::GuestCapability>],
    tty: bool,
) -> Result<NegotiatedCaps, ExecEstablishError> {
    let advertises = |cap: pb::GuestCapability| {
        capabilities
            .iter()
            .filter_map(|value| value.enum_value().ok())
            .any(|value| value == cap)
    };
    // The guest authenticated but advertises no attached-exec capability: exec
    // is disabled or not built in (NOT old-generation - that is a connect-time
    // failure). Fail closed to the capability slug, whose remediation points at
    // `guest.exec.enable = true`.
    if !advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED) {
        return Err(ExecEstablishError::Capability);
    }
    // Every reachable exec session streams stdout/stderr back via ReadOutput, so
    // a guest that does not advertise EXEC_LOGS cannot serve a session at all.
    // Fail fast rather than establishing a session that can never deliver
    // output. A real exec-enabled guestd always advertises EXEC_LOGS alongside
    // EXEC_ATTACHED, so this never rejects a correctly-configured guest.
    if !advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS) {
        return Err(ExecEstablishError::Capability);
    }
    if !advertises(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS) {
        return Err(ExecEstablishError::Capability);
    }
    if tty
        && (!advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY)
            || !advertises(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE))
    {
        return Err(ExecEstablishError::Capability);
    }
    Ok(NegotiatedCaps {
        tty,
        signals: advertises(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        tty_resize: advertises(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
        output: advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
    })
}

pub fn build_exec_create_request(vm_id: &str, spec: &ExecStartSpec) -> pb::ExecCreateRequest {
    let mut metadata = pb::RequestMetadata::new();
    metadata.vm_id = vm_id.to_owned();
    metadata.request_id = spec
        .request_id
        .clone()
        .unwrap_or_else(|| "guest-control-exec".to_owned());
    metadata.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;

    let mut request = pb::ExecCreateRequest::new();
    request.metadata = MessageField::some(metadata);
    // The exec target user is host-fixed by guestd (the VM's workload user,
    // `--exec-user`), and guestd ignores the wire `user` field entirely, so a
    // client cannot select or escalate the target user. The daemon therefore
    // leaves `user` unset.
    request.argv = spec.argv.clone();
    request.cwd = spec.cwd.clone();
    request.env = spec
        .env
        .iter()
        .map(|(key, value)| {
            let mut var = pb::EnvVar::new();
            var.key = key.clone();
            var.value = value.clone();
            var
        })
        .collect();
    request.tty = spec.tty;
    // guestd accepts an open stdin only in interactive TTY mode
    // (`validate_and_authorize_tty`); both non-TTY validators
    // (`validate_and_authorize` / `_detached`) reject `stdin_open` as
    // `UnsupportedMode`. Mirror that contract: open stdin iff a PTY was
    // requested. Hardcoding `true` made every non-TTY `vm exec` (and every
    // detached exec) fail `ExecCreate` before the guest process could spawn.
    request.stdin_open = spec.tty;
    request.detached = spec.detached;
    if let Some((rows, cols)) = spec.term_size {
        let mut size = pb::TerminalSize::new();
        size.rows = rows;
        size.cols = cols;
        request.initial_terminal_size = MessageField::some(size);
    }
    let mut policy = pb::OutputPolicy::new();
    policy.max_chunk_bytes = d2b_contracts_control::public_wire::EXEC_MAX_CHUNK_BYTES;
    request.output_policy = MessageField::some(policy);
    request
}

pub fn ack_result(ack: &pb::ControlAck) -> Result<(), ExecOpError> {
    if let Some(error) = ack.error.as_ref()
        && !is_unspecified(error.kind)
    {
        return Err(map_guest_control_error(error));
    }
    Ok(())
}

pub fn terminal_from_state(
    state: pb::ExecState,
    status: Option<&pb::TerminalStatus>,
) -> Option<TerminalKind> {
    match state {
        pb::ExecState::EXEC_STATE_EXITED => {
            match status.and_then(|status| status.outcome.as_ref()) {
                Some(pb::terminal_status::Outcome::ExitCode(code)) => {
                    Some(TerminalKind::Exited(*code))
                }
                Some(pb::terminal_status::Outcome::StatusCode(code)) => {
                    Some(TerminalKind::Exited(*code))
                }
                // EXITED without a WIFEXITED code is a protocol violation, not a
                // synthesized success.
                _ => Some(TerminalKind::Error("protocol-error")),
            }
        }
        pb::ExecState::EXEC_STATE_SIGNALED => {
            match status.and_then(|status| status.outcome.as_ref()) {
                Some(pb::terminal_status::Outcome::Signal(signal)) => {
                    Some(TerminalKind::Signaled(*signal))
                }
                _ => Some(TerminalKind::Error("protocol-error")),
            }
        }
        pb::ExecState::EXEC_STATE_CANCELLED | pb::ExecState::EXEC_STATE_SLOW_CONSUMER_CANCELLED => {
            Some(TerminalKind::Error("cancelled"))
        }
        pb::ExecState::EXEC_STATE_LOST_GUESTD => Some(TerminalKind::Error("lost-guestd")),
        pb::ExecState::EXEC_STATE_REAPED => Some(TerminalKind::Error("reaped")),
        pb::ExecState::EXEC_STATE_PROTOCOL_ERROR => Some(TerminalKind::Error("protocol-error")),
        _ => None,
    }
}

pub fn is_unspecified(kind: EnumOrUnknown<pb::GuestControlErrorKind>) -> bool {
    matches!(
        kind.enum_value(),
        Ok(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_UNSPECIFIED)
    )
}

pub fn map_guest_control_error(error: &pb::GuestControlError) -> ExecOpError {
    use pb::GuestControlErrorKind as K;
    match error.kind.enum_value() {
        Ok(K::GUEST_CONTROL_ERROR_KIND_AUTH_FAILED) => ExecOpError::Auth,
        Ok(K::GUEST_CONTROL_ERROR_KIND_STALE_SESSION) => ExecOpError::StaleSession,
        Ok(K::GUEST_CONTROL_ERROR_KIND_TRANSPORT_UNREACHABLE) => ExecOpError::Transport,
        Ok(K::GUEST_CONTROL_ERROR_KIND_GUEST_CONTROL_UNAVAILABLE_OLD_GENERATION) => {
            ExecOpError::OldGeneration
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED) => ExecOpError::Capability,
        Ok(K::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR) => {
            ExecOpError::Guest(GuestOpError::Protocol)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_MAX_CHUNK_EXCEEDED) => {
            ExecOpError::Guest(GuestOpError::MaxChunkExceeded)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_STDIN_BACKPRESSURE) => {
            ExecOpError::Guest(GuestOpError::StdinBackpressure)
        }
        Ok(
            K::GUEST_CONTROL_ERROR_KIND_STDIN_CLOSED
            | K::GUEST_CONTROL_ERROR_KIND_STDIN_CLOSED_BY_PROCESS,
        ) => ExecOpError::Guest(GuestOpError::StdinClosed),
        Ok(K::GUEST_CONTROL_ERROR_KIND_STDIN_NOT_OPEN) => {
            ExecOpError::Guest(GuestOpError::StdinNotOpen)
        }
        Ok(
            K::GUEST_CONTROL_ERROR_KIND_STDIN_OFFSET_MISMATCH
            | K::GUEST_CONTROL_ERROR_KIND_OFFSET_EXPIRED
            | K::GUEST_CONTROL_ERROR_KIND_OFFSET_IN_FUTURE
            | K::GUEST_CONTROL_ERROR_KIND_OFFSET_EXHAUSTED,
        ) => ExecOpError::Guest(GuestOpError::OffsetMismatch),
        Ok(K::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND) => {
            ExecOpError::Guest(GuestOpError::ExecNotFound)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_EXEC_ALREADY_EXITED) => {
            ExecOpError::Guest(GuestOpError::ExecAlreadyExited)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_EXEC_EXPIRED) => {
            ExecOpError::Guest(GuestOpError::ExecExpired)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_CONTROL_SEQ_MISMATCH) => {
            ExecOpError::Guest(GuestOpError::ControlSeqMismatch)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_RATE_LIMITED) => {
            ExecOpError::Guest(GuestOpError::RateLimited)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_INVALID_PROGRAM) => {
            ExecOpError::Guest(GuestOpError::InvalidProgram)
        }
        _ => ExecOpError::Guest(GuestOpError::Other),
    }
}

pub fn map_op_health_error(error: GuestControlHealthError) -> ExecOpError {
    match error {
        GuestControlHealthError::TransportIo
        | GuestControlHealthError::Ttrpc
        | GuestControlHealthError::Signer => ExecOpError::Transport,
        GuestControlHealthError::Timeout => ExecOpError::Timeout,
        GuestControlHealthError::AuthFailed => ExecOpError::Auth,
        GuestControlHealthError::StaleSession => ExecOpError::StaleSession,
        GuestControlHealthError::Protocol => ExecOpError::Protocol,
    }
}

pub fn map_op_health_error_for_establish(error: GuestControlHealthError) -> ExecEstablishError {
    op_to_establish(map_op_health_error(error))
}

pub fn map_establish_health_error(error: GuestControlHealthError) -> ExecEstablishError {
    match error {
        GuestControlHealthError::TransportIo
        | GuestControlHealthError::Ttrpc
        | GuestControlHealthError::Signer => ExecEstablishError::Transport,
        GuestControlHealthError::Timeout => ExecEstablishError::Timeout,
        GuestControlHealthError::AuthFailed | GuestControlHealthError::StaleSession => {
            ExecEstablishError::Auth
        }
        GuestControlHealthError::Protocol => ExecEstablishError::Protocol,
    }
}

pub fn op_to_establish(error: ExecOpError) -> ExecEstablishError {
    match error {
        ExecOpError::Transport => ExecEstablishError::Transport,
        ExecOpError::Auth => ExecEstablishError::Auth,
        ExecOpError::StaleSession => ExecEstablishError::Auth,
        ExecOpError::Protocol => ExecEstablishError::Protocol,
        ExecOpError::Timeout => ExecEstablishError::Timeout,
        ExecOpError::OldGeneration => ExecEstablishError::OldGeneration,
        ExecOpError::Capability => ExecEstablishError::Capability,
        ExecOpError::DetachedUnavailable => ExecEstablishError::Capability,
        ExecOpError::Guest(inner) => ExecEstablishError::Guest(inner),
    }
}

#[cfg(test)]
mod exec_protocol_tests {
    use super::*;
    use d2b_contracts_control::guest_proto as pb;

    fn cap(value: pb::GuestCapability) -> EnumOrUnknown<pb::GuestCapability> {
        EnumOrUnknown::new(value)
    }

    /// The full capability set a TTY exec needs.
    fn full_tty_caps() -> Vec<EnumOrUnknown<pb::GuestCapability>> {
        vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY),
            cap(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
        ]
    }

    #[test]
    fn no_exec_capability_is_capability_unavailable_after_auth() {
        // A guest advertising only health/capabilities (no exec) has reached
        // this gate AFTER authenticating, so it is up and reachable - exec is
        // disabled or not built in, NOT a genuine old generation (that is a
        // connect-time failure). Fail closed to the capability slug (exit 70,
        // NO SSH fallback) whose remediation points at `guest.exec.enable`.
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_HEALTH),
            cap(pb::GuestCapability::GUEST_CAPABILITY_CAPABILITIES),
        ];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::Capability)
        );
        assert_eq!(
            gate_capabilities(&caps, true),
            Err(ExecEstablishError::Capability)
        );
    }

    #[test]
    fn exec_without_output_capability_is_capability_unavailable() {
        // EXEC_ATTACHED without EXEC_LOGS: every reachable session must stream
        // output, so the host fails closed rather than establishing a session
        // that can never deliver stdout/stderr.
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        ];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::Capability)
        );
        assert_eq!(
            gate_capabilities(&caps, true),
            Err(ExecEstablishError::Capability)
        );
    }

    #[test]
    fn exec_without_signals_is_capability_unavailable() {
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
        ];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::Capability)
        );
    }

    #[test]
    fn non_tty_session_succeeds_without_tty_caps() {
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        ];
        let negotiated = gate_capabilities(&caps, false).expect("non-tty session is allowed");
        assert_eq!(
            negotiated,
            NegotiatedCaps {
                tty: false,
                signals: true,
                tty_resize: false,
                output: true,
            }
        );
    }

    #[test]
    fn negotiated_caps_reflect_output_and_resize_advertisements() {
        // The cap snapshot used for per-op gating reflects exactly what the
        // guest advertised: ExecLogs → output, TtyResize → tty_resize.
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY),
            cap(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
        ];
        let negotiated = gate_capabilities(&caps, true).expect("full tty caps allowed");
        assert_eq!(
            negotiated,
            NegotiatedCaps {
                tty: true,
                signals: true,
                tty_resize: true,
                output: true,
            }
        );
    }

    #[test]
    fn tty_session_requires_exec_tty_and_tty_resize() {
        // Missing EXEC_TTY.
        let no_exec_tty = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
        ];
        assert_eq!(
            gate_capabilities(&no_exec_tty, true),
            Err(ExecEstablishError::Capability)
        );
        // Missing TTY_RESIZE.
        let no_resize = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY),
        ];
        assert_eq!(
            gate_capabilities(&no_resize, true),
            Err(ExecEstablishError::Capability)
        );
        // A non-tty session does not need the tty caps even when absent.
        assert!(gate_capabilities(&no_exec_tty, false).is_ok());
    }

    #[test]
    fn full_capability_set_passes_for_tty_and_non_tty() {
        assert!(gate_capabilities(&full_tty_caps(), true).is_ok());
        assert!(gate_capabilities(&full_tty_caps(), false).is_ok());
    }
}
