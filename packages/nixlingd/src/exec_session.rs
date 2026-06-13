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

/// Outcome of a `ReadOutput` transport call. `Debug` is redacted so a stray
/// `{:?}` can never leak the guest output bytes (WR12); only the length and the
/// framing flags are observable.
#[derive(Clone, PartialEq, Eq)]
pub struct ReadOutputOutcome {
    pub data: Vec<u8>,
    pub next_offset: u64,
    pub eof: bool,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
}

impl std::fmt::Debug for ReadOutputOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadOutputOutcome")
            .field("data_len", &self.data.len())
            .field("next_offset", &self.next_offset)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
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

/// Establishment spec resolved from a validated [`ExecOp::Start`]. `Debug` is
/// redacted so a stray `{:?}` can never leak argv / env keys+values / cwd
/// (WR12).
#[derive(Clone, PartialEq, Eq)]
pub struct ExecStartSpec {
    pub vm: String,
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
/// each proxied op can be gated fail-closed BEFORE it reaches the guest (WR8/
/// F6). A guest that did not advertise the cap an op needs (or a non-tty
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
    /// All capabilities present — used by tests that exercise the happy path.
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

/// Owner-socket teardown seam for the terminal-cleanup reaper (F5/WR13).
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

/// Default terminal-cleanup grace (WR13): after the guest command goes
/// terminal, a stalled owner that never closes its connection is reaped after
/// this long so it cannot pin a session slot indefinitely. Generous enough for
/// a well-behaved CLI to read the terminal status and close first. The reaper
/// never kills a LIVE command — cleanup only arms once `Wait` returns terminal.
pub const EXEC_TERMINAL_CLEANUP_TTL: Duration = Duration::from_secs(10);

/// Records when the guest command first went terminal and decides — against an
/// injected [`Clock`] — whether the terminal-cleanup TTL has since elapsed.
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
/// runtime, and drops every client clone — prompting the guest teardown.
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
        control_replay: std::collections::VecDeque::new(),
        caps,
    };

    while let Some(WorkerCommand { op, reply }) = control_rx.recv().await {
        match op {
            ExecOp::ReadOutput(_) | ExecOp::Wait(_) => {
                // Fail closed before spawning a `ReadOutput` long-poll if the
                // guest never advertised the output (`ExecLogs`) cap (WR8/F6).
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
                    // then arm the terminal-cleanup reaper (WR13/F5). The reaper
                    // only releases the slot AFTER the command is terminal; it
                    // never kills a live command.
                    if is_wait {
                        if let Ok(ExecOpResponse::Wait(wait)) = &result {
                            if wait.terminal_status.is_some() && reaper.mark_terminal() {
                                arm_terminal_reap(reaper, owner_reaper);
                            }
                        }
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
    // `control_rx` closed → owner disconnected → drop `state` (and its sole
    // client reference). Any still-spawned long-poll is aborted when the
    // runtime is dropped at thread exit.
}

/// Arm the terminal-cleanup timer (F5): after the TTL elapses, if the command
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
    // Negotiated caps + session shape for fail-closed per-op gating (WR8/F6).
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
                // Only cache writes that made progress or closed stdin. A
                // zero-progress (backpressured) write must NOT be replay-cached:
                // its offset never advances, so caching it would pin the session
                // at perpetual backpressure even after the guest budget recovers
                // and the CLI retries the same offset (F3).
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
                // non-tty session or a guest missing the cap fails closed (F6).
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
/// `Debug` is redacted so a stray `{:?}` can never leak the unguessable
/// session handle capability token (WR12); only the leak-safe uid / vm /
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
// Tests (WR16 hermetic matrices: session-table adversarial, worker lifecycle
// + teardown, no-head-of-line concurrency, backpressure/offset/idempotency,
// and fake-clock rate limiting). All fakes are injected; no live transport.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nixling_ipc::public_wire::{
        ExecCloseArgs, ExecReadOutputArgs, ExecResizeArgs, ExecSignalArgs, ExecStream,
        ExecWaitArgs, ExecWriteStdinArgs,
    };

    #[test]
    fn exec_start_spec_debug_redacts_argv_env_cwd() {
        // WR12: a stray `{:?}` on the resolved establishment spec must never
        // leak argv, env keys/values, or cwd; only the VM name, shape, and
        // counts are observable.
        const SECRET_ARGV: &str = "SENTINEL_ARGV_dspc";
        const SECRET_KEY: &str = "SENTINEL_ENV_KEY_dspc";
        const SECRET_VAL: &str = "SENTINEL_ENV_VAL_dspc";
        const SECRET_CWD: &str = "SENTINEL_CWD_dspc";
        let spec = ExecStartSpec {
            vm: "corp-vm".to_owned(),
            argv: vec!["sh".to_owned(), SECRET_ARGV.to_owned()],
            tty: true,
            detached: false,
            env: vec![(SECRET_KEY.to_owned(), SECRET_VAL.to_owned())],
            cwd: Some(SECRET_CWD.to_owned()),
            term_size: Some((24, 80)),
        };
        let rendered = format!("{spec:?}");
        for secret in [SECRET_ARGV, SECRET_KEY, SECRET_VAL, SECRET_CWD] {
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
        signal_calls: AtomicUsize,
        resize_calls: AtomicUsize,
        read_calls: AtomicUsize,
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
    impl ExecGuestClient for FakeClient {
        async fn write_stdin(
            &self,
            _offset: u64,
            _data: Vec<u8>,
            _eof: bool,
            _timeout: Duration,
        ) -> Result<WriteStdinOutcome, ExecOpError> {
            self.shared.write_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.write_outcome.clone())
        }

        async fn read_output(
            &self,
            stream: OutputStreamSel,
            _offset: u64,
            _max_len: u64,
            _wait: bool,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<ReadOutputOutcome, ExecOpError> {
            self.shared.read_calls.fetch_add(1, Ordering::SeqCst);
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
            _timeout: Duration,
        ) -> Result<(), ExecOpError> {
            self.shared.signal_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn resize(
            &self,
            _control_seq: u64,
            _rows: u32,
            _cols: u32,
            _timeout: Duration,
        ) -> Result<(), ExecOpError> {
            self.shared.resize_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn wait(
            &self,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<WaitOutcome, ExecOpError> {
            let outcome = self.waits.lock().unwrap().pop_front();
            Ok(outcome.unwrap_or(WaitOutcome {
                running: false,
                terminal: Some(TerminalKind::Exited(0)),
            }))
        }

        async fn close_stdin(&self, _offset: u64, _timeout: Duration) -> Result<(), ExecOpError> {
            self.shared.close_calls.fetch_add(1, Ordering::SeqCst);
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
        async fn establish(&self, _spec: &ExecStartSpec) -> Result<Established, ExecEstablishError> {
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

    fn established_with_caps(client: Arc<dyn ExecGuestClient>, caps: NegotiatedCaps) -> Established {
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
        tx.blocking_send(WorkerCommand { op, reply: reply_tx })
            .expect("worker accepts command");
        reply_rx.blocking_recv().expect("worker replies")
    }

    fn start_worker(
        connector: Arc<dyn ExecGuestConnector>,
    ) -> (mpsc::Sender<WorkerCommand>, JoinHandle<()>, EstablishReply) {
        let (control_tx, control_rx) = mpsc::channel(16);
        let (establish_tx, establish_rx) = oneshot::channel();
        let worker = spawn_session_worker(WorkerSpawn {
            connector,
            spec: spec(),
            deadlines: ExecOpDeadlines::default(),
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
        let builder: Builder = Box::new(move || {
            alive_for_builder.fetch_add(1, Ordering::SeqCst);
            established(Arc::new(FakeClient {
                alive: alive_for_builder,
                shared,
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
        assert_eq!(alive.load(Ordering::SeqCst), 1, "client alive after establish");

        // Owner disconnects: drop the channel, join the worker.
        drop(control_tx);
        worker.join().expect("worker joins");
        assert_eq!(
            alive.load(Ordering::SeqCst),
            0,
            "client dropped on teardown (prompts guest close_connection)"
        );
    }

    #[test]
    fn dropping_channel_mid_long_poll_aborts_and_drops_client() {
        let alive = Arc::new(AtomicUsize::new(0));
        let alive_for_builder = Arc::clone(&alive);
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_builder = Arc::clone(&gate);
        let builder: Builder = Box::new(move || {
            alive_for_builder.fetch_add(1, Ordering::SeqCst);
            established(Arc::new(FakeClient {
                alive: alive_for_builder,
                shared: Arc::new(FakeShared::default()),
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
        let (control_tx, worker, reply) = start_worker(FakeConnector::ok(builder));
        assert!(reply.is_ok());
        (control_tx, worker, shared)
    }

    /// A worker whose session advertises exactly `caps`, for per-op fail-closed
    /// gating tests (F6). The fake client records whether each op reached it.
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
        // forever (F3).
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
        // ack WITHOUT re-delivering the signal to the guest (F3b idempotency).
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
        // WR12/G2: a stray `{:?}` on the reserved-slot guard must never leak the
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
        // WR12/G2: a stray `{:?}` on a `ReadOutput` outcome must never render
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

    // ---- (f) terminal-cleanup reaper (WR13/F5) --------------------------------

    #[test]
    fn terminal_reaper_is_not_due_before_a_terminal_observation() {
        let clock = FakeClock::new();
        let reaper = TerminalReaper::new(Arc::clone(&clock) as Arc<dyn Clock>, Duration::from_secs(10));
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
        establish_rx.blocking_recv().expect("establish").expect("ok");

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
        assert!(reaped_seen, "terminal-cleanup reaper did not fire for a stalled owner");

        drop(control_tx);
        worker.join().expect("worker joins");
    }
}
