//! Attached, non-interactive guest exec runtime.
//!
//! This module implements the create-and-start attached exec subset: non-TTY,
//! non-detached, stdin-closed commands only. It is guest-local process
//! execution inside the VM. There is no host broker op, no CLI surface, no
//! readiness wiring, and no user-session-daemon participation.
//!
//! Security posture: attached exec is trusted-control-plane execution that
//! runs as the VM's **host-fixed workload user** (`ssh.user`), never root.
//! It is gated behind the host-owned per-VM `exec.enable` policy plus a
//! resolved workload user; the wire `user` field is never consulted, so a
//! guest-control client cannot target root or any other user. It is not a
//! sandbox and makes no CPU/memory/fd kernel-isolation claim; it bounds the
//! protocol/session resources it owns and applies a wall-clock runtime ceiling.
//!
//! Process spawning is abstracted behind [`ProcessSpawner`] so the lifecycle
//! state machine, output retention, and offset accounting can be exercised
//! without launching real processes.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::Notify,
    task::JoinHandle,
};

use nixling_ipc::guest_wire::GuestControlErrorKind as WireErrorKind;

use crate::exec_pty::{PtyProcessSpawner, SpawnedPtyProcess, StdinWriteOk, TerminalSize, TtyState};

/// Maximum retained live-buffer bytes per output stream (drop-oldest cap).
pub const STDOUT_LIVE_BUFFER_BYTES: usize = 1024 * 1024;
pub const STDERR_LIVE_BUFFER_BYTES: usize = 1024 * 1024;
/// Maximum number of concurrently tracked attached execs per VM.
pub const ATTACHED_SESSIONS_PER_VM: usize = 8;
/// Maximum total retained exec records (running + terminal) per VM.
pub const EXEC_SESSIONS_PER_VM: usize = 32;
/// Maximum pending `ExecWait` long-polls per VM.
pub const PENDING_EXEC_WAITS_PER_VM: usize = 64;
/// Maximum pending `ReadOutput` long-polls per stream.
pub const PENDING_READ_OUTPUT_WAITS_PER_STREAM: usize = 64;
/// Hard ceiling for a single output chunk returned to a reader.
pub const HARD_MAX_CHUNK_BYTES: u64 = 1024 * 1024;
/// Hard ceiling for a long-poll timeout, in milliseconds.
pub const HARD_MAX_LONG_POLL_TIMEOUT_MS: u64 = 1_000;
/// Wall-clock ceiling for a single attached exec, in milliseconds.
pub const MAX_EXEC_RUNTIME_MS: u64 = 6 * 60 * 60 * 1_000;
/// Bounded grace, in milliseconds, between releasing the PTY master (`SIGHUP`)
/// and the SIGKILL session sweep on interactive-exec teardown.
pub const TTY_TEARDOWN_GRACE_MS: u64 = 2_000;
/// Maximum argv entries accepted by the runtime.
pub const MAX_ARGV: usize = 128;
/// Maximum bytes for a single argv entry.
pub const MAX_ARG_BYTES: usize = 4096;
/// Maximum bytes for a working directory path.
pub const MAX_CWD_BYTES: usize = 4096;
/// Maximum bytes for an environment variable name.
pub const MAX_ENV_KEY_BYTES: usize = 128;
/// Maximum bytes for an environment variable value.
pub const MAX_ENV_VALUE_BYTES: usize = 8192;
/// Maximum environment entries accepted by the runtime.
pub const MAX_ENV_ENTRIES: usize = 256;
/// Maximum bytes for a server-generated exec id.
pub const EXEC_ID_BYTES: usize = 32;
/// Grace period for draining output after the direct child exits.
const POST_EXIT_DRAIN_GRACE_MS: u64 = 2_000;
/// Read buffer size for the per-stream pipe drain loop.
const PIPE_READ_CHUNK: usize = 64 * 1024;

/// Host-owned guest exec policy, delivered out of band (CLI flags from the
/// dormant guest unit). Defaults are fail-closed: exec disabled, no target
/// user. The target user is **host-fixed** (the VM's workload user); the
/// wire `user` field is never consulted for authorization, so a guest-control
/// client can never target root or any other user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecPolicy {
    pub enabled: bool,
    /// The single host-configured workload user every exec runs as. `None`
    /// means no target is configured (exec is effectively disabled even if
    /// `enabled` is set). Never `root`.
    pub exec_user: Option<String>,
}

impl ExecPolicy {
    pub fn disabled() -> Self {
        Self::default()
    }
}

/// Typed runtime outcome surfaced as a `GuestControlErrorKind`. Carries no
/// caller-supplied bytes, paths, or free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecError {
    ExecDisabled,
    RootDenied,
    UserDenied,
    UnsupportedMode,
    InvalidArgv,
    InvalidProgram,
    CwdInvalid,
    InvalidEnv,
    MaxChunkExceeded,
    ExecCapacityExceeded,
    AttachCapacityExceeded,
    WaitCapacityExceeded,
    ReadWaitCapacityExceeded,
    ExecNotFound,
    OffsetExpired,
    OffsetInFuture,
    SpawnFailed,
    RetainedLogPathUnsafe,
    RetainedLogQuotaExceeded,
    StaleSession,
    ExecExpired,
    // Interactive TTY exec. These map to existing wire kinds; no new wire
    // enum variant is introduced.
    InvalidTerminalSize,
    TtyStderrUnavailable,
    TtyRequired,
    StdinClosed,
    StdinOffsetMismatch,
    StdinByteBudgetExhausted,
    StdinBackpressure,
    ControlSeqMismatch,
    InvalidSignal,
    ExecClosing,
    Internal,
}

impl ExecError {
    pub fn wire_kind(self) -> WireErrorKind {
        match self {
            Self::ExecDisabled => WireErrorKind::GuestExecDisabled,
            Self::RootDenied => WireErrorKind::GuestExecRootDenied,
            Self::UserDenied => WireErrorKind::GuestExecUserDenied,
            // The supported attached protocol subset is non-TTY/non-detached/
            // stdin-closed. Requests outside that subset are protocol errors;
            // no new wire enum variants are added for them.
            Self::UnsupportedMode => WireErrorKind::ProtocolError,
            Self::InvalidArgv => WireErrorKind::ProtocolError,
            Self::InvalidProgram => WireErrorKind::InvalidProgram,
            Self::CwdInvalid => WireErrorKind::CwdInvalid,
            Self::InvalidEnv => WireErrorKind::ProtocolError,
            Self::MaxChunkExceeded => WireErrorKind::MaxChunkExceeded,
            Self::ExecCapacityExceeded => WireErrorKind::ExecCapacityExceeded,
            Self::AttachCapacityExceeded => WireErrorKind::ExecAttachCapacityExceeded,
            Self::WaitCapacityExceeded => WireErrorKind::WaitCapacityExceeded,
            Self::ReadWaitCapacityExceeded => WireErrorKind::ReadWaitCapacityExceeded,
            Self::ExecNotFound => WireErrorKind::ExecNotFound,
            Self::OffsetExpired => WireErrorKind::OffsetExpired,
            Self::OffsetInFuture => WireErrorKind::OffsetInFuture,
            Self::SpawnFailed => WireErrorKind::ProtocolError,
            Self::RetainedLogPathUnsafe => WireErrorKind::RetainedLogPathUnsafe,
            Self::RetainedLogQuotaExceeded => WireErrorKind::RetainedLogQuotaExceeded,
            Self::StaleSession => WireErrorKind::StaleSession,
            Self::ExecExpired => WireErrorKind::ExecExpired,
            // Interactive TTY exec: reuse existing wire kinds.
            Self::InvalidTerminalSize => WireErrorKind::ProtocolError,
            Self::TtyStderrUnavailable => WireErrorKind::TtyStderrUnavailable,
            Self::TtyRequired => WireErrorKind::TtyRequired,
            Self::StdinClosed => WireErrorKind::StdinClosed,
            Self::StdinOffsetMismatch => WireErrorKind::StdinOffsetMismatch,
            Self::StdinByteBudgetExhausted => WireErrorKind::StdinByteBudgetExhausted,
            Self::StdinBackpressure => WireErrorKind::StdinBackpressure,
            Self::ControlSeqMismatch => WireErrorKind::ControlSeqMismatch,
            Self::InvalidSignal => WireErrorKind::ProtocolError,
            Self::ExecClosing => WireErrorKind::ExecAlreadyExited,
            Self::Internal => WireErrorKind::ProtocolError,
        }
    }
}

/// One of the two captured output streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Public-facing exec lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecState {
    Running,
    Exited,
    Signaled,
    Cancelled,
    Reaped,
    LostGuestd,
}

/// Terminal disposition for a finished child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOutcome {
    Exited(i32),
    Signaled(u32),
}

/// A command that has passed validation and policy and is ready to spawn.
#[derive(Clone, PartialEq, Eq)]
pub struct ValidatedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

// Redacted Debug: never print argv/cwd/env values (they may carry secrets).
impl std::fmt::Debug for ValidatedCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedCommand")
            .field("argc", &(self.args.len() + 1))
            .field("env_count", &self.env.len())
            .finish_non_exhaustive()
    }
}

/// A spawned child handed back by a [`ProcessSpawner`]. The stdout/stderr
/// readers MUST be continuously drained by the runtime. The `killer` signals
/// the whole process group and is lock-free and idempotent; the `waiter` is
/// owned exclusively by the supervisor task.
pub struct SpawnedProcess {
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    pub stderr: Box<dyn AsyncRead + Send + Unpin>,
    pub killer: Arc<dyn ProcessKiller>,
    pub waiter: Box<dyn ProcessWaiter>,
}

/// Lock-free, idempotent process-group terminator. Held by the exec entry so
/// cancellation can signal the group without contending with the supervisor's
/// in-flight `wait`.
pub trait ProcessKiller: Send + Sync {
    /// Signal the whole process group (idempotent, best-effort).
    fn kill_group(&self);
}

/// Owns the direct child and waits for it to terminate. Exclusively owned by
/// the supervisor task, so no locking is required.
#[async_trait]
pub trait ProcessWaiter: Send {
    /// Wait for the direct child to terminate, reaping it.
    async fn wait(&mut self) -> ExitOutcome;
}

/// Spawns validated commands into their own process group.
#[async_trait]
pub trait ProcessSpawner: Send + Sync + 'static {
    async fn spawn(&self, command: ValidatedCommand) -> Result<SpawnedProcess, ExecError>;
}

/// Drop-oldest bounded ring buffer with monotonic offsets.
struct OutputRing {
    start_offset: u64,
    data: std::collections::VecDeque<u8>,
    cap: usize,
    dropped_bytes: u64,
    truncated: bool,
    eof: bool,
}

impl OutputRing {
    fn new(cap: usize) -> Self {
        Self {
            start_offset: 0,
            data: std::collections::VecDeque::new(),
            cap,
            dropped_bytes: 0,
            truncated: false,
            eof: false,
        }
    }

    fn end_offset(&self) -> u64 {
        self.start_offset.saturating_add(self.data.len() as u64)
    }

    fn append(&mut self, bytes: &[u8]) {
        self.data.extend(bytes.iter().copied());
        while self.data.len() > self.cap {
            if self.data.pop_front().is_some() {
                self.start_offset = self.start_offset.saturating_add(1);
                self.dropped_bytes = self.dropped_bytes.saturating_add(1);
                self.truncated = true;
            } else {
                break;
            }
        }
    }

    fn mark_eof(&mut self) {
        self.eof = true;
    }

    /// Read up to `max_len` bytes starting at `offset`. Returns the chunk and
    /// the offset following the returned bytes.
    fn read(&self, offset: u64, max_len: u64) -> Result<RingChunk, ExecError> {
        let end = self.end_offset();
        if offset < self.start_offset {
            return Err(ExecError::OffsetExpired);
        }
        if offset > end {
            return Err(ExecError::OffsetInFuture);
        }
        let available = end - offset;
        let take = available.min(max_len);
        let begin = (offset - self.start_offset) as usize;
        let take_usize = take as usize;
        let data: Vec<u8> = self
            .data
            .iter()
            .skip(begin)
            .take(take_usize)
            .copied()
            .collect();
        let next_offset = offset.saturating_add(take);
        Ok(RingChunk {
            data,
            start_offset: self.start_offset,
            end_offset: end,
            next_offset,
            dropped_bytes: self.dropped_bytes,
            truncated: self.truncated,
            // EOF is only observable once the stream is fully drained.
            eof: self.eof && next_offset >= end,
        })
    }
}

/// Result of a ring read.
#[derive(Clone, PartialEq, Eq)]
pub struct RingChunk {
    pub data: Vec<u8>,
    pub start_offset: u64,
    pub end_offset: u64,
    pub next_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub eof: bool,
}

// Redacted Debug: never print captured stdout/stderr bytes.
impl std::fmt::Debug for RingChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RingChunk")
            .field("len", &self.data.len())
            .field("start_offset", &self.start_offset)
            .field("end_offset", &self.end_offset)
            .field("next_offset", &self.next_offset)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("eof", &self.eof)
            .finish()
    }
}

/// Mutable, lock-protected exec state. All critical sections are short and
/// never span an await point.
struct ExecShared {
    state: ExecState,
    outcome: Option<ExitOutcome>,
    stdout: OutputRing,
    stderr: OutputRing,
    state_generation: u64,
}

impl ExecShared {
    fn bump(&mut self) {
        self.state_generation = self.state_generation.saturating_add(1);
    }

    fn ring(&self, stream: Stream) -> &OutputRing {
        match stream {
            Stream::Stdout => &self.stdout,
            Stream::Stderr => &self.stderr,
        }
    }

    fn ring_mut(&mut self, stream: Stream) -> &mut OutputRing {
        match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        }
    }
}

