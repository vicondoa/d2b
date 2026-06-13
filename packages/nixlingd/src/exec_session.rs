//! In-process exec session table + per-session worker for `nixling vm exec`.
//!
//! The daemon owns a long-lived, authenticated guest-control client per exec
//! session. The CLI drives the session through admin-gated `public.sock`
//! round trips (one frame per [`ExecOp`]); the daemon never opens a new
//! transport per op. A dedicated worker thread (current-thread tokio runtime)
//! owns the authenticated client, the guest `exec_id`, the authoritative stdin
//! offset, and the monotone control sequence; it is reached over a bounded
//! sync command channel.
//!
//! Concurrency contract (no head-of-line blocking): long-poll ops
//! (`ReadOutput`, `Wait`) are spawned onto the worker runtime so the worker
//! keeps servicing fast control ops (`WriteStdin`, `Signal`, `Resize`,
//! `Close`) while a poll is in flight. Fast ops are handled inline because
//! they mutate shared session state (stdin offset, control sequence).
//!
//! Teardown contract (non-detached): when the owner connection drops, the
//! command channel closes, the worker returns, the runtime is dropped, and
//! every clone of the authenticated client is dropped with it. That prompts
//! the guest's `close_connection` → W14 PTY hangup→grace→stop. The daemon
//! never issues `ExecCancel` for a non-detached session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use nixling_core::base64_codec;
use nixling_ipc::public_wire::{
    ExecCloseResult, ExecControlResult, ExecOp, ExecOpResponse, ExecReadOutputResult,
    ExecStartResult, ExecStream, ExecTerminalStatus, ExecWaitResult, ExecWriteStdinResult,
    EXEC_MAX_CHUNK_BYTES,
};
use tokio::sync::{mpsc, oneshot};

/// Output stream selector handed to the transport client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStreamSel {
    Stdout,
    Stderr,
}

/// Outcome of a `WriteStdin` transport call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteStdinOutcome {
    pub accepted_len: u64,
    pub next_offset: u64,
    pub backpressured: bool,
    pub stdin_closed: bool,
}

/// Outcome of a `ReadOutput` transport call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOutputOutcome {
    pub data: Vec<u8>,
    pub next_offset: u64,
    pub eof: bool,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
}

/// Terminal disposition of the guest command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalKind {
    Exited(i32),
    Signaled(u32),
    Error(&'static str),
}

/// Outcome of a `Wait` transport call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitOutcome {
    pub running: bool,
    pub terminal: Option<TerminalKind>,
}

/// Closed enum of per-op proxy failures. Each maps to a redaction-safe slug;
/// the daemon never attaches argv, env, output bytes, or a guest-supplied
/// string to the failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecOpError {
    Transport,
    Auth,
    Protocol,
    Timeout,
    OldGeneration,
    Capability,
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
    ControlSeqMismatch,
    RateLimited,
    MaxChunkExceeded,
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
            Self::ControlSeqMismatch => "control-seq-mismatch",
            Self::RateLimited => "rate-limited",
            Self::MaxChunkExceeded => "max-chunk-exceeded",
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
            Self::Protocol => "guest-control-protocol-error",
            Self::Timeout => "guest-control-timeout",
            Self::OldGeneration => "guest-control-unavailable-old-generation",
            Self::Capability => "guest-control-capability-unavailable",
            Self::Guest(inner) => inner.slug(),
        }
    }

    /// Closed-enum `error_kind` metric label (hard allowlist).
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

/// Per-op absolute deadlines. Each op draws a FRESH deadline (WR3): the
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

/// Establishment spec resolved from a validated [`ExecOp::Start`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecStartSpec {
    pub vm: String,
    pub argv: Vec<String>,
    pub tty: bool,
    pub detached: bool,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
    pub term_size: Option<(u32, u32)>,
}

/// Session info reported back to the owner on a successful establish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecSessionInfo {
    pub tty: bool,
    pub stdout_offset: u64,
    pub stderr_offset: u64,
}

/// A freshly established session: the authenticated client, the info echoed to
/// the owner, and the initial control sequence from `ExecCreate`.
pub struct Established {
    pub client: Arc<dyn ExecGuestClient>,
    pub info: ExecSessionInfo,
    pub control_seq: u64,
}

/// Transport seam: the authenticated guest-control client, one per session.
/// Production implementation wraps a ttRPC client; tests inject a fake.
#[async_trait]
pub trait ExecGuestClient: Send + Sync {
    async fn write_stdin(
        &self,
        offset: u64,
        data: Vec<u8>,
        eof: bool,
        timeout: Duration,
    ) -> Result<WriteStdinOutcome, ExecOpError>;

    async fn read_output(
        &self,
        stream: OutputStreamSel,
        offset: u64,
        max_len: u64,
        wait: bool,
        timeout_ms: u64,
        timeout: Duration,
    ) -> Result<ReadOutputOutcome, ExecOpError>;

    async fn signal(
        &self,
        control_seq: u64,
        signo: u32,
        timeout: Duration,
    ) -> Result<(), ExecOpError>;

    async fn resize(
        &self,
        control_seq: u64,
        rows: u32,
        cols: u32,
        timeout: Duration,
    ) -> Result<(), ExecOpError>;

    async fn wait(&self, timeout_ms: u64, timeout: Duration) -> Result<WaitOutcome, ExecOpError>;

