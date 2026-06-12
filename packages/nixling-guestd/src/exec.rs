//! Attached, non-interactive guest exec runtime.
//!
//! This module implements the create-and-start attached exec subset: non-TTY,
//! non-detached, stdin-closed commands only. It is guest-local process
//! execution inside the VM. There is no host broker op, no CLI surface, no
//! readiness wiring, and no user-session-daemon participation.
//!
//! Security posture: attached exec is trusted-control-plane guest-root
//! execution. It is gated behind an explicit `user = "root"` request plus the
//! host-owned per-VM `exec.enable` + `allowRoot` policy. It is not a sandbox
//! and makes no CPU/memory/fd kernel-isolation claim; it bounds the
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
/// dormant guest unit). Defaults are fail-closed: exec disabled, root denied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecPolicy {
    pub enabled: bool,
    pub allow_root: bool,
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
}

impl ExecEntry {
    fn snapshot(&self) -> ExecSnapshot {
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
    policy: ExecPolicy,
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

    // Root-only gate. Omitted users fail closed; non-root is unsupported.
    match input.user.as_deref() {
        Some("root") => {
            if !policy.allow_root {
                return Err(ExecError::RootDenied);
            }
        }
        Some(_) => return Err(ExecError::UserDenied),
        None => return Err(ExecError::RootDenied),
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
    // argv[0] must be an absolute path; no PATH-based lookup is performed.
    if !program.starts_with('/') {
        return Err(ExecError::InvalidArgv);
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
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
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
        }
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
        let command = validate_and_authorize(&input, self.policy)?;

        // Reserve a capacity slot under the execs lock so concurrent creates
        // cannot collectively exceed the caps. The guard releases the slot on
        // every early return; it is dropped once the placeholder occupies it.
        let reservation = {
            let execs = self.lock_execs();
            let reserved = self.reservations.load(Ordering::SeqCst) as usize;
            if execs.len() + reserved >= EXEC_SESSIONS_PER_VM {
                return Err(ExecError::ExecCapacityExceeded);
            }
            let running = execs
                .values()
                .filter(|entry| !entry.is_terminal())
                .count();
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
        let placeholder = new_exec_entry(owner.clone(), guest_boot_id.clone(), Arc::new(NoopKiller));
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
    pub fn close_connection(&self, owner: &ConnectionKey) {
        let owned: Vec<(String, Arc<ExecEntry>)> = {
            let execs = self.lock_execs();
            execs
                .iter()
                .filter(|(_, entry)| &entry.owner == owner)
                .map(|(id, entry)| (id.clone(), Arc::clone(entry)))
                .collect()
        };
        for (id, entry) in owned {
            entry.cancel();
            self.lock_execs().remove(&id);
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

/// Build a fresh running exec entry with empty output rings.
fn new_exec_entry(
    owner: ConnectionKey,
    guest_boot_id: String,
    killer: Arc<dyn ProcessKiller>,
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
) -> JoinHandle<()> {
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
                shared.outcome = if ceiling_exceeded { None } else { Some(outcome) };
                shared.stdout.mark_eof();
                shared.stderr.mark_eof();
                shared.bump();
            }
        }
        entry.notify.notify_waiters();
    })
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

    #[test]
    fn validate_rejects_unsupported_modes() {
        let policy = ExecPolicy {
            enabled: true,
            allow_root: true,
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
                validate_and_authorize(&input, policy),
                Err(ExecError::UnsupportedMode)
            );
        }
    }

    #[test]
    fn validate_enforces_root_only_policy() {
        let enabled_root = ExecPolicy {
            enabled: true,
            allow_root: true,
        };
        let enabled_no_root = ExecPolicy {
            enabled: true,
            allow_root: false,
        };
        let disabled = ExecPolicy::disabled();

        let mut omitted = good_input();
        omitted.user = None;
        assert_eq!(
            validate_and_authorize(&omitted, enabled_root),
            Err(ExecError::RootDenied)
        );

        let mut non_root = good_input();
        non_root.user = Some("alice".to_owned());
        assert_eq!(
            validate_and_authorize(&non_root, enabled_root),
            Err(ExecError::UserDenied)
        );

        assert_eq!(
            validate_and_authorize(&good_input(), enabled_no_root),
            Err(ExecError::RootDenied)
        );
        assert_eq!(
            validate_and_authorize(&good_input(), disabled),
            Err(ExecError::ExecDisabled)
        );
        assert!(validate_and_authorize(&good_input(), enabled_root).is_ok());
    }

    #[test]
    fn validate_rejects_bad_command_shapes() {
        let policy = ExecPolicy {
            enabled: true,
            allow_root: true,
        };

        let mut empty = good_input();
        empty.argv = vec![];
        assert_eq!(
            validate_and_authorize(&empty, policy),
            Err(ExecError::InvalidArgv)
        );

        let mut relative = good_input();
        relative.argv = vec!["echo".to_owned()];
        assert_eq!(
            validate_and_authorize(&relative, policy),
            Err(ExecError::InvalidArgv)
        );

        let mut nul = good_input();
        nul.argv = vec!["/bin/echo".to_owned(), "a\0b".to_owned()];
        assert_eq!(
            validate_and_authorize(&nul, policy),
            Err(ExecError::InvalidArgv)
        );

        let mut too_many = good_input();
        too_many.argv = std::iter::once("/bin/echo".to_owned())
            .chain((0..MAX_ARGV).map(|_| "x".to_owned()))
            .collect();
        assert_eq!(
            validate_and_authorize(&too_many, policy),
            Err(ExecError::InvalidArgv)
        );

        let mut rel_cwd = good_input();
        rel_cwd.cwd = Some("rel".to_owned());
        assert_eq!(
            validate_and_authorize(&rel_cwd, policy),
            Err(ExecError::CwdInvalid)
        );

        let mut empty_cwd = good_input();
        empty_cwd.cwd = Some(String::new());
        assert_eq!(
            validate_and_authorize(&empty_cwd, policy),
            Err(ExecError::CwdInvalid)
        );

        let mut bad_env = good_input();
        bad_env.env = vec![("1BAD".to_owned(), "v".to_owned())];
        assert_eq!(
            validate_and_authorize(&bad_env, policy),
            Err(ExecError::InvalidEnv)
        );

        let mut dup_env = good_input();
        dup_env.env = vec![
            ("A".to_owned(), "1".to_owned()),
            ("A".to_owned(), "2".to_owned()),
        ];
        assert_eq!(
            validate_and_authorize(&dup_env, policy),
            Err(ExecError::InvalidEnv)
        );

        let mut big_chunk = good_input();
        big_chunk.max_chunk_bytes = HARD_MAX_CHUNK_BYTES + 1;
        assert_eq!(
            validate_and_authorize(&big_chunk, policy),
            Err(ExecError::MaxChunkExceeded)
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

    fn runtime(policy: ExecPolicy) -> (ExecRuntime<FakeSpawner, SeqIds>, SpawnHooks) {
        let (spawner, hooks) = FakeSpawner::new();
        (ExecRuntime::new(spawner, SeqIds::new(), policy), hooks)
    }

    #[tokio::test]
    async fn create_streams_output_and_reports_exit() {
        let policy = ExecPolicy {
            enabled: true,
            allow_root: true,
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
                .wait(&owner, &exec_id, "boot-1", Some(snap.state_generation), 1000)
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
            allow_root: true,
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
            allow_root: true,
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
            allow_root: true,
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
                allow_root: true,
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
            .read_output(&owner, &exec_id, "boot-1", Stream::Stdout, 0, 1024, false, 0)
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
            .read_output(&owner, &exec_id, "boot-1", Stream::Stdout, 0, 1024, true, 50)
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
                allow_root: true,
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
}