/// One tracked exec, owned by the connection that created it.
struct ExecEntry {
    owner: ConnectionKey,
    guest_boot_id: String,
    shared: Mutex<ExecShared>,
    notify: Arc<Notify>,
    killer: Arc<dyn ProcessKiller>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    // Pending ReadOutput long-polls, counted per stream (stdout, stderr).
    pending_reads: [AtomicU64; 2],
    // Interactive TTY session state, present iff this exec is a PTY-backed
    // interactive exec (tty=true, non-detached). Non-TTY execs leave this None.
    tty: Option<Arc<TtyState>>,
}

impl ExecEntry {
    fn snapshot(&self) -> ExecSnapshot {
        // Resolve the TTY stdin disposition + last control seq WITHOUT holding
        // the shared lock (these are independent short std mutexes on TtyState;
        // computing them first avoids any cross-lock ordering concern).
        let (stdin, last_control_seq) = match self.tty.as_ref() {
            None => (TtyStdinSnapshot::NotInteractive, 0),
            Some(tty) => {
                let disposition = if tty.stdin_closed() {
                    TtyStdinSnapshot::Closed
                } else if tty.is_closing() {
                    TtyStdinSnapshot::Closing
                } else {
                    TtyStdinSnapshot::Open
                };
                (disposition, tty.last_control_seq())
            }
        };
        let shared = self.lock_shared();
        ExecSnapshot {
            state: shared.state,
            outcome: shared.outcome,
            state_generation: shared.state_generation,
            stdout_start_offset: shared.stdout.start_offset,
            stdout_end_offset: shared.stdout.end_offset(),
            stderr_start_offset: shared.stderr.start_offset,
            stderr_end_offset: shared.stderr.end_offset(),
            stdout_dropped_bytes: shared.stdout.dropped_bytes,
            stderr_dropped_bytes: shared.stderr.dropped_bytes,
            stdout_truncated: shared.stdout.truncated,
            stderr_truncated: shared.stderr.truncated,
            stdin,
            last_control_seq,
        }
    }

    fn lock_shared(&self) -> std::sync::MutexGuard<'_, ExecShared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn is_terminal(&self) -> bool {
        !matches!(self.lock_shared().state, ExecState::Running)
    }

    fn pending_reads(&self, stream: Stream) -> &AtomicU64 {
        match stream {
            Stream::Stdout => &self.pending_reads[0],
            Stream::Stderr => &self.pending_reads[1],
        }
    }

    /// Idempotent cancellation. The process-group signal goes first and is
    /// lock-free, so it always reaches the group even while the supervisor is
    /// mid-`wait`. The reader tasks are aborted; the supervisor task is left to
    /// run free — once `kill_group` makes the child exit, the supervisor reaps
    /// it via its owned waiter and finishes on its own.
    fn cancel(&self) {
        self.killer.kill_group();
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for task in tasks.drain(..) {
            task.abort();
        }
        {
            let mut shared = self.lock_shared();
            if matches!(shared.state, ExecState::Running) {
                shared.state = ExecState::Cancelled;
                shared.stdout.mark_eof();
                shared.stderr.mark_eof();
                shared.bump();
            }
        }
        self.notify.notify_waiters();
    }
}

/// Interactive (TTY) stdin disposition carried in an [`ExecSnapshot`] so
/// `ExecInspect` is TTY-aware: a live TTY exec accepting `WriteStdin` shows
/// `Open`, a closed (VEOF-injected) one shows `Closed`, a tearing-down one shows
/// `Closing`. Non-TTY execs report `NotInteractive` (their stdin is never
/// writable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtyStdinSnapshot {
    NotInteractive,
    Open,
    Closing,
    Closed,
}

/// Public snapshot of an exec's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecSnapshot {
    pub state: ExecState,
    pub outcome: Option<ExitOutcome>,
    pub state_generation: u64,
    pub stdout_start_offset: u64,
    pub stdout_end_offset: u64,
    pub stderr_start_offset: u64,
    pub stderr_end_offset: u64,
    pub stdout_dropped_bytes: u64,
    pub stderr_dropped_bytes: u64,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    /// Interactive stdin disposition (TTY-aware inspect).
    pub stdin: TtyStdinSnapshot,
    /// Highest admitted resize/signal control sequence (0 for non-TTY execs).
    pub last_control_seq: u64,
}

/// Stable identity of an authenticated connection that owns its execs.
pub type ConnectionKey = Vec<u8>;

/// Source of opaque, server-generated exec ids.
pub trait ExecIdSource: Send + Sync + 'static {
    fn next_exec_id(&self) -> Result<String, ExecError>;
}

/// Validated exec request fields the runtime accepts, decoupled from the wire
/// types so the service layer owns protobuf decoding.
pub struct ExecCreateInput {
    pub argv: Vec<String>,
    pub user: Option<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub tty: bool,
    pub stdin_open: bool,
    pub detached: bool,
    pub has_terminal_size: bool,
    pub max_chunk_bytes: u64,
}

/// Validate the supported-subset flags and command shape, then apply policy.
pub fn validate_and_authorize(
    input: &ExecCreateInput,
    policy: &ExecPolicy,
) -> Result<ValidatedCommand, ExecError> {
    if !policy.enabled {
        return Err(ExecError::ExecDisabled);
    }
    if input.tty || input.detached || input.stdin_open || input.has_terminal_size {
        return Err(ExecError::UnsupportedMode);
    }
    if input.max_chunk_bytes == 0 || input.max_chunk_bytes > HARD_MAX_CHUNK_BYTES {
        return Err(ExecError::MaxChunkExceeded);
    }

    // The target user is host-fixed (`policy.exec_user`, the VM workload
    // user). The wire `user` field is never consulted for authorization, so a
    // guest-control client cannot target root or any other user. A missing
    // host-configured user fails closed.
    if policy.exec_user.is_none() {
        return Err(ExecError::ExecDisabled);
    }

    let command = validate_command(input)?;
    Ok(command)
}

/// Validate + authorize a **detached** create. Mirrors [`validate_and_authorize`]
/// but permits `detached = true` while still rejecting the unsupported
/// interactive flags (`tty`, `stdin_open`, `has_terminal_size`). Detached execs
/// do not use the attached `max_chunk_bytes` policy (logs use fixed per-stream
/// caps), so that bound is not enforced here.
pub fn validate_and_authorize_detached(
    input: &ExecCreateInput,
    policy: &ExecPolicy,
) -> Result<ValidatedCommand, ExecError> {
    if !policy.enabled {
        return Err(ExecError::ExecDisabled);
    }
    if input.tty || input.stdin_open || input.has_terminal_size {
        return Err(ExecError::UnsupportedMode);
    }

    // Host-fixed target user; the wire `user` field is ignored. Fail closed
    // when no workload user is configured.
    if policy.exec_user.is_none() {
        return Err(ExecError::ExecDisabled);
    }

    let command = validate_command(input)?;
    Ok(command)
}

/// Validate + authorize an **interactive TTY** create (`tty = true`,
/// non-detached). Permits `tty`, `stdin_open`, and an initial terminal size
/// while still rejecting `detached` (the interactive path is connection-owned
/// and non-durable; `tty && detached` is an unsupported mode). The merged PTY
/// output is delivered through the same chunk policy as attached non-TTY exec,
/// so the `max_chunk_bytes` bound is enforced here too.
pub fn validate_and_authorize_tty(
    input: &ExecCreateInput,
    policy: &ExecPolicy,
) -> Result<ValidatedCommand, ExecError> {
    if !policy.enabled {
        return Err(ExecError::ExecDisabled);
    }
    // `tty && detached` and a non-TTY request routed here are unsupported.
    if input.detached || !input.tty {
        return Err(ExecError::UnsupportedMode);
    }
    if input.max_chunk_bytes == 0 || input.max_chunk_bytes > HARD_MAX_CHUNK_BYTES {
        return Err(ExecError::MaxChunkExceeded);
    }

    // Host-fixed target user; the wire `user` field is ignored. Fail closed
    // when no workload user is configured.
    if policy.exec_user.is_none() {
        return Err(ExecError::ExecDisabled);
    }

    let command = validate_command(input)?;
    Ok(command)
}

fn validate_command(input: &ExecCreateInput) -> Result<ValidatedCommand, ExecError> {
    if input.argv.is_empty() || input.argv.len() > MAX_ARGV {
        return Err(ExecError::InvalidArgv);
    }
    for arg in &input.argv {
        if arg.len() > MAX_ARG_BYTES || arg.as_bytes().contains(&0) {
            return Err(ExecError::InvalidArgv);
        }
    }
    let program = &input.argv[0];
    // The login-shell wrapper invokes `exec "$@"`; reject leading '-' so argv[0]
    // cannot be parsed as an exec option before PATH/relative/absolute lookup.
    if program.is_empty() || program.starts_with('-') {
        return Err(ExecError::InvalidProgram);
    }
    let program = PathBuf::from(program);

    let cwd = match input.cwd.as_deref() {
        Some(cwd) => {
            if cwd.is_empty()
                || cwd.len() > MAX_CWD_BYTES
                || !cwd.starts_with('/')
                || cwd.as_bytes().contains(&0)
            {
                return Err(ExecError::CwdInvalid);
            }
            PathBuf::from(cwd)
        }
        None => PathBuf::from("/"),
    };

    if input.env.len() > MAX_ENV_ENTRIES {
        return Err(ExecError::InvalidEnv);
    }
    let mut seen = std::collections::BTreeSet::new();
    for (key, value) in &input.env {
        if !valid_env_key(key) || value.len() > MAX_ENV_VALUE_BYTES || value.as_bytes().contains(&0)
        {
            return Err(ExecError::InvalidEnv);
        }
        if !seen.insert(key.clone()) {
            return Err(ExecError::InvalidEnv);
        }
    }

    Ok(ValidatedCommand {
        program,
        args: input.argv[1..].to_vec(),
        cwd,
        env: input.env.clone(),
    })
}