    async fn close_stdin(&self, offset: u64, timeout: Duration) -> Result<(), ExecOpError>;
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

/// Spawn a session worker on its own OS thread with a dedicated current-thread
/// tokio runtime. The worker establishes the session, reports the result over
/// `establish_tx`, then services `WorkerCommand`s until the channel closes.
/// Dropping the sender (owner disconnect) returns the worker, drops the
/// runtime, and drops every client clone — prompting the guest teardown.
pub fn spawn_session_worker(
    connector: Arc<dyn ExecGuestConnector>,
    spec: ExecStartSpec,
    deadlines: ExecOpDeadlines,
    establish_tx: oneshot::Sender<EstablishReply>,
    control_rx: mpsc::Receiver<WorkerCommand>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("nixling-exec".to_owned())
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
    } = established;
    if establish_tx.send(Ok(info)).is_err() {
        // Owner vanished before the establish reply landed. Returning here
        // drops `client`, closing the guest connection promptly.
        return;
    }

    let mut state = WorkerState {
        client,
        deadlines,
        next_stdin_offset: 0,
        control_seq,
        last_write: None,
        stdin_closed: false,
    };

    while let Some(WorkerCommand { op, reply }) = control_rx.recv().await {
        match op {
            ExecOp::ReadOutput(_) | ExecOp::Wait(_) => {
                // Long-polls are spawned so the worker keeps servicing fast
                // control ops while a poll is in flight (no head-of-line
                // blocking). They touch no shared mutable session state.
                let client = Arc::clone(&state.client);
                let deadlines = state.deadlines;
                tokio::spawn(async move {
                    let result = run_long_poll(client.as_ref(), op, deadlines).await;
                    let _ = reply.send(result);
                });
            }
            other => {
                let result = state.handle_inline(other).await;
                let _ = reply.send(result);
            }
        }
    }
    // `control_rx` closed → owner disconnected → drop `state` (and its sole
    // client reference). Any still-spawned long-poll is aborted when the
    // runtime is dropped at thread exit.
}

struct WorkerState {
    client: Arc<dyn ExecGuestClient>,
    deadlines: ExecOpDeadlines,
    next_stdin_offset: u64,
    control_seq: u64,
    last_write: Option<(u64, ExecWriteStdinResult)>,
    stdin_closed: bool,
}

impl WorkerState {
    async fn handle_inline(&mut self, op: ExecOp) -> Result<ExecOpResponse, ExecOpError> {
        match op {
            ExecOp::WriteStdin(args) => {
                let data = base64_codec::decode(&args.chunk_base64)
                    .map_err(|_| ExecOpError::Protocol)?;
                if data.len() as u64 > EXEC_MAX_CHUNK_BYTES {
                    return Err(ExecOpError::Guest(GuestOpError::MaxChunkExceeded));
                }
                // Idempotent retry of the most recent write at the same offset.
                if let Some((offset, cached)) = &self.last_write {
                    if *offset == args.offset {
                        return Ok(ExecOpResponse::WriteStdin(cached.clone()));
                    }
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
                self.last_write = Some((args.offset, result.clone()));
                Ok(ExecOpResponse::WriteStdin(result))
            }
            ExecOp::Signal(args) => {
                self.control_seq = self.control_seq.saturating_add(1);
                let timeout = self.deadlines.control;
                self.client
                    .signal(self.control_seq, args.signo, timeout)
                    .await?;
                Ok(ExecOpResponse::Signal(ExecControlResult { delivered: true }))
            }
            ExecOp::Resize(args) => {
                self.control_seq = self.control_seq.saturating_add(1);
                let timeout = self.deadlines.control;
                self.client
                    .resize(self.control_seq, args.rows, args.cols, timeout)
                    .await?;
                Ok(ExecOpResponse::Resize(ExecControlResult { delivered: true }))
            }
            ExecOp::Close(_) => {
                if self.stdin_closed {
                    return Ok(ExecOpResponse::Close(ExecCloseResult { stdin_closed: true }));
                }
                let timeout = self.deadlines.control;
                // A close on a session whose stdin the process already shut is
                // idempotent: treat a not-open/closed guest error as success.
                match self.client.close_stdin(self.next_stdin_offset, timeout).await {
                    Ok(()) => {}
                    Err(ExecOpError::Guest(
                        GuestOpError::StdinClosed | GuestOpError::StdinNotOpen,
                    )) => {}
                    Err(error) => return Err(error),
                }
                self.stdin_closed = true;
                Ok(ExecOpResponse::Close(ExecCloseResult { stdin_closed: true }))
            }
            ExecOp::Start(_) => Err(ExecOpError::Protocol),
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
                .read_output(stream, args.offset, max_len, args.wait, timeout_ms, op_deadline)
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
// Session table (WR13): global / per-uid / per-vm caps + opaque handles.
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
        mut gen: impl FnMut() -> Option<[u8; 16]>,
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
        if inner
            .sessions
            .values()
            .filter(|meta| meta.vm == vm)
            .count()
            >= self.caps.per_vm
        {
            return Err(SessionReserveError::PerVmCap);
        }
        let mut handle = None;
        for _ in 0..HANDLE_RETRY_LIMIT {
            let candidate = match gen() {
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
pub struct SessionSlot {
    handle: String,
    uid: u32,
    vm: String,
    table: Arc<SessionTable>,
    released: bool,
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