fn valid_env_key(key: &str) -> bool {
    if key.is_empty() || key.len() > MAX_ENV_KEY_BYTES {
        return false;
    }
    let mut chars = key.chars();
    let first = chars.next().unwrap_or('=');
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The guest exec runtime: tracks per-connection execs and drives spawning,
/// output retention, waiting, and cleanup.
pub struct ExecRuntime<Sp, Id> {
    spawner: Arc<Sp>,
    ids: Arc<Id>,
    policy: ExecPolicy,
    execs: Mutex<BTreeMap<String, Arc<ExecEntry>>>,
    // In-flight create reservations, counted against capacity so concurrent
    // creates cannot collectively exceed the caps before they are inserted.
    reservations: AtomicU64,
    // Pending ExecWait long-polls, counted at VM scope.
    pending_exec_waits: AtomicU64,
    // Interactive TTY support. The PTY spawner is a trait object so the
    // generic `Sp`/`Id` shape (and every test/ctor) is unchanged; it defaults
    // to a null spawner that reports the mode unsupported. `tty_usable` is set
    // only when a real PTY spawner is wired in (and gates the advertised
    // capability). `interactive_ceiling` is the per-session runtime ceiling for
    // tty execs (None = unlimited); non-TTY attached execs keep
    // `MAX_EXEC_RUNTIME_MS`.
    pty_spawner: Arc<dyn PtyProcessSpawner>,
    tty_usable: bool,
    interactive_ceiling: Option<Duration>,
    tty_grace: Duration,
}

impl<Sp, Id> ExecRuntime<Sp, Id>
where
    Sp: ProcessSpawner,
    Id: ExecIdSource,
{
    pub fn new(spawner: Sp, ids: Id, policy: ExecPolicy) -> Self {
        Self {
            spawner: Arc::new(spawner),
            ids: Arc::new(ids),
            policy,
            execs: Mutex::new(BTreeMap::new()),
            reservations: AtomicU64::new(0),
            pending_exec_waits: AtomicU64::new(0),
            pty_spawner: Arc::new(crate::exec_pty::NullPtySpawner),
            tty_usable: false,
            interactive_ceiling: None,
            tty_grace: Duration::from_millis(TTY_TEARDOWN_GRACE_MS),
        }
    }

    /// Wire in a production PTY spawner, enabling the interactive TTY path and
    /// allowing the `exec_tty` capability to be advertised. Consumes self.
    pub fn with_pty_spawner(mut self, spawner: Arc<dyn PtyProcessSpawner>) -> Self {
        self.pty_spawner = spawner;
        self.tty_usable = true;
        self
    }

    /// Set the per-session runtime ceiling for interactive (TTY) execs.
    /// `None` means unlimited. Non-TTY attached execs are unaffected.
    pub fn with_interactive_ceiling(mut self, ceiling: Option<Duration>) -> Self {
        self.interactive_ceiling = ceiling;
        self
    }

    /// Override the teardown grace window between the `SIGHUP` (master release)
    /// and the SIGKILL session sweep. Test-only.
    #[cfg(test)]
    pub fn with_tty_grace(mut self, grace: Duration) -> Self {
        self.tty_grace = grace;
        self
    }

    /// True when the interactive TTY path is usable (a real PTY spawner is
    /// wired in). The service layer advertises `exec_tty` only when this holds.
    pub fn tty_usable(&self) -> bool {
        self.tty_usable
    }

    /// The effective exec policy (read-only; used by the detached path to
    /// authorize a create routed to the detached registry).
    pub fn policy(&self) -> &ExecPolicy {
        &self.policy
    }

    fn lock_execs(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Arc<ExecEntry>>> {
        self.execs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Look up an exec the caller is allowed to observe. Enforces same-owner
    /// connection identity and matching guest boot id.
    fn lookup_owned(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
    ) -> Result<Arc<ExecEntry>, ExecError> {
        let execs = self.lock_execs();
        let entry = execs.get(exec_id).ok_or(ExecError::ExecNotFound)?;
        if &entry.owner != owner || entry.guest_boot_id != guest_boot_id {
            return Err(ExecError::ExecNotFound);
        }
        Ok(Arc::clone(entry))
    }

    /// Create-and-start an attached exec. Validates, authorizes, allocates a
    /// server-generated id, spawns, and registers reader/supervisor tasks.
    ///
    /// A placeholder entry is inserted under the execs lock before the spawn
    /// await so that a concurrent `close_connection` for the owning connection
    /// observes and cancels the in-flight exec; if the placeholder is gone when
    /// the spawn completes, the just-spawned process is torn down rather than
    /// left running for a closed connection.
    pub async fn create(
        &self,
        owner: ConnectionKey,
        guest_boot_id: String,
        input: ExecCreateInput,
    ) -> Result<(String, ExecSnapshot), ExecError> {
        let command = validate_and_authorize(&input, &self.policy)?;

        // Reserve a capacity slot under the execs lock so concurrent creates
        // cannot collectively exceed the caps. The guard releases the slot on
        // every early return; it is dropped once the placeholder occupies it.
        let reservation = {
            let execs = self.lock_execs();
            let reserved = self.reservations.load(Ordering::SeqCst) as usize;
            if execs.len() + reserved >= EXEC_SESSIONS_PER_VM {
                return Err(ExecError::ExecCapacityExceeded);
            }
            let running = execs.values().filter(|entry| !entry.is_terminal()).count();
            if running + reserved >= ATTACHED_SESSIONS_PER_VM {
                return Err(ExecError::AttachCapacityExceeded);
            }
            self.reservations.fetch_add(1, Ordering::SeqCst);
            CounterGuard {
                counter: &self.reservations,
            }
        };

        let exec_id = self.ids.next_exec_id()?;
        if exec_id.is_empty() || exec_id.len() > EXEC_ID_BYTES {
            return Err(ExecError::Internal);
        }

        // Insert a placeholder so close_connection can see this in-flight exec.
        let placeholder =
            new_exec_entry(owner.clone(), guest_boot_id.clone(), Arc::new(NoopKiller));
        self.lock_execs()
            .insert(exec_id.clone(), Arc::clone(&placeholder));
        // The placeholder now occupies the slot; release the reservation.
        drop(reservation);

        let spawned = match self.spawner.spawn(command).await {
            Ok(spawned) => spawned,
            Err(error) => {
                self.lock_execs().remove(&exec_id);
                return Err(error);
            }
        };
        let SpawnedProcess {
            stdout,
            stderr,
            killer,
            waiter,
        } = spawned;

        let entry = new_exec_entry(owner, guest_boot_id, Arc::clone(&killer));
        let stdout_task = spawn_reader(Arc::clone(&entry), Stream::Stdout, stdout);
        let stderr_task = spawn_reader(Arc::clone(&entry), Stream::Stderr, stderr);
        {
            let mut tasks = entry
                .tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            tasks.push(stdout_task);
            tasks.push(stderr_task);
        }
        // The supervisor owns the waiter exclusively and runs free; it reaps
        // the child and finishes on its own (it holds an Arc to the entry).
        spawn_supervisor(Arc::clone(&entry), waiter, Arc::clone(&killer));

        // Commit: swap the placeholder for the real entry iff it is still
        // present. If close_connection removed it during spawn, the connection
        // is gone — tear the process down instead of tracking it.
        let committed = {
            let mut execs = self.lock_execs();
            if execs.remove(&exec_id).is_some() {
                execs.insert(exec_id.clone(), Arc::clone(&entry));
                true
            } else {
                false
            }
        };
        if !committed {
            entry.cancel();
            return Err(ExecError::ExecNotFound);
        }

        let snapshot = entry.snapshot();
        Ok((exec_id, snapshot))
    }

    /// Create-and-start an **interactive TTY** exec (`tty = true`,
    /// non-detached). Allocates a PTY pair, spawns the first-party helper as the
    /// session leader with the slave as its controlling terminal, drains the
    /// merged master output into the stdout ring, and wires a TTY supervisor +
    /// teardown. Returns the id, the initial snapshot, and the initial
    /// `control_seq` (0). Capacity is shared with attached non-TTY execs.
    pub async fn create_tty(
        &self,
        owner: ConnectionKey,
        guest_boot_id: String,
        input: ExecCreateInput,
        initial_size: Option<(u32, u32)>,
    ) -> Result<(String, ExecSnapshot, u64), ExecError> {
        let command = validate_and_authorize_tty(&input, &self.policy)?;
        // The capability is advertised only when usable; a create that reaches
        // here without a real PTY spawner fails closed.
        if !self.tty_usable {
            return Err(ExecError::UnsupportedMode);
        }
        let size = TerminalSize::resolve_initial(initial_size)?;

        // Reserve a capacity slot under the execs lock (shared caps with the
        // attached non-TTY path).
        let reservation = {
            let execs = self.lock_execs();
            let reserved = self.reservations.load(Ordering::SeqCst) as usize;
            if execs.len() + reserved >= EXEC_SESSIONS_PER_VM {
                return Err(ExecError::ExecCapacityExceeded);
            }
            let running = execs.values().filter(|entry| !entry.is_terminal()).count();
            if running + reserved >= ATTACHED_SESSIONS_PER_VM {
                return Err(ExecError::AttachCapacityExceeded);
            }
            self.reservations.fetch_add(1, Ordering::SeqCst);
            CounterGuard {
                counter: &self.reservations,
            }
        };

        let exec_id = self.ids.next_exec_id()?;
        if exec_id.is_empty() || exec_id.len() > EXEC_ID_BYTES {
            return Err(ExecError::Internal);
        }

        let placeholder =
            new_exec_entry(owner.clone(), guest_boot_id.clone(), Arc::new(NoopKiller));
        self.lock_execs()
            .insert(exec_id.clone(), Arc::clone(&placeholder));
        drop(reservation);

        let spawned = match self.pty_spawner.spawn(command, size).await {
            Ok(spawned) => spawned,
            Err(error) => {
                self.lock_execs().remove(&exec_id);
                return Err(error);
            }
        };
        let SpawnedPtyProcess {
            reader,
            writer,
            control,
            waiter,
            reaper,
        } = spawned;

        let tty = Arc::new(TtyState::new(writer, control, reaper));
        // The entry killer SIGKILLs the whole session (so an accidental
        // `cancel()` path still reaps the interactive session).
        let killer: Arc<dyn ProcessKiller> = Arc::new(SessionKiller {
            tty: Arc::clone(&tty),
        });
        let entry = new_exec_entry_with_tty(owner, guest_boot_id, killer, Some(Arc::clone(&tty)));
        // Merged output: stderr is folded into the PTY, so the stderr stream is
        // never produced. Mark it EOF up front; ReadOutput(Stderr) on a TTY exec
        // is rejected with TtyStderrUnavailable before it reaches the ring.
        {
            let mut shared = entry.lock_shared();
            shared.stderr.mark_eof();
        }
        let reader_task = spawn_reader(Arc::clone(&entry), Stream::Stdout, reader);
        {
            let mut tasks = entry
                .tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            tasks.push(reader_task);
        }
        spawn_tty_supervisor(
            Arc::clone(&entry),
            Arc::clone(&tty),
            waiter,
            self.interactive_ceiling,
            self.tty_grace,
        );

        let committed = {
            let mut execs = self.lock_execs();
            if execs.remove(&exec_id).is_some() {
                execs.insert(exec_id.clone(), Arc::clone(&entry));
                true
            } else {
                false
            }
        };
        if !committed {
            // The connection vanished during spawn; tear the session down.
            let grace = self.tty_grace;
            tokio::spawn(teardown_tty(entry, tty, None, grace));
            return Err(ExecError::ExecNotFound);
        }

        let snapshot = entry.snapshot();
        Ok((exec_id, snapshot, 0))
    }

    /// Look up an owned TTY exec, returning its session state. Non-TTY execs
    /// yield `TtyRequired`; an unknown/foreign exec yields `ExecNotFound`.
    fn lookup_tty(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
    ) -> Result<Arc<TtyState>, ExecError> {
        let entry = self.lookup_owned(owner, exec_id, guest_boot_id)?;
        entry.tty.clone().ok_or(ExecError::TtyRequired)
    }

    /// Look up an owned exec's TTY state for a stdin RPC. A non-TTY exec has no
    /// writable stdin, so WriteStdin/CloseStdin map to `StdinClosed`.
    fn lookup_tty_stdin(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
    ) -> Result<Arc<TtyState>, ExecError> {
        let entry = self.lookup_owned(owner, exec_id, guest_boot_id)?;
        entry.tty.clone().ok_or(ExecError::StdinClosed)
    }

    /// WriteStdin to an owned TTY exec at `offset` (optionally injecting VEOF
    /// via `close_after`). Returns the accepted-byte count, the new next-offset,
    /// and whether stdin is now closed (partial-write-aware — see [`StdinWriteOk`]).
    pub async fn write_stdin(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
        offset: u64,
        data: &[u8],
        close_after: bool,
    ) -> Result<StdinWriteOk, ExecError> {
        let tty = self.lookup_tty_stdin(owner, exec_id, guest_boot_id)?;
        tty.write_stdin(offset, data, close_after).await
    }

    /// CloseStdin on an owned TTY exec: inject VEOF, keep the master open, and
    /// mark stdin closed (idempotent). Returns `(final_offset, duplicate)`.
    pub async fn close_stdin(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
        offset: u64,
    ) -> Result<(u64, bool), ExecError> {
        let tty = self.lookup_tty_stdin(owner, exec_id, guest_boot_id)?;
        tty.close_stdin(offset).await
    }

    /// Apply a TtyWinResize to an owned TTY exec (control_seq-ordered).
    pub fn tty_resize(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
        control_seq: u64,
        rows: u32,
        cols: u32,
    ) -> Result<(), ExecError> {
        let tty = self.lookup_tty(owner, exec_id, guest_boot_id)?;
        tty.resize(control_seq, rows, cols)
    }

    /// Deliver an ExecSignal to an owned TTY exec's foreground process group
    /// (control_seq-ordered; allowlisted signals only).
    pub fn tty_signal(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
        control_seq: u64,
        signal: u32,
    ) -> Result<(), ExecError> {
        let tty = self.lookup_tty(owner, exec_id, guest_boot_id)?;
        tty.signal(control_seq, signal)
    }

    /// Snapshot an owned exec's current state.
    pub fn inspect(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
    ) -> Result<ExecSnapshot, ExecError> {
        let entry = self.lookup_owned(owner, exec_id, guest_boot_id)?;
        Ok(entry.snapshot())
    }

    /// Long-poll for a state change up to a bounded timeout.
    pub async fn wait(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
        known_generation: Option<u64>,
        timeout_ms: u64,
    ) -> Result<(ExecSnapshot, bool), ExecError> {
        let entry = self.lookup_owned(owner, exec_id, guest_boot_id)?;
        if self.pending_exec_waits.fetch_add(1, Ordering::SeqCst) as usize
            >= PENDING_EXEC_WAITS_PER_VM
        {
            self.pending_exec_waits.fetch_sub(1, Ordering::SeqCst);
            return Err(ExecError::WaitCapacityExceeded);
        }
        // Guard releases the slot even if this future is cancelled mid-await.
        let _guard = CounterGuard {
            counter: &self.pending_exec_waits,
        };
        self.wait_inner(&entry, known_generation, timeout_ms).await
    }

    async fn wait_inner(
        &self,
        entry: &Arc<ExecEntry>,
        known_generation: Option<u64>,
        timeout_ms: u64,
    ) -> Result<(ExecSnapshot, bool), ExecError> {
        let timeout = Duration::from_millis(timeout_ms.min(HARD_MAX_LONG_POLL_TIMEOUT_MS));
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Subscribe BEFORE reading state so a transition that races the
            // wait cannot be missed.
            let notified = entry.notify.notified();
            let snapshot = entry.snapshot();
            let changed = known_generation
                .map(|known| snapshot.state_generation != known)
                .unwrap_or(true);
            if !matches!(snapshot.state, ExecState::Running) || changed {
                return Ok((snapshot, false));
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok((snapshot, true));
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Ok((entry.snapshot(), true));
            }
        }
    }

    /// Read a bounded chunk of output, optionally long-polling for new bytes.
    /// Returns the chunk and whether a long-poll timed out.
    #[allow(clippy::too_many_arguments)] // distinct read parameters; grouping adds no clarity
    pub async fn read_output(
        &self,
        owner: &ConnectionKey,
        exec_id: &str,
        guest_boot_id: &str,
        stream: Stream,
        offset: u64,
        max_len: u64,
        wait: bool,
        timeout_ms: u64,
    ) -> Result<(RingChunk, bool), ExecError> {
        let entry = self.lookup_owned(owner, exec_id, guest_boot_id)?;
        // A TTY exec merges stderr into the PTY master (delivered as stdout), so
        // there is no separate stderr stream to read.
        if entry.tty.is_some() && matches!(stream, Stream::Stderr) {
            return Err(ExecError::TtyStderrUnavailable);
        }
        let max_len = if max_len == 0 {
            return Err(ExecError::MaxChunkExceeded);
        } else {
            max_len.min(HARD_MAX_CHUNK_BYTES)
        };

        // Fast path: immediate read.
        {
            let shared = entry.lock_shared();
            let chunk = shared.ring(stream).read(offset, max_len)?;
            if !wait || !chunk.data.is_empty() || chunk.eof {
                return Ok((chunk, false));
            }
        }

        if entry.pending_reads(stream).fetch_add(1, Ordering::SeqCst) as usize
            >= PENDING_READ_OUTPUT_WAITS_PER_STREAM
        {
            entry.pending_reads(stream).fetch_sub(1, Ordering::SeqCst);
            return Err(ExecError::ReadWaitCapacityExceeded);
        }
        // Guard releases the slot even if this future is cancelled mid-await.
        let _guard = CounterGuard {
            counter: entry.pending_reads(stream),
        };
        self.read_wait_inner(&entry, stream, offset, max_len, timeout_ms)
            .await
    }

    async fn read_wait_inner(
        &self,
        entry: &Arc<ExecEntry>,
        stream: Stream,
        offset: u64,
        max_len: u64,
        timeout_ms: u64,
    ) -> Result<(RingChunk, bool), ExecError> {
        let timeout = Duration::from_millis(timeout_ms.min(HARD_MAX_LONG_POLL_TIMEOUT_MS));
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = entry.notify.notified();
            {
                let shared = entry.lock_shared();
                let chunk = shared.ring(stream).read(offset, max_len)?;
                if !chunk.data.is_empty() || chunk.eof {
                    return Ok((chunk, false));
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                let shared = entry.lock_shared();
                // Timed-out reads still report the current readable window.
                let chunk = shared.ring(stream).read(offset, max_len)?;
                return Ok((chunk, true));
            }
        }
    }

    /// Terminate and forget every exec owned by a disconnecting connection.
    ///
    /// Removal and collection happen atomically under the execs lock, so the
    /// entry that is cancelled is exactly the entry that was removed. This is
    /// consistent with create()'s single-locked commit: close either observes
    /// the placeholder (and removes/cancels it, so create's later commit finds
    /// it gone and tears the real process down) or observes the committed real
    /// entry (and removes/cancels it with its real killer). There is no window
    /// in which the real entry is removed from tracking without its group being
    /// signalled.
    ///
    /// A TTY exec is torn down via the async `Running → Closing → Terminal`
    /// teardown (release master → SIGHUP → bounded grace → SIGKILL the session)
    /// rather than the synchronous group-kill `cancel()` path.
    pub fn close_connection(&self, owner: &ConnectionKey) {
        let owned: Vec<Arc<ExecEntry>> = {
            let mut execs = self.lock_execs();
            let ids: Vec<String> = execs
                .iter()
                .filter(|(_, entry)| &entry.owner == owner)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| execs.remove(id)).collect()
        };
        for entry in owned {
            match entry.tty.clone() {
                Some(tty) => {
                    let grace = self.tty_grace;
                    tokio::spawn(teardown_tty(entry, tty, None, grace));
                }
                None => entry.cancel(),
            }
        }
    }

    #[cfg(test)]
    fn tracked_len(&self) -> usize {
        self.lock_execs().len()
    }
}

/// Decrements an atomic counter on drop, so capacity counters are released
/// even if the holding future is cancelled mid-await.
struct CounterGuard<'a> {
    counter: &'a AtomicU64,
}

impl Drop for CounterGuard<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// No-op killer used for the pre-spawn placeholder entry (no process yet).
struct NoopKiller;

impl ProcessKiller for NoopKiller {
    fn kill_group(&self) {}
}

/// Killer for a TTY exec entry: SIGKILLs the whole interactive session via the
/// session reaper. Used so an accidental `cancel()` on a TTY entry still reaps
/// the session (the normal teardown path is `teardown_tty`).
struct SessionKiller {
    tty: Arc<TtyState>,
}

impl ProcessKiller for SessionKiller {
    fn kill_group(&self) {
        self.tty.reaper().kill_session();
    }
}

/// Build a fresh running exec entry with empty output rings.
fn new_exec_entry(
    owner: ConnectionKey,
    guest_boot_id: String,
    killer: Arc<dyn ProcessKiller>,
) -> Arc<ExecEntry> {
    new_exec_entry_with_tty(owner, guest_boot_id, killer, None)
}

/// Build a fresh running exec entry, optionally carrying interactive TTY state.
fn new_exec_entry_with_tty(
    owner: ConnectionKey,
    guest_boot_id: String,
    killer: Arc<dyn ProcessKiller>,
    tty: Option<Arc<TtyState>>,
) -> Arc<ExecEntry> {
    Arc::new(ExecEntry {
        owner,
        guest_boot_id,
        shared: Mutex::new(ExecShared {
            state: ExecState::Running,
            outcome: None,
            stdout: OutputRing::new(STDOUT_LIVE_BUFFER_BYTES),
            stderr: OutputRing::new(STDERR_LIVE_BUFFER_BYTES),
            state_generation: 1,
        }),
        notify: Arc::new(Notify::new()),
        killer,
        tasks: Mutex::new(Vec::new()),
        pending_reads: [AtomicU64::new(0), AtomicU64::new(0)],
        tty,
    })
}

/// Continuously drain a child output stream into its ring buffer until EOF or
/// task cancellation. Never stops reading because the client is slow.
fn spawn_reader(
    entry: Arc<ExecEntry>,
    stream: Stream,
    mut reader: Box<dyn AsyncRead + Send + Unpin>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0_u8; PIPE_READ_CHUNK];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    {
                        let mut shared = entry.lock_shared();
                        shared.ring_mut(stream).append(&buf[..n]);
                        shared.bump();
                    }
                    entry.notify.notify_waiters();
                }
                Err(_) => break,
            }
        }
        {
            let mut shared = entry.lock_shared();
            shared.ring_mut(stream).mark_eof();
            shared.bump();
        }
        entry.notify.notify_waiters();
    })
}

/// Await the direct child (bounded by a wall-clock runtime ceiling), then clean
/// up the process group, drain output within a grace window, and publish the
/// terminal state.
///
/// The `waiter` is owned exclusively by this task (no shared lock), so the
/// lock-free `killer` held by the exec entry can always signal the group even
/// while this task is parked in `wait`. On a normal exit the waiter has already
/// reaped the direct child; the subsequent `kill_group` then cleans up any
/// descendants. A process group remains allocated while any member is alive, so
/// once the leader is reaped the group is either still populated (its PGID is
/// still in use and cannot be reassigned) or empty (the signal is a harmless
/// `ESRCH`); the leader PID cannot be recycled within this window.
fn spawn_supervisor(
    entry: Arc<ExecEntry>,
    mut waiter: Box<dyn ProcessWaiter>,
    killer: Arc<dyn ProcessKiller>,
) {
    // Detached on purpose: the task holds an Arc to the entry and finishes on
    // its own once the child is reaped. The JoinHandle is intentionally
    // dropped.
    tokio::spawn(async move {
        let ceiling = Duration::from_millis(MAX_EXEC_RUNTIME_MS);
        let waited = tokio::time::timeout(ceiling, waiter.wait()).await;
        let (outcome, ceiling_exceeded) = match waited {
            Ok(outcome) => (outcome, false),
            // Runtime ceiling hit: force-terminate the group, then explicitly
            // wait the (still-unreaped) direct child so it is reaped rather
            // than left to best-effort kill_on_drop cleanup.
            Err(_) => {
                killer.kill_group();
                let outcome = waiter.wait().await;
                (outcome, true)
            }
        };
        // Clean up any descendants left in the group.
        killer.kill_group();
        // Bounded drain grace: give readers a chance to observe trailing
        // output before forcing terminal accounting.
        tokio::time::sleep(Duration::from_millis(POST_EXIT_DRAIN_GRACE_MS)).await;
        // Keep the waiter owned until here so the child is never reaped via a
        // best-effort drop path.
        drop(waiter);
        {
            let mut shared = entry.lock_shared();
            if matches!(shared.state, ExecState::Running) {
                shared.state = if ceiling_exceeded {
                    ExecState::Cancelled
                } else {
                    match outcome {
                        ExitOutcome::Exited(_) => ExecState::Exited,
                        ExitOutcome::Signaled(_) => ExecState::Signaled,
                    }
                };
                shared.outcome = if ceiling_exceeded {
                    None
                } else {
                    Some(outcome)
                };
                shared.stdout.mark_eof();
                shared.stderr.mark_eof();
                shared.bump();
            }
        }
        entry.notify.notify_waiters();
    });
}

/// Supervise an interactive TTY exec: await the direct child (helper → target),
/// bounded by an optional interactive runtime ceiling (`None` = unlimited), then
/// run the idempotent `Running → Closing → Terminal` teardown. On ceiling
/// exceed the session is torn down (SIGHUP + SIGKILL) and the killed child is
/// reaped so it never lingers as a zombie.
fn spawn_tty_supervisor(
    entry: Arc<ExecEntry>,
    tty: Arc<TtyState>,
    mut waiter: Box<dyn ProcessWaiter>,
    ceiling: Option<Duration>,
    grace: Duration,
) {
    tokio::spawn(async move {
        let outcome = match ceiling {
            Some(limit) => tokio::time::timeout(limit, waiter.wait()).await.ok(),
            None => Some(waiter.wait().await),
        };
        match outcome {
            Some(outcome) => {
                teardown_tty(Arc::clone(&entry), Arc::clone(&tty), Some(outcome), grace).await;
            }
            // Ceiling exceeded: tear down (SIGHUP + SIGKILL the session), then
            // reap the now-killed direct child so it is not left as a zombie.
            None => {
                teardown_tty(Arc::clone(&entry), Arc::clone(&tty), None, grace).await;
                let _ = waiter.wait().await;
            }
        }
        drop(waiter);
    });
}

/// Idempotent interactive-exec teardown. The first caller to win the
/// `Running → Closing` race owns the teardown: it releases the master write
/// half and control surface, then drops the merged-output reader (dropping the
/// last master reference sends `SIGHUP` to the session), waits a bounded grace,
/// then SIGKILLs every process remaining in the session and publishes the
/// terminal state. `outcome = None` => the session was cancelled (disconnect /
/// ceiling); `Some(_)` => the child exited on its own.
///
/// Master-clone ordering: ALL tasks that hold a master clone (the abortable writer task
/// AND the merged-output reader task) are dropped/awaited BEFORE the
/// SIGHUP→grace→KILL window, so the master `OwnedFd` is actually closed and the
/// kernel delivers `SIGHUP` to the foreground session within the grace. The
/// writer is released by aborting its task (never by contending for the lock a
/// blocked PTY write holds), so a child that stopped reading stdin cannot
/// deadlock teardown.
///
/// Drain: on a normal child exit (`outcome = Some`) the slave is gone, so the
/// master read returns EOF/EIO on its own — the reader is given a bounded grace
/// to drain trailing output before being aborted. On cancel/disconnect
/// (`outcome = None`) the child may still be running, so the reader is aborted
/// immediately to drop its master clone and force `SIGHUP`.
///
/// Both the supervisor (on child exit) and `close_connection` (on disconnect)
/// call this; the loser of the `begin_closing` race early-returns, so the state
/// is published exactly once and the disconnect (Cancelled) wins a race with a
/// late child-exit.
async fn teardown_tty(
    entry: Arc<ExecEntry>,
    tty: Arc<TtyState>,
    outcome: Option<ExitOutcome>,
    grace: Duration,
) {
    if !tty.begin_closing() {
        return;
    }
    // Entering Closing has already rejected new stdin/control RPCs. Release the
    // master write half (abort+await the writer task — never lock-contend) and
    // the control clone so two of the three master references are gone.
    tty.release_writer().await;
    let _ = tty.take_control();
    // Take the reader task(s) out of the entry and drop their master clone
    // BEFORE the grace window: on a normal exit, drain to natural EOF within a
    // bounded grace (capturing trailing output) then abort if still running; on
    // cancel/disconnect, abort immediately so the last master clone drops and
    // SIGHUP is delivered.
    let reader_tasks: Vec<JoinHandle<()>> = {
        let mut tasks = entry
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tasks.drain(..).collect()
    };
    for mut handle in reader_tasks {
        if outcome.is_some() {
            match tokio::time::timeout(grace, &mut handle).await {
                // Reader finished on its own (natural EOF). The JoinHandle
                // future already completed here — must NOT be awaited again.
                Ok(_joined) => {}
                Err(_elapsed) => {
                    handle.abort();
                    let _ = handle.await;
                }
            }
        } else {
            handle.abort();
            let _ = handle.await;
        }
    }
    // All three master references (writer task, control, reader task) are now
    // dropped → the master OwnedFd is closed → the kernel delivers SIGHUP to the
    // foreground session. Bounded grace for SIGHUP to take effect before the
    // SIGKILL sweep.
    tokio::time::sleep(grace).await;
    // SIGKILL every process still in the session (repeats until empty). The
    // no-orphan guarantee is scoped to in-session processes; a setsid/double
    // fork escapee is a documented trusted-root limitation.
    tty.reaper().kill_session();
    {
        let mut shared = entry.lock_shared();
        if matches!(shared.state, ExecState::Running) {
            shared.state = match outcome {
                None => ExecState::Cancelled,
                Some(ExitOutcome::Exited(_)) => ExecState::Exited,
                Some(ExitOutcome::Signaled(_)) => ExecState::Signaled,
            };
            shared.outcome = outcome;
            shared.stdout.mark_eof();
            shared.stderr.mark_eof();
            shared.bump();
        }
    }
    tty.mark_terminal();
    entry.notify.notify_waiters();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{duplex, AsyncWriteExt, DuplexStream};

    struct SeqIds {
        counter: AtomicUsize,
    }

    impl SeqIds {
        fn new() -> Self {
            Self {
                counter: AtomicUsize::new(0),
            }
        }
    }

    impl ExecIdSource for SeqIds {
        fn next_exec_id(&self) -> Result<String, ExecError> {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(format!("exec-{n:08}"))
        }
    }

    struct FakeKiller {
        killed: Arc<AtomicU64>,
    }

    impl ProcessKiller for FakeKiller {
        fn kill_group(&self) {
            self.killed.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeWaiter {
        exit_rx: Option<tokio::sync::oneshot::Receiver<ExitOutcome>>,
        reaped: Arc<AtomicU64>,
    }

    #[async_trait]
    impl ProcessWaiter for FakeWaiter {
        async fn wait(&mut self) -> ExitOutcome {
            let outcome = match self.exit_rx.take() {
                Some(rx) => rx.await.unwrap_or(ExitOutcome::Exited(0)),
                // No exit channel: keep the child "running" until cancelled.
                None => std::future::pending::<ExitOutcome>().await,
            };
            self.reaped.fetch_add(1, Ordering::SeqCst);
            outcome
        }
    }

    impl Drop for FakeWaiter {
        fn drop(&mut self) {
            // Mirror kill_on_drop reaping for the ceiling/abort paths.
            self.reaped.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct SpawnControls {
        stdout: DuplexStream,
        stderr: DuplexStream,
        exit_rx: tokio::sync::oneshot::Receiver<ExitOutcome>,
    }

    struct FakeSpawner {
        controls: Mutex<Option<SpawnControls>>,
        killed: Arc<AtomicU64>,
        reaped: Arc<AtomicU64>,
        fail: bool,
    }

    struct SpawnHooks {
        stdout: Option<DuplexStream>,
        stderr: Option<DuplexStream>,
        exit_tx: Option<tokio::sync::oneshot::Sender<ExitOutcome>>,
        killed: Arc<AtomicU64>,
        reaped: Arc<AtomicU64>,
    }

    impl FakeSpawner {
        fn new() -> (Self, SpawnHooks) {
            // Guest child writes; test reads via the *_w ends in hooks.
            let (stdout_w, stdout_r) = duplex(64 * 1024);
            let (stderr_w, stderr_r) = duplex(64 * 1024);
            let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
            let killed = Arc::new(AtomicU64::new(0));
            let reaped = Arc::new(AtomicU64::new(0));
            let spawner = Self {
                controls: Mutex::new(Some(SpawnControls {
                    stdout: stdout_r,
                    stderr: stderr_r,
                    exit_rx,
                })),
                killed: Arc::clone(&killed),
                reaped: Arc::clone(&reaped),
                fail: false,
            };
            let hooks = SpawnHooks {
                stdout: Some(stdout_w),
                stderr: Some(stderr_w),
                exit_tx: Some(exit_tx),
                killed,
                reaped,
            };
            (spawner, hooks)
        }

        fn failing() -> Self {
            Self {
                controls: Mutex::new(None),
                killed: Arc::new(AtomicU64::new(0)),
                reaped: Arc::new(AtomicU64::new(0)),
                fail: true,
            }
        }
    }

    #[async_trait]
    impl ProcessSpawner for FakeSpawner {
        async fn spawn(&self, _command: ValidatedCommand) -> Result<SpawnedProcess, ExecError> {
            if self.fail {
                return Err(ExecError::SpawnFailed);
            }
            let controls = {
                let mut guard = self
                    .controls
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.take().expect("spawn called once")
            };
            Ok(SpawnedProcess {
                stdout: Box::new(controls.stdout),
                stderr: Box::new(controls.stderr),
                killer: Arc::new(FakeKiller {
                    killed: Arc::clone(&self.killed),
                }),
                waiter: Box::new(FakeWaiter {
                    exit_rx: Some(controls.exit_rx),
                    reaped: Arc::clone(&self.reaped),
                }),
            })
        }
    }

    fn good_input() -> ExecCreateInput {
        ExecCreateInput {
            argv: vec!["/bin/echo".to_owned(), "hi".to_owned()],
            user: Some("root".to_owned()),
            cwd: None,
            env: vec![],
            tty: false,
            stdin_open: false,
            detached: false,
            has_terminal_size: false,
            max_chunk_bytes: 64 * 1024,
        }
    }

    fn enabled_policy() -> ExecPolicy {
        ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        }
    }

    #[test]
    fn validate_rejects_unsupported_modes() {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        for mutate in [
            |i: &mut ExecCreateInput| i.tty = true,
            |i: &mut ExecCreateInput| i.detached = true,
            |i: &mut ExecCreateInput| i.stdin_open = true,
            |i: &mut ExecCreateInput| i.has_terminal_size = true,
        ] {
            let mut input = good_input();
            mutate(&mut input);
            assert_eq!(
                validate_and_authorize(&input, &policy),
                Err(ExecError::UnsupportedMode)
            );
        }
    }

    #[test]
    fn validate_ignores_wire_user_and_requires_configured_user() {
        // The target user is host-fixed (`policy.exec_user`); the wire `user`
        // field is never consulted, so a client cannot escalate to root or
        // pick another user. A configured workload user authorizes regardless
        // of the wire `user`; a missing workload user fails closed; disabled
        // policy fails closed.
        let enabled_user = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let enabled_no_user = ExecPolicy {
            enabled: true,
            exec_user: None,
        };
        let disabled = ExecPolicy::disabled();

        // Wire user = root is ignored: the configured workload user still wins.
        let mut root_req = good_input();
        root_req.user = Some("root".to_owned());
        assert!(validate_and_authorize(&root_req, &enabled_user).is_ok());

        // Wire user = some other name is ignored too.
        let mut other_req = good_input();
        other_req.user = Some("alice".to_owned());
        assert!(validate_and_authorize(&other_req, &enabled_user).is_ok());

        // Omitted wire user is fine — it is not consulted.
        let mut omitted = good_input();
        omitted.user = None;
        assert!(validate_and_authorize(&omitted, &enabled_user).is_ok());

        // No configured workload user => fail closed.
        assert_eq!(
            validate_and_authorize(&good_input(), &enabled_no_user),
            Err(ExecError::ExecDisabled)
        );
        // Disabled policy => fail closed.
        assert_eq!(
            validate_and_authorize(&good_input(), &disabled),
            Err(ExecError::ExecDisabled)
        );
    }

    #[test]
    fn detached_and_tty_validators_ignore_wire_user_and_require_configured_user() {
        // The detached and interactive-TTY validators MUST share the non-TTY
        // validator's authorization contract: the wire `user` field is never
        // consulted (no escalation to root or another user), a configured
        // workload user authorizes, and a missing workload user / disabled
        // policy fails closed.
        let enabled_user = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let enabled_no_user = ExecPolicy {
            enabled: true,
            exec_user: None,
        };
        let disabled = ExecPolicy::disabled();

        // Detached create input (detached=true, no interactive flags).
        let detached_input = || {
            let mut i = good_input();
            i.detached = true;
            i
        };
        // Interactive-TTY create input (tty=true, stdin open, terminal size).
        let tty_input = || {
            let mut i = good_input();
            i.tty = true;
            i.stdin_open = true;
            i.has_terminal_size = true;
            i
        };

        for (label, mk) in [
            ("detached", &detached_input as &dyn Fn() -> ExecCreateInput),
            ("tty", &tty_input as &dyn Fn() -> ExecCreateInput),
        ] {
            let validate = |input: &ExecCreateInput, policy: &ExecPolicy| {
                if label == "detached" {
                    validate_and_authorize_detached(input, policy)
                } else {
                    validate_and_authorize_tty(input, policy)
                }
            };

            // Wire user = root is ignored: the configured workload user wins.
            let mut root_req = mk();
            root_req.user = Some("root".to_owned());
            assert!(
                validate(&root_req, &enabled_user).is_ok(),
                "{label}: wire root user must be ignored, configured user authorizes"
            );

            // Wire user = some other name is ignored too.
            let mut other_req = mk();
            other_req.user = Some("alice".to_owned());
            assert!(
                validate(&other_req, &enabled_user).is_ok(),
                "{label}: wire alice user must be ignored"
            );

            // Omitted wire user is fine — it is not consulted.
            let mut omitted = mk();
            omitted.user = None;
            assert!(
                validate(&omitted, &enabled_user).is_ok(),
                "{label}: omitted wire user is fine"
            );

            // No configured workload user => fail closed.
            assert_eq!(
                validate(&mk(), &enabled_no_user),
                Err(ExecError::ExecDisabled),
                "{label}: missing workload user must fail closed"
            );
            // Disabled policy => fail closed.
            assert_eq!(
                validate(&mk(), &disabled),
                Err(ExecError::ExecDisabled),
                "{label}: disabled policy must fail closed"
            );
        }
    }

    #[test]
    fn validate_rejects_bad_command_shapes() {
        let policy = enabled_policy();

        let mut empty = good_input();
        empty.argv = vec![];
        assert_eq!(
            validate_and_authorize(&empty, &policy),
            Err(ExecError::InvalidArgv)
        );

        let mut empty_program = good_input();
        empty_program.argv = vec![String::new()];
        assert_eq!(
            validate_and_authorize(&empty_program, &policy),
            Err(ExecError::InvalidProgram)
        );

        let mut leading_dash_program = good_input();
        leading_dash_program.argv = vec!["-echo".to_owned()];
        assert_eq!(
            validate_and_authorize(&leading_dash_program, &policy),
            Err(ExecError::InvalidProgram)
        );

        let mut nul_program = good_input();
        nul_program.argv = vec!["echo\0bad".to_owned()];
        assert_eq!(
            validate_and_authorize(&nul_program, &policy),
            Err(ExecError::InvalidArgv)
        );

        let mut nul = good_input();
        nul.argv = vec!["/bin/echo".to_owned(), "a\0b".to_owned()];
        assert_eq!(
            validate_and_authorize(&nul, &policy),
            Err(ExecError::InvalidArgv)
        );

        let mut over_length_program = good_input();
        over_length_program.argv = vec!["x".repeat(MAX_ARG_BYTES + 1)];
        assert_eq!(
            validate_and_authorize(&over_length_program, &policy),
            Err(ExecError::InvalidArgv)
        );

        let mut too_many = good_input();
        too_many.argv = std::iter::once("/bin/echo".to_owned())
            .chain((0..MAX_ARGV).map(|_| "x".to_owned()))
            .collect();
        assert_eq!(
            validate_and_authorize(&too_many, &policy),
            Err(ExecError::InvalidArgv)
        );

        let mut rel_cwd = good_input();
        rel_cwd.cwd = Some("rel".to_owned());
        assert_eq!(
            validate_and_authorize(&rel_cwd, &policy),
            Err(ExecError::CwdInvalid)
        );

        let mut empty_cwd = good_input();
        empty_cwd.cwd = Some(String::new());
        assert_eq!(
            validate_and_authorize(&empty_cwd, &policy),
            Err(ExecError::CwdInvalid)
        );

        let mut bad_env = good_input();
        bad_env.env = vec![("1BAD".to_owned(), "v".to_owned())];
        assert_eq!(
            validate_and_authorize(&bad_env, &policy),
            Err(ExecError::InvalidEnv)
        );

        let mut dup_env = good_input();
        dup_env.env = vec![
            ("A".to_owned(), "1".to_owned()),
            ("A".to_owned(), "2".to_owned()),
        ];
        assert_eq!(
            validate_and_authorize(&dup_env, &policy),
            Err(ExecError::InvalidEnv)
        );

        let mut big_chunk = good_input();
        big_chunk.max_chunk_bytes = HARD_MAX_CHUNK_BYTES + 1;
        assert_eq!(
            validate_and_authorize(&big_chunk, &policy),
            Err(ExecError::MaxChunkExceeded)
        );
    }

    #[test]
    fn validate_accepts_absolute_bare_and_relative_program_names() {
        let policy = enabled_policy();

        for program in ["/bin/echo", "id", "./scripts/hello"] {
            let mut input = good_input();
            input.argv = vec![program.to_owned(), "arg".to_owned()];
            let command =
                validate_and_authorize(&input, &policy).expect("program name should validate");
            assert_eq!(command.program, std::path::PathBuf::from(program));
            assert_eq!(command.args, vec!["arg".to_owned()]);
        }
    }

    #[test]
    fn validate_detached_inherits_bare_program_relaxation() {
        let policy = enabled_policy();
        let mut input = good_input();
        input.detached = true;
        input.argv = vec!["id".to_owned()];

        let command = validate_and_authorize_detached(&input, &policy)
            .expect("detached validation should accept bare argv[0]");
        assert_eq!(command.program, std::path::PathBuf::from("id"));
        assert!(command.args.is_empty());
    }

    #[test]
    fn invalid_program_maps_to_distinct_wire_kind() {
        assert_eq!(
            ExecError::InvalidProgram.wire_kind(),
            WireErrorKind::InvalidProgram
        );
    }

    #[test]
    fn ring_drops_oldest_and_tracks_offsets() {
        let mut ring = OutputRing::new(4);
        ring.append(b"abcd");
        assert_eq!(ring.start_offset, 0);
        assert_eq!(ring.end_offset(), 4);
        ring.append(b"ef");
        assert_eq!(ring.start_offset, 2);
        assert_eq!(ring.end_offset(), 6);
        assert_eq!(ring.dropped_bytes, 2);
        assert!(ring.truncated);

        assert_eq!(ring.read(0, 10).unwrap_err(), ExecError::OffsetExpired);
        assert_eq!(ring.read(7, 10).unwrap_err(), ExecError::OffsetInFuture);
        let chunk = ring.read(2, 10).unwrap();
        assert_eq!(chunk.data, b"cdef");
        assert_eq!(chunk.next_offset, 6);
        assert!(!chunk.eof);
        ring.mark_eof();
        let chunk = ring.read(6, 10).unwrap();
        assert!(chunk.data.is_empty());
        assert!(chunk.eof);
    }

    // Parity: the guestd in-memory `OutputRing` (attached path) and the runner's
    // on-disk `FileRing` (detached path) MUST expose identical drop-oldest,
    // offset, dropped-byte, truncation, and EOF semantics. This compares the two
    // REAL implementations against each other (not a local model copy), so a
    // future divergence in either ring is caught.
    #[test]
    fn output_ring_and_runner_file_ring_have_identical_semantics() {
        use nixling_exec_runner::filering::{FileRing, FileRingReader};

        let unique = format!(
            "exec-parity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        // cwd-relative scratch (never /tmp); cleaned up at the end.
        let dir = std::path::PathBuf::from(".").join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.join("stdout");
        let side = dir.join("stdout.meta");

        let cap = 16u64;
        let mut file_ring = FileRing::create(&data, &side, cap).unwrap();
        let mut mem_ring = OutputRing::new(cap as usize);

        let appends: &[&[u8]] = &[
            b"abc",
            b"defgh",
            b"",
            b"ijklmnop",
            b"qrstuvwxyz0123456789",
            b"!",
        ];
        for chunk in appends {
            file_ring.append(chunk).unwrap();
            mem_ring.append(chunk);
            let reader = FileRingReader::open(&data, &side).unwrap();
            let end = mem_ring.end_offset();
            for off in [0u64, end.saturating_sub(5), end.saturating_sub(1), end] {
                for max in [0u64, 1, 7, 1024] {
                    let mem = mem_ring.read(off, max);
                    let file = reader.read(off, max);
                    match (mem, file) {
                        (Ok(m), Ok(f)) => {
                            assert_eq!(m.data, f.data, "data off={off} max={max}");
                            assert_eq!(m.start_offset, f.start_offset, "start off={off}");
                            assert_eq!(m.end_offset, f.end_offset, "end off={off}");
                            assert_eq!(m.next_offset, f.next_offset, "next off={off}");
                            assert_eq!(m.dropped_bytes, f.dropped_bytes, "dropped off={off}");
                            assert_eq!(m.truncated, f.truncated, "truncated off={off}");
                            assert_eq!(m.eof, f.eof, "eof off={off}");
                        }
                        // Both rings reject the same out-of-window offsets.
                        (Err(_), Err(_)) => {}
                        (m, f) => {
                            panic!("parity mismatch off={off} max={max}: mem={m:?} file={f:?}")
                        }
                    }
                }
            }
        }

        // EOF parity: only observable once the stream is fully drained.
        mem_ring.mark_eof();
        file_ring.mark_eof().unwrap();
        let reader = FileRingReader::open(&data, &side).unwrap();
        let end = mem_ring.end_offset();
        let mem_mid = mem_ring.read(mem_ring.start_offset, 1).unwrap();
        let file_mid = reader.read(mem_ring.start_offset, 1).unwrap();
        assert_eq!(mem_mid.eof, file_mid.eof, "eof not signalled mid-stream");
        let mem_end = mem_ring.read(end, 8).unwrap();
        let file_end = reader.read(end, 8).unwrap();
        assert!(mem_end.eof && file_end.eof, "eof at drain");
        assert_eq!(mem_end.data, file_end.data);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn runtime(policy: ExecPolicy) -> (ExecRuntime<FakeSpawner, SeqIds>, SpawnHooks) {
        let (spawner, hooks) = FakeSpawner::new();
        (ExecRuntime::new(spawner, SeqIds::new(), policy), hooks)
    }

    #[tokio::test]
    async fn create_streams_output_and_reports_exit() {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let (rt, mut hooks) = runtime(policy);
        let owner = b"conn-1".to_vec();
        let (exec_id, snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .expect("create");
        assert_eq!(snap.state, ExecState::Running);

        let mut stdout = hooks.stdout.take().unwrap();
        stdout.write_all(b"hello").await.unwrap();
        drop(stdout);
        let mut stderr = hooks.stderr.take().unwrap();
        stderr.write_all(b"warn").await.unwrap();
        drop(stderr);

        // Read stdout via long-poll.
        let (chunk, _timed_out) = rt
            .read_output(
                &owner,
                &exec_id,
                "boot-1",
                Stream::Stdout,
                0,
                1024,
                true,
                500,
            )
            .await
            .unwrap();
        assert_eq!(chunk.data, b"hello");

        // stderr is captured separately.
        let (chunk, _timed_out) = rt
            .read_output(
                &owner,
                &exec_id,
                "boot-1",
                Stream::Stderr,
                0,
                1024,
                true,
                500,
            )
            .await
            .unwrap();
        assert_eq!(chunk.data, b"warn");

        // Drive the child to exit; the supervisor publishes the terminal state.
        hooks
            .exit_tx
            .take()
            .unwrap()
            .send(ExitOutcome::Exited(7))
            .unwrap();
        let (snap, _timed_out) = rt
            .wait(&owner, &exec_id, "boot-1", None, 1000)
            .await
            .unwrap();
        // The supervisor applies a bounded drain grace before publishing; allow
        // a couple of long-poll rounds for the terminal transition.
        let mut snap = snap;
        for _ in 0..40 {
            if !matches!(snap.state, ExecState::Running) {
                break;
            }
            snap = rt
                .wait(
                    &owner,
                    &exec_id,
                    "boot-1",
                    Some(snap.state_generation),
                    1000,
                )
                .await
                .unwrap()
                .0;
        }
        assert_eq!(snap.state, ExecState::Exited);
        assert_eq!(snap.outcome, Some(ExitOutcome::Exited(7)));
        assert!(hooks.killed.load(Ordering::SeqCst) >= 1);
        assert!(hooks.reaped.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn cross_connection_access_is_denied() {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let (rt, _hooks) = runtime(policy);
        let owner = b"conn-A".to_vec();
        let (exec_id, _snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        let other = b"conn-B".to_vec();
        assert_eq!(
            rt.inspect(&other, &exec_id, "boot-1").unwrap_err(),
            ExecError::ExecNotFound
        );
        assert_eq!(
            rt.inspect(&owner, &exec_id, "boot-OTHER").unwrap_err(),
            ExecError::ExecNotFound
        );
    }

    #[tokio::test]
    async fn spawn_failure_leaves_no_state() {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let rt = ExecRuntime::new(FakeSpawner::failing(), SeqIds::new(), policy);
        let owner = b"conn-1".to_vec();
        let err = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::SpawnFailed);
        assert_eq!(rt.tracked_len(), 0);
    }

    #[tokio::test]
    async fn close_connection_cancels_and_forgets() {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let (rt, _hooks) = runtime(policy);
        let owner = b"conn-1".to_vec();
        let (exec_id, _snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        rt.close_connection(&owner);
        assert_eq!(rt.tracked_len(), 0);
        assert_eq!(
            rt.inspect(&owner, &exec_id, "boot-1").unwrap_err(),
            ExecError::ExecNotFound
        );
    }

    /// Spawns processes that never exit and produce no output, for capacity and
    /// timeout tests. Each spawn is independent and idempotent.
    struct PendingSpawner {
        killed: Arc<AtomicU64>,
    }

    #[async_trait]
    impl ProcessSpawner for PendingSpawner {
        async fn spawn(&self, _command: ValidatedCommand) -> Result<SpawnedProcess, ExecError> {
            let (_w_out, r_out) = duplex(1024);
            let (_w_err, r_err) = duplex(1024);
            // Leak the write ends so the pipes never reach EOF (process stays
            // "running" until cancelled).
            std::mem::forget(_w_out);
            std::mem::forget(_w_err);
            Ok(SpawnedProcess {
                stdout: Box::new(r_out),
                stderr: Box::new(r_err),
                killer: Arc::new(FakeKiller {
                    killed: Arc::clone(&self.killed),
                }),
                waiter: Box::new(FakeWaiter {
                    exit_rx: None,
                    reaped: Arc::new(AtomicU64::new(0)),
                }),
            })
        }
    }

    fn pending_runtime() -> ExecRuntime<PendingSpawner, SeqIds> {
        pending_runtime_with_kills().0
    }

    fn pending_runtime_with_kills() -> (ExecRuntime<PendingSpawner, SeqIds>, Arc<AtomicU64>) {
        let killed = Arc::new(AtomicU64::new(0));
        let rt = ExecRuntime::new(
            PendingSpawner {
                killed: Arc::clone(&killed),
            },
            SeqIds::new(),
            ExecPolicy {
                enabled: true,
                exec_user: Some("john".to_owned()),
            },
        );
        (rt, killed)
    }

    #[tokio::test]
    async fn close_connection_kills_process_group() {
        let (rt, killed) = pending_runtime_with_kills();
        let owner = b"conn-1".to_vec();
        rt.create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        assert_eq!(killed.load(Ordering::SeqCst), 0);
        rt.close_connection(&owner);
        // The lock-free killer must have signalled the group even though the
        // supervisor was parked in wait() at disconnect time.
        assert!(killed.load(Ordering::SeqCst) >= 1);
        assert_eq!(rt.tracked_len(), 0);
    }

    #[tokio::test]
    async fn attach_capacity_is_enforced() {
        let rt = pending_runtime();
        let owner = b"conn-1".to_vec();
        for _ in 0..ATTACHED_SESSIONS_PER_VM {
            rt.create(owner.clone(), "boot-1".to_owned(), good_input())
                .await
                .unwrap();
        }
        let err = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::AttachCapacityExceeded);
    }

    #[tokio::test]
    async fn wait_times_out_for_running_exec() {
        let rt = pending_runtime();
        let owner = b"conn-1".to_vec();
        let (exec_id, snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        let (snap, timed_out) = rt
            .wait(&owner, &exec_id, "boot-1", Some(snap.state_generation), 50)
            .await
            .unwrap();
        assert!(timed_out);
        assert_eq!(snap.state, ExecState::Running);
    }

    #[tokio::test]
    async fn read_output_reports_future_offset() {
        let rt = pending_runtime();
        let owner = b"conn-1".to_vec();
        let (exec_id, _snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        let err = rt
            .read_output(
                &owner,
                &exec_id,
                "boot-1",
                Stream::Stdout,
                100,
                1024,
                false,
                0,
            )
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::OffsetInFuture);

        // A non-waiting read at the current (empty) window returns no data.
        let (chunk, timed_out) = rt
            .read_output(
                &owner,
                &exec_id,
                "boot-1",
                Stream::Stdout,
                0,
                1024,
                false,
                0,
            )
            .await
            .unwrap();
        assert!(chunk.data.is_empty());
        assert!(!timed_out);
    }

    #[tokio::test]
    async fn read_output_long_poll_times_out_empty() {
        let rt = pending_runtime();
        let owner = b"conn-1".to_vec();
        let (exec_id, _snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        let (chunk, timed_out) = rt
            .read_output(
                &owner,
                &exec_id,
                "boot-1",
                Stream::Stdout,
                0,
                1024,
                true,
                50,
            )
            .await
            .unwrap();
        assert!(chunk.data.is_empty());
        assert!(timed_out);
    }

    #[tokio::test]
    async fn cancelled_wait_releases_capacity() {
        let rt = pending_runtime();
        let owner = b"conn-1".to_vec();
        let (exec_id, snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        let gen = snap.state_generation;
        // Cancel many in-flight waits by dropping the future mid-park. Passing
        // the current generation makes each wait actually park (the state is
        // unchanged), so the timeout cancels a parked future. Without the
        // CounterGuard the VM-scoped counter would leak and exhaust capacity;
        // with it, capacity is always released.
        for _ in 0..(PENDING_EXEC_WAITS_PER_VM + 5) {
            let fut = rt.wait(&owner, &exec_id, "boot-1", Some(gen), 1000);
            let _ = tokio::time::timeout(Duration::from_millis(1), fut).await;
        }
        // A fresh wait must still be admitted (it times out, not capacity-fails).
        let (_snap, timed_out) = rt
            .wait(&owner, &exec_id, "boot-1", Some(gen), 30)
            .await
            .unwrap();
        assert!(timed_out);
    }

    /// Spawner that parks inside `spawn` until released, so a test can drive a
    /// disconnect that races an in-flight create.
    struct GatedSpawner {
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        killed: Arc<AtomicU64>,
    }

    #[async_trait]
    impl ProcessSpawner for GatedSpawner {
        async fn spawn(&self, _command: ValidatedCommand) -> Result<SpawnedProcess, ExecError> {
            if let Some(tx) = self
                .entered
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                let _ = tx.send(());
            }
            let rx = self.release.lock().await.take();
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            let (_w_out, r_out) = duplex(1024);
            let (_w_err, r_err) = duplex(1024);
            std::mem::forget(_w_out);
            std::mem::forget(_w_err);
            Ok(SpawnedProcess {
                stdout: Box::new(r_out),
                stderr: Box::new(r_err),
                killer: Arc::new(FakeKiller {
                    killed: Arc::clone(&self.killed),
                }),
                waiter: Box::new(FakeWaiter {
                    exit_rx: None,
                    reaped: Arc::new(AtomicU64::new(0)),
                }),
            })
        }
    }

    #[tokio::test]
    async fn create_during_disconnect_is_torn_down() {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let killed = Arc::new(AtomicU64::new(0));
        let rt = Arc::new(ExecRuntime::new(
            GatedSpawner {
                entered: Mutex::new(Some(entered_tx)),
                release: tokio::sync::Mutex::new(Some(release_rx)),
                killed: Arc::clone(&killed),
            },
            SeqIds::new(),
            ExecPolicy {
                enabled: true,
                exec_user: Some("john".to_owned()),
            },
        ));
        let owner = b"conn-1".to_vec();

        let rt_task = Arc::clone(&rt);
        let owner_task = owner.clone();
        let create = tokio::spawn(async move {
            rt_task
                .create(owner_task, "boot-1".to_owned(), good_input())
                .await
        });

        // Wait until create is parked inside spawn (placeholder inserted).
        entered_rx.await.unwrap();
        assert_eq!(rt.tracked_len(), 1);
        // Disconnect mid-create: the placeholder is cancelled and removed.
        rt.close_connection(&owner);
        // Let spawn finish; create must observe the closed connection.
        release_tx.send(()).unwrap();

        let result = create.await.unwrap();
        assert!(result.is_err());
        assert_eq!(rt.tracked_len(), 0);
        // The just-spawned process group was torn down.
        assert!(killed.load(Ordering::SeqCst) >= 1);
    }

    // ---- interactive TTY exec: fake-driven runtime matrix ----

    use crate::exec_pty::{PtyControl, SessionReaper, TtySignal, VEOF};
    use tokio::io::AsyncReadExt;

    /// Fake control surface recording the resize geometries, foreground signals,
    /// and the initial geometry the helper would have applied at spawn.
    struct FakePtyControl {
        resizes: Mutex<Vec<TerminalSize>>,
        signals: Mutex<Vec<TtySignal>>,
        initial: Mutex<Option<TerminalSize>>,
    }

    impl FakePtyControl {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                resizes: Mutex::new(Vec::new()),
                signals: Mutex::new(Vec::new()),
                initial: Mutex::new(None),
            })
        }

        fn resizes(&self) -> Vec<TerminalSize> {
            self.resizes
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }

        fn signals(&self) -> Vec<TtySignal> {
            self.signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }

        fn initial(&self) -> Option<TerminalSize> {
            *self.initial.lock().unwrap_or_else(|p| p.into_inner())
        }
    }

    impl PtyControl for FakePtyControl {
        fn resize(&self, size: TerminalSize) {
            self.resizes
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(size);
        }

        fn signal_foreground(&self, signal: TtySignal) {
            self.signals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(signal);
        }
    }

    struct FakeSessionReaper {
        kills: Arc<AtomicU64>,
    }

    impl SessionReaper for FakeSessionReaper {
        fn kill_session(&self) {
            self.kills.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct PtySpawnControls {
        reader: DuplexStream,
        writer: DuplexStream,
        exit_rx: tokio::sync::oneshot::Receiver<ExitOutcome>,
    }

    /// Single-shot fake PTY spawner backing the deterministic TTY runtime tests:
    /// the master read/write halves are duplex streams the test drives directly.
    struct FakePtySpawner {
        controls: Mutex<Option<PtySpawnControls>>,
        control: Arc<FakePtyControl>,
        kills: Arc<AtomicU64>,
        reaped: Arc<AtomicU64>,
    }

    /// Test-side handles for a [`FakePtySpawner`] session.
    struct PtyHooks {
        /// Test writes merged PTY output here; the runtime reads it as `reader`.
        output_w: Option<DuplexStream>,
        /// Test reads the stdin bytes the runtime wrote to the master here.
        stdin_r: Option<DuplexStream>,
        /// Drives the direct child's exit.
        exit_tx: Option<tokio::sync::oneshot::Sender<ExitOutcome>>,
        control: Arc<FakePtyControl>,
        kills: Arc<AtomicU64>,
    }

    impl FakePtySpawner {
        fn new() -> (Arc<Self>, PtyHooks) {
            let (test_output, guest_output) = duplex(64 * 1024);
            let (guest_stdin, test_stdin) = duplex(64 * 1024);
            let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
            let control = FakePtyControl::new();
            let kills = Arc::new(AtomicU64::new(0));
            let reaped = Arc::new(AtomicU64::new(0));
            let spawner = Arc::new(Self {
                controls: Mutex::new(Some(PtySpawnControls {
                    reader: guest_output,
                    writer: guest_stdin,
                    exit_rx,
                })),
                control: Arc::clone(&control),
                kills: Arc::clone(&kills),
                reaped,
            });
            let hooks = PtyHooks {
                output_w: Some(test_output),
                stdin_r: Some(test_stdin),
                exit_tx: Some(exit_tx),
                control,
                kills,
            };
            (spawner, hooks)
        }
    }

    #[async_trait]
    impl PtyProcessSpawner for FakePtySpawner {
        async fn spawn(
            &self,
            _command: ValidatedCommand,
            initial_size: TerminalSize,
        ) -> Result<SpawnedPtyProcess, ExecError> {
            *self
                .control
                .initial
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(initial_size);
            let controls = self
                .controls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .expect("spawn called once");
            Ok(SpawnedPtyProcess {
                reader: Box::new(controls.reader),
                writer: Box::new(controls.writer),
                control: Arc::clone(&self.control) as Arc<dyn PtyControl>,
                waiter: Box::new(FakeWaiter {
                    exit_rx: Some(controls.exit_rx),
                    reaped: Arc::clone(&self.reaped),
                }),
                reaper: Arc::new(FakeSessionReaper {
                    kills: Arc::clone(&self.kills),
                }),
            })
        }
    }

    /// Multi-spawn fake PTY spawner for capacity tests: every spawn yields a
    /// fresh never-exiting session (leaked test-side duplex ends keep the master
    /// halves open). `allocs` counts how many times a PTY was actually
    /// allocated, so a capacity test can assert an over-cap create fails BEFORE
    /// any PTY/helper allocation.
    struct MultiPtySpawner {
        kills: Arc<AtomicU64>,
        reaped: Arc<AtomicU64>,
        allocs: Arc<AtomicU64>,
    }

    impl MultiPtySpawner {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                kills: Arc::new(AtomicU64::new(0)),
                reaped: Arc::new(AtomicU64::new(0)),
                allocs: Arc::new(AtomicU64::new(0)),
            })
        }
    }

    #[async_trait]
    impl PtyProcessSpawner for MultiPtySpawner {
        async fn spawn(
            &self,
            _command: ValidatedCommand,
            _initial_size: TerminalSize,
        ) -> Result<SpawnedPtyProcess, ExecError> {
            self.allocs.fetch_add(1, Ordering::SeqCst);
            let (test_o, guest_o) = duplex(1024);
            let (guest_i, test_i) = duplex(1024);
            std::mem::forget(test_o);
            std::mem::forget(test_i);
            Ok(SpawnedPtyProcess {
                reader: Box::new(guest_o),
                writer: Box::new(guest_i),
                control: FakePtyControl::new(),
                waiter: Box::new(FakeWaiter {
                    exit_rx: None,
                    reaped: Arc::clone(&self.reaped),
                }),
                reaper: Arc::new(FakeSessionReaper {
                    kills: Arc::clone(&self.kills),
                }),
            })
        }
    }

    fn tty_input() -> ExecCreateInput {
        ExecCreateInput {
            argv: vec!["/bin/sh".to_owned()],
            user: Some("root".to_owned()),
            cwd: None,
            env: vec![],
            tty: true,
            stdin_open: true,
            detached: false,
            has_terminal_size: false,
            max_chunk_bytes: 64 * 1024,
        }
    }

    fn tty_runtime(ceiling: Option<Duration>) -> (ExecRuntime<FakeSpawner, SeqIds>, PtyHooks) {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let (pty, hooks) = FakePtySpawner::new();
        let (base, _non_tty_hooks) = FakeSpawner::new();
        let rt = ExecRuntime::new(base, SeqIds::new(), policy)
            .with_pty_spawner(pty)
            .with_interactive_ceiling(ceiling)
            .with_tty_grace(Duration::from_millis(1));
        (rt, hooks)
    }

    #[tokio::test]
    async fn tty_create_streams_merged_output_and_rejects_stderr() {
        let (rt, mut hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, snap, seq) = rt
            .create_tty(
                owner.clone(),
                "boot-1".to_owned(),
                tty_input(),
                Some((40, 120)),
            )
            .await
            .expect("create_tty");
        assert_eq!(seq, 0);
        assert_eq!(snap.state, ExecState::Running);
        assert_eq!(
            hooks.control.initial(),
            Some(TerminalSize {
                rows: 40,
                cols: 120
            })
        );

        let mut output = hooks.output_w.take().unwrap();
        output.write_all(b"merged-out").await.unwrap();

        let (chunk, _timed_out) = rt
            .read_output(
                &owner,
                &exec_id,
                "boot-1",
                Stream::Stdout,
                0,
                1024,
                true,
                500,
            )
            .await
            .unwrap();
        assert_eq!(chunk.data, b"merged-out");

        // Stderr is never produced for a TTY exec; the merged stream is stdout.
        assert_eq!(
            rt.read_output(
                &owner,
                &exec_id,
                "boot-1",
                Stream::Stderr,
                0,
                1024,
                false,
                0
            )
            .await
            .unwrap_err(),
            ExecError::TtyStderrUnavailable
        );
        // Keep the output writer alive until the assertions complete.
        drop(output);
    }

    #[tokio::test]
    async fn tty_create_defaults_terminal_size_when_absent() {
        let (rt, hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        rt.create_tty(owner, "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        assert_eq!(hooks.control.initial(), Some(TerminalSize::defaulted()));
    }

    #[tokio::test]
    async fn tty_write_stdin_offset_machine_and_close_after_veof() {
        let (rt, mut hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        let mut stdin_r = hooks.stdin_r.take().unwrap();

        // Contiguous write at offset 0.
        let out = rt
            .write_stdin(&owner, &exec_id, "boot-1", 0, b"abc", false)
            .await
            .unwrap();
        assert_eq!(out.accepted_len, 3);
        assert_eq!(out.next_offset, 3);
        assert!(!out.closed);
        // Replayed / stale offset is rejected.
        assert_eq!(
            rt.write_stdin(&owner, &exec_id, "boot-1", 0, b"x", false)
                .await
                .unwrap_err(),
            ExecError::StdinOffsetMismatch
        );
        // Future / out-of-order offset is rejected.
        assert_eq!(
            rt.write_stdin(&owner, &exec_id, "boot-1", 5, b"x", false)
                .await
                .unwrap_err(),
            ExecError::StdinOffsetMismatch
        );
        // close_after writes the data then injects VEOF.
        let out = rt
            .write_stdin(&owner, &exec_id, "boot-1", 3, b"de", true)
            .await
            .unwrap();
        assert_eq!(out.accepted_len, 2);
        assert_eq!(out.next_offset, 5);
        assert!(out.closed);
        // Any further write after close is rejected.
        assert_eq!(
            rt.write_stdin(&owner, &exec_id, "boot-1", 5, b"z", false)
                .await
                .unwrap_err(),
            ExecError::StdinClosed
        );

        let mut buf = vec![0u8; 6];
        stdin_r.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"abcde\x04");
    }

    #[tokio::test]
    async fn tty_close_stdin_injects_veof_and_is_idempotent() {
        let (rt, mut hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        let mut stdin_r = hooks.stdin_r.take().unwrap();

        let (final_offset, duplicate) =
            rt.close_stdin(&owner, &exec_id, "boot-1", 0).await.unwrap();
        assert_eq!(final_offset, 0);
        assert!(!duplicate);
        // Second close is an idempotent duplicate (no second VEOF).
        let (final_offset2, duplicate2) =
            rt.close_stdin(&owner, &exec_id, "boot-1", 0).await.unwrap();
        assert_eq!(final_offset2, 0);
        assert!(duplicate2);

        let mut one = [0u8; 1];
        stdin_r.read_exact(&mut one).await.unwrap();
        assert_eq!(one[0], VEOF);

        // The idempotent duplicate close must NOT inject a second VEOF: a bounded
        // read for another byte times out (nothing more is on the master).
        let mut extra = [0u8; 1];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), stdin_r.read_exact(&mut extra))
                .await
                .is_err(),
            "a duplicate CloseStdin must not write a second VEOF"
        );

        // A WriteStdin after close is rejected.
        assert_eq!(
            rt.write_stdin(&owner, &exec_id, "boot-1", 0, b"z", false)
                .await
                .unwrap_err(),
            ExecError::StdinClosed
        );
    }

    #[tokio::test]
    async fn tty_resize_and_signal_respect_control_seq_and_allowlist() {
        let (rt, hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();

        // resize at seq=1 is applied.
        rt.tty_resize(&owner, &exec_id, "boot-1", 1, 50, 100)
            .unwrap();
        // A stale/duplicate seq on the shared dispatcher is rejected.
        assert_eq!(
            rt.tty_resize(&owner, &exec_id, "boot-1", 1, 10, 10)
                .unwrap_err(),
            ExecError::ControlSeqMismatch
        );
        // signal at seq=2 (SIGINT) is delivered.
        rt.tty_signal(&owner, &exec_id, "boot-1", 2, 2).unwrap();
        // Replayed seq=2 is rejected.
        assert_eq!(
            rt.tty_signal(&owner, &exec_id, "boot-1", 2, 15)
                .unwrap_err(),
            ExecError::ControlSeqMismatch
        );
        // Out-of-allowlist signal at a fresh seq is rejected.
        assert_eq!(
            rt.tty_signal(&owner, &exec_id, "boot-1", 3, 11)
                .unwrap_err(),
            ExecError::InvalidSignal
        );
        // Invalid geometry at a fresh seq is rejected.
        assert_eq!(
            rt.tty_resize(&owner, &exec_id, "boot-1", 4, 0, 80)
                .unwrap_err(),
            ExecError::InvalidTerminalSize
        );

        assert_eq!(
            hooks.control.resizes(),
            vec![TerminalSize {
                rows: 50,
                cols: 100
            }]
        );
        assert_eq!(hooks.control.signals(), vec![TtySignal::Int]);
    }

    #[tokio::test]
    async fn tty_create_without_spawner_is_unsupported() {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let (base, _hooks) = FakeSpawner::new();
        let rt = ExecRuntime::new(base, SeqIds::new(), policy);
        assert!(!rt.tty_usable());
        let owner = b"c1".to_vec();
        let err = rt
            .create_tty(owner, "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::UnsupportedMode);
    }

    #[tokio::test]
    async fn tty_child_exit_publishes_terminal_and_reaps_session() {
        let (rt, mut hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();

        hooks
            .exit_tx
            .take()
            .unwrap()
            .send(ExitOutcome::Exited(3))
            .unwrap();

        let mut snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        for _ in 0..200 {
            if !matches!(snap.state, ExecState::Running) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        }
        assert_eq!(snap.state, ExecState::Exited);
        assert_eq!(snap.outcome, Some(ExitOutcome::Exited(3)));
        assert!(hooks.kills.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn tty_disconnect_cancels_session() {
        let (rt, hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        rt.close_connection(&owner);
        // The entry is forgotten from tracking immediately on disconnect.
        assert_eq!(
            rt.inspect(&owner, &exec_id, "boot-1").unwrap_err(),
            ExecError::ExecNotFound
        );
        // Teardown is spawned; wait for the session SIGKILL sweep.
        for _ in 0..200 {
            if hooks.kills.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(hooks.kills.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn tty_runtime_ceiling_tears_down_when_exceeded() {
        let (rt, _hooks) = tty_runtime(Some(Duration::from_millis(10)));
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        // The child never exits; the per-session ceiling fires teardown.
        let mut snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        for _ in 0..400 {
            if !matches!(snap.state, ExecState::Running) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        }
        assert_eq!(snap.state, ExecState::Cancelled);
    }

    #[tokio::test]
    async fn tty_create_shares_attached_capacity() {
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let (base, _hooks) = FakeSpawner::new();
        let spawner = MultiPtySpawner::new();
        let allocs = Arc::clone(&spawner.allocs);
        let rt = ExecRuntime::new(base, SeqIds::new(), policy)
            .with_pty_spawner(spawner)
            .with_tty_grace(Duration::from_millis(1));
        let owner = b"c1".to_vec();
        for _ in 0..ATTACHED_SESSIONS_PER_VM {
            rt.create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
                .await
                .unwrap();
        }
        assert_eq!(
            allocs.load(Ordering::SeqCst),
            ATTACHED_SESSIONS_PER_VM as u64
        );
        // The interactive path shares the attached-session capacity.
        assert_eq!(
            rt.create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
                .await
                .unwrap_err(),
            ExecError::AttachCapacityExceeded
        );
        // The over-cap create fails BEFORE any PTY/helper allocation: the
        // spawn-count is unchanged, so there is no half-created session.
        assert_eq!(
            allocs.load(Ordering::SeqCst),
            ATTACHED_SESSIONS_PER_VM as u64
        );
    }

    #[tokio::test]
    async fn total_exec_cap_counts_terminal_execs_and_blocks_both_paths() {
        // The EXEC_SESSIONS_PER_VM ceiling is the TOTAL retained-session cap and
        // counts terminal (exited/cancelled) execs too — distinct from the
        // ATTACHED_SESSIONS_PER_VM cap, which counts only running sessions. Fill
        // the runtime with exactly EXEC_SESSIONS_PER_VM terminal entries so the
        // running count stays 0 (well under the attached cap): the next create —
        // TTY or non-TTY — must fail with ExecCapacityExceeded, NOT
        // AttachCapacityExceeded, and the rejected TTY create must allocate no
        // PTY (it fails before the spawner is reached).
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let (base, _hooks) = FakeSpawner::new();
        let spawner = MultiPtySpawner::new();
        let allocs = Arc::clone(&spawner.allocs);
        let rt = ExecRuntime::new(base, SeqIds::new(), policy)
            .with_pty_spawner(spawner)
            .with_tty_grace(Duration::from_millis(1));
        let owner = b"c1".to_vec();

        // Directly seat EXEC_SESSIONS_PER_VM terminal entries (no spawner, no
        // PTY): they occupy the total cap while contributing 0 to the running
        // count.
        {
            let mut execs = rt.lock_execs();
            for i in 0..EXEC_SESSIONS_PER_VM {
                let entry =
                    new_exec_entry(owner.clone(), "boot-1".to_owned(), Arc::new(NoopKiller));
                entry.lock_shared().state = ExecState::Exited;
                execs.insert(format!("term-{i}"), entry);
            }
        }
        assert_eq!(rt.tracked_len(), EXEC_SESSIONS_PER_VM);

        // Non-TTY create trips the TOTAL cap (running is 0, so it is NOT the
        // attached cap that fires).
        assert_eq!(
            rt.create(owner.clone(), "boot-1".to_owned(), good_input())
                .await
                .unwrap_err(),
            ExecError::ExecCapacityExceeded
        );
        // TTY create trips the same total cap and allocates no PTY.
        assert_eq!(
            rt.create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
                .await
                .unwrap_err(),
            ExecError::ExecCapacityExceeded
        );
        assert_eq!(allocs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn attached_cap_is_shared_across_mixed_tty_and_non_tty_execs() {
        // The attached-session cap is shared: a mix of running non-TTY execs and
        // running interactive TTY sessions counts against ONE ceiling. Interleave
        // 4 non-TTY + 4 TTY running sessions to exactly fill
        // ATTACHED_SESSIONS_PER_VM (8), then prove the 9th — of either kind —
        // fails with AttachCapacityExceeded, and the rejected TTY create
        // allocates no PTY.
        assert_eq!(ATTACHED_SESSIONS_PER_VM, 8, "test interleave assumes cap 8");
        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let killed = Arc::new(AtomicU64::new(0));
        let base = PendingSpawner {
            killed: Arc::clone(&killed),
        };
        let spawner = MultiPtySpawner::new();
        let allocs = Arc::clone(&spawner.allocs);
        let rt = ExecRuntime::new(base, SeqIds::new(), policy)
            .with_pty_spawner(spawner)
            .with_tty_grace(Duration::from_millis(1));
        let owner = b"c1".to_vec();

        // 4 non-TTY + 4 TTY, interleaved → 8 running sessions.
        for _ in 0..4 {
            rt.create(owner.clone(), "boot-1".to_owned(), good_input())
                .await
                .unwrap();
            rt.create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
                .await
                .unwrap();
        }
        assert_eq!(rt.tracked_len(), ATTACHED_SESSIONS_PER_VM);
        assert_eq!(
            allocs.load(Ordering::SeqCst),
            4,
            "exactly the 4 TTY sessions"
        );

        // The 9th attached session is refused regardless of kind, and the TTY
        // refusal allocates no PTY.
        assert_eq!(
            rt.create(owner.clone(), "boot-1".to_owned(), good_input())
                .await
                .unwrap_err(),
            ExecError::AttachCapacityExceeded
        );
        assert_eq!(
            rt.create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
                .await
                .unwrap_err(),
            ExecError::AttachCapacityExceeded
        );
        assert_eq!(
            allocs.load(Ordering::SeqCst),
            4,
            "over-cap TTY create did not alloc"
        );
    }

    #[tokio::test]
    async fn tty_inspect_is_tty_aware_for_stdin_and_control_seq() {
        // A live TTY exec reports OPEN stdin + the highest admitted control
        // seq; after CloseStdin it reports CLOSED. (Non-TTY execs keep CLOSED +
        // seq 0, covered by the detached/attached paths.)
        let (rt, _hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();

        let snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        assert_eq!(snap.stdin, TtyStdinSnapshot::Open);
        assert_eq!(snap.last_control_seq, 0);

        rt.tty_resize(&owner, &exec_id, "boot-1", 7, 40, 80)
            .unwrap();
        let snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        assert_eq!(snap.stdin, TtyStdinSnapshot::Open);
        assert_eq!(snap.last_control_seq, 7);

        rt.close_stdin(&owner, &exec_id, "boot-1", 0).await.unwrap();
        let snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        assert_eq!(snap.stdin, TtyStdinSnapshot::Closed);
        assert_eq!(snap.last_control_seq, 7);
    }

    #[tokio::test]
    async fn tty_terminal_size_inclusive_boundaries() {
        // Initial size at the inclusive boundaries 1 and 65535 is accepted.
        for size in [(1u32, 1u32), (65535u32, 65535u32)] {
            let (rt, hooks) = tty_runtime(None);
            let owner = b"c1".to_vec();
            let mut input = tty_input();
            input.has_terminal_size = true;
            rt.create_tty(owner, "boot-1".to_owned(), input, Some(size))
                .await
                .unwrap();
            assert_eq!(
                hooks.control.initial(),
                Some(TerminalSize {
                    rows: size.0 as u16,
                    cols: size.1 as u16
                })
            );
        }

        // Resize at the inclusive boundaries is accepted; just outside is rejected.
        let (rt, hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        rt.tty_resize(&owner, &exec_id, "boot-1", 1, 1, 1).unwrap();
        rt.tty_resize(&owner, &exec_id, "boot-1", 2, 65535, 65535)
            .unwrap();
        assert_eq!(
            rt.tty_resize(&owner, &exec_id, "boot-1", 3, 0, 80)
                .unwrap_err(),
            ExecError::InvalidTerminalSize
        );
        assert_eq!(
            rt.tty_resize(&owner, &exec_id, "boot-1", 4, 65536, 80)
                .unwrap_err(),
            ExecError::InvalidTerminalSize
        );
        assert_eq!(
            hooks.control.resizes(),
            vec![
                TerminalSize { rows: 1, cols: 1 },
                TerminalSize {
                    rows: 65535,
                    cols: 65535
                }
            ]
        );
    }

    #[tokio::test]
    async fn tty_rpcs_from_non_owner_are_rejected_with_no_side_effect() {
        // An authenticated-but-not-owner connection cannot drive another
        // connection's TTY exec: every RPC is ExecNotFound with NO PTY/VEOF/
        // resize/signal side effect.
        let (rt, mut hooks) = tty_runtime(None);
        let owner = b"owner".to_vec();
        let intruder = b"intruder".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        let mut stdin_r = hooks.stdin_r.take().unwrap();

        assert_eq!(
            rt.write_stdin(&intruder, &exec_id, "boot-1", 0, b"x", false)
                .await
                .unwrap_err(),
            ExecError::ExecNotFound
        );
        assert_eq!(
            rt.close_stdin(&intruder, &exec_id, "boot-1", 0)
                .await
                .unwrap_err(),
            ExecError::ExecNotFound
        );
        assert_eq!(
            rt.tty_resize(&intruder, &exec_id, "boot-1", 1, 40, 80)
                .unwrap_err(),
            ExecError::ExecNotFound
        );
        assert_eq!(
            rt.tty_signal(&intruder, &exec_id, "boot-1", 1, 2)
                .unwrap_err(),
            ExecError::ExecNotFound
        );

        // No side effects: no resize/signal recorded, and no stdin bytes/VEOF.
        assert!(hooks.control.resizes().is_empty());
        assert!(hooks.control.signals().is_empty());
        let owner_snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        assert_eq!(owner_snap.last_control_seq, 0);
        assert_eq!(owner_snap.stdin, TtyStdinSnapshot::Open);
        // The owner can still write at offset 0 (the intruder never advanced it).
        let out = rt
            .write_stdin(&owner, &exec_id, "boot-1", 0, b"ok", false)
            .await
            .unwrap();
        assert_eq!(out.next_offset, 2);
        let mut buf = [0u8; 2];
        stdin_r.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ok");
    }

    #[tokio::test(start_paused = true)]
    async fn tty_default_session_runs_past_the_non_tty_runtime_ceiling() {
        // A default interactive session has NO runtime ceiling (None): it must
        // stay Running well past the non-TTY MAX_EXEC_RUNTIME_MS (6h).
        let (rt, _hooks) = tty_runtime(None);
        let owner = b"c1".to_vec();
        let (exec_id, ..) = rt
            .create_tty(owner.clone(), "boot-1".to_owned(), tty_input(), None)
            .await
            .unwrap();
        tokio::time::advance(Duration::from_millis(MAX_EXEC_RUNTIME_MS * 2)).await;
        // Let any (erroneously scheduled) ceiling timer fire.
        tokio::task::yield_now().await;
        let snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        assert_eq!(
            snap.state,
            ExecState::Running,
            "an unlimited TTY session must outlive the non-TTY 6h ceiling"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn non_tty_session_is_still_cancelled_at_the_6h_ceiling() {
        // The non-TTY wall-clock ceiling (MAX_EXEC_RUNTIME_MS) must still
        // fire: a non-TTY exec whose child never exits on its own is Cancelled
        // once 6h elapse. This guards against the interactive-ceiling work
        // accidentally loosening the attached ceiling.
        //
        // A dedicated spawner models a real child: its `wait()` parks until the
        // ceiling's `kill_group()` fires, after which it reports `Signaled` — so
        // the supervisor's post-kill `wait()` actually returns (the generic
        // FakeWaiter would pend forever on the second wait).
        use tokio::sync::Notify;

        struct CeilingProbe {
            killed: Notify,
        }
        struct CeilingKiller(Arc<CeilingProbe>);
        impl ProcessKiller for CeilingKiller {
            fn kill_group(&self) {
                self.0.killed.notify_waiters();
                // Also store a permit for a waiter that has not yet parked.
                self.0.killed.notify_one();
            }
        }
        struct CeilingWaiter(Arc<CeilingProbe>);
        #[async_trait]
        impl ProcessWaiter for CeilingWaiter {
            async fn wait(&mut self) -> ExitOutcome {
                self.0.killed.notified().await;
                ExitOutcome::Signaled(9)
            }
        }
        struct CeilingSpawner {
            probe: Arc<CeilingProbe>,
        }
        #[async_trait]
        impl ProcessSpawner for CeilingSpawner {
            async fn spawn(&self, _command: ValidatedCommand) -> Result<SpawnedProcess, ExecError> {
                let (_t_o, g_o) = duplex(1024);
                let (_t_e, g_e) = duplex(1024);
                std::mem::forget(_t_o);
                std::mem::forget(_t_e);
                Ok(SpawnedProcess {
                    stdout: Box::new(g_o),
                    stderr: Box::new(g_e),
                    killer: Arc::new(CeilingKiller(Arc::clone(&self.probe))),
                    waiter: Box::new(CeilingWaiter(Arc::clone(&self.probe))),
                })
            }
        }

        let policy = ExecPolicy {
            enabled: true,
            exec_user: Some("john".to_owned()),
        };
        let spawner = CeilingSpawner {
            probe: Arc::new(CeilingProbe {
                killed: Notify::new(),
            }),
        };
        let rt = ExecRuntime::new(spawner, SeqIds::new(), policy);
        let owner = b"c1".to_vec();
        let (exec_id, _snap) = rt
            .create(owner.clone(), "boot-1".to_owned(), good_input())
            .await
            .unwrap();
        // Let the supervisor task run so it arms its 6h timeout BEFORE advancing
        // (a paused clock only fires already-registered timers).
        tokio::task::yield_now().await;
        // Advance just past the ceiling, then let the supervisor timeout fire and
        // teardown publish the terminal state (graces are on the mock clock too).
        tokio::time::advance(Duration::from_millis(MAX_EXEC_RUNTIME_MS + 1)).await;
        let mut snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        for _ in 0..200 {
            if !matches!(snap.state, ExecState::Running) {
                break;
            }
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_millis(1_000)).await;
            snap = rt.inspect(&owner, &exec_id, "boot-1").unwrap();
        }
        assert_eq!(snap.state, ExecState::Cancelled);
    }
}
