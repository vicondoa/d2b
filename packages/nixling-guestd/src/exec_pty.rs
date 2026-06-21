//! Interactive TTY exec: PTY plumbing for the connection-owned,
//! non-durable, attached interactive exec path.
//!
//! Security/safety posture: nixling guest binaries are built fully static and
//! with `unsafe_code = "forbid"`, so the PTY session setup that classically
//! requires a `pre_exec`/fork hook is NOT done in first-party code. Instead the
//! controlling-terminal handshake (`setsid` + `TIOCSCTTY` + `dup2` +
//! `TIOCSWINSZ`) runs inside the first-party static helper
//! (`nixling-exec-runner --tty-exec`), which performs the setup in ordinary
//! safe `rustix` code and then `exec`s the target. guestd opens the PTY master
//! and slave with `O_NOCTTY|O_CLOEXEC` and hands the slave to the helper via the
//! safe `Stdio::from(OwnedFd)` fd contract; guestd itself never acquires a
//! controlling terminal.
//!
//! This module owns the PTY *mechanism* behind fakeable traits plus the pure
//! per-session protocol state (stdin offset machine, control-seq dispatcher,
//! teardown phase). The low-level PTY allocation/control syscalls
//! (`openpt`/`grantpt`/`unlockpt`/`ptsname`/`TIOCSWINSZ`/`tcgetpgrp`) live here
//! and in the runner helper, never in the attached spawner
//! (`exec.rs`/`exec_linux.rs`) — see `tests/guest-exec-runtime-static.sh`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::exec::{ExecError, ProcessWaiter, ValidatedCommand};
pub use crate::terminal_io::{
    ControlSeqState, StdinLogic, StdinWriteOk, TerminalIoError, TerminalSize, TtyPhase, TtySignal,
    VEOF,
};

impl From<TerminalIoError> for ExecError {
    fn from(value: TerminalIoError) -> Self {
        match value {
            TerminalIoError::InvalidTerminalSize => Self::InvalidTerminalSize,
            TerminalIoError::StdinClosed => Self::StdinClosed,
            TerminalIoError::StdinOffsetMismatch => Self::StdinOffsetMismatch,
            TerminalIoError::ControlSeqMismatch => Self::ControlSeqMismatch,
        }
    }
}

/// Control surface for a live PTY master, fakeable for tests.
pub trait PtyControl: Send + Sync {
    /// Apply `TIOCSWINSZ` to the master; the kernel delivers `SIGWINCH` to the
    /// foreground process group. Idempotent.
    fn resize(&self, size: TerminalSize);

    /// Deliver `signal` to the master's *current* foreground process group,
    /// resolved via `tcgetpgrp(master)` at delivery time (job-control shells
    /// move the foreground PG). Best-effort / at-least-once.
    fn signal_foreground(&self, signal: TtySignal);
}

/// Best-effort containment for the helper-created TTY session, fakeable for
/// tests. The no-orphan guarantee covers only processes that remain in the
/// session (sid == session-leader pid); a `setsid()`/double-fork escapee is an
/// accepted trusted-root limitation (interactive exec is root-only/opt-in).
pub trait SessionReaper: Send + Sync {
    /// SIGKILL every process still in the TTY session. Idempotent; repeats
    /// internally (bounded) until the session is empty.
    fn kill_session(&self);
}

/// A spawned PTY-backed interactive exec. Distinct from the
/// [`crate::exec::SpawnedProcess`] (which only exposes stdout/stderr + killer +
/// waiter): the PTY master is a single bidirectional fd surfaced as an
/// independent merged-output [`AsyncRead`] half and a stdin-sink [`AsyncWrite`]
/// half, plus a control handle (resize / foreground-PG signal), a `waiter` that
/// reaps the direct child, and a `reaper` that SIGKILLs any process remaining in
/// the helper-created TTY session on teardown.
pub struct SpawnedPtyProcess {
    /// Merged stdout+stderr from the PTY master (raw bytes).
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    /// Stdin sink to the PTY master (WriteStdin / VEOF inject).
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    /// Resize + foreground-process-group signalling against the master.
    pub control: Arc<dyn PtyControl>,
    /// Reaps the direct child (helper → target), owned by the supervisor.
    pub waiter: Box<dyn ProcessWaiter>,
    /// SIGKILLs every process remaining in the TTY session on teardown.
    pub reaper: Arc<dyn SessionReaper>,
}

/// Bounded depth of the per-session writer task request queue. Kept small: the
/// service layer already sheds excess concurrent `WriteStdin` handlers per
/// connection, and the writer task processes one op at a time, so a deep queue
/// would only add latency to backpressure.
const WRITER_QUEUE_DEPTH: usize = 8;

/// A request submitted to a session's dedicated writer task. The writer task
/// owns the PTY master write half and the stdin offset machine, so admit →
/// write → advance is serialized without any handler holding a lock across the
/// (potentially blocking) PTY write.
enum WriteOp {
    Write {
        offset: u64,
        data: Vec<u8>,
        close_after: bool,
        ack: oneshot::Sender<Result<StdinWriteOk, ExecError>>,
    },
    Close {
        offset: u64,
        ack: oneshot::Sender<Result<(u64, bool), ExecError>>,
    },
}

/// Per-session interactive state, held by the owning [`crate::exec::ExecEntry`].
/// Mutable protocol state lives behind short, non-await std mutexes. The PTY
/// master write half is owned by a dedicated, abortable **writer task** (not a
/// handler-held lock): `WriteStdin`/`CloseStdin` submit a [`WriteOp`] over a
/// bounded channel and await the result. This is the deadlock fix — a child that
/// stops reading stdin can block the writer task on a full PTY, but teardown
/// drops the master write clone by **aborting the writer task**, never by
/// contending for a lock the blocked write holds.
pub struct TtyState {
    /// Bounded request channel to the writer task. A closed channel (the task
    /// was aborted by teardown) makes new submissions fail with `StdinClosed`.
    writer_tx: mpsc::Sender<WriteOp>,
    /// Join handle for the writer task, taken + aborted + awaited by teardown
    /// so the master write clone is dropped BEFORE the SIGHUP→grace→KILL window.
    writer_task: Mutex<Option<JoinHandle<()>>>,
    /// Master control surface (resize / foreground signal). `None` after
    /// teardown.
    control: Mutex<Option<Arc<dyn PtyControl>>>,
    /// Session reaper, retained for teardown (and for the entry's killer).
    reaper: Arc<dyn SessionReaper>,
    /// Pure stdin offset machine. Mutated only by the writer task (serialized);
    /// read by inspect.
    stdin: Arc<Mutex<StdinLogic>>,
    /// Pure control-seq dispatcher (resize + signal share it).
    seq: Mutex<ControlSeqState>,
    /// Teardown phase gate, shared with the writer task so a write submitted
    /// just before `Closing` is refused deterministically (no write after
    /// `Closing`).
    phase: Arc<Mutex<TtyPhase>>,
}

impl TtyState {
    pub fn new(
        writer: Box<dyn AsyncWrite + Send + Unpin>,
        control: Arc<dyn PtyControl>,
        reaper: Arc<dyn SessionReaper>,
    ) -> Self {
        let stdin = Arc::new(Mutex::new(StdinLogic::new()));
        let phase = Arc::new(Mutex::new(TtyPhase::Running));
        let (writer_tx, writer_rx) = mpsc::channel(WRITER_QUEUE_DEPTH);
        // Spawn the writer task that owns the master write half. Called from the
        // async create path, so a runtime is always present.
        let writer_task = tokio::spawn(writer_loop(
            writer,
            writer_rx,
            Arc::clone(&stdin),
            Arc::clone(&phase),
        ));
        Self {
            writer_tx,
            writer_task: Mutex::new(Some(writer_task)),
            control: Mutex::new(Some(control)),
            reaper,
            stdin,
            seq: Mutex::new(ControlSeqState::new()),
            phase,
        }
    }

    fn lock_phase(&self) -> std::sync::MutexGuard<'_, TtyPhase> {
        self.phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_stdin(&self) -> std::sync::MutexGuard<'_, StdinLogic> {
        self.stdin
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_seq(&self) -> std::sync::MutexGuard<'_, ControlSeqState> {
        self.seq
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_control(&self) -> std::sync::MutexGuard<'_, Option<Arc<dyn PtyControl>>> {
        self.control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Snapshot of the current teardown phase.
    pub fn phase(&self) -> TtyPhase {
        *self.lock_phase()
    }

    /// True once the session has left `Running` (no further stdin/control side
    /// effects are accepted).
    pub fn is_closing(&self) -> bool {
        !matches!(*self.lock_phase(), TtyPhase::Running)
    }

    /// Atomically transition `Running → Closing`. Returns true exactly once
    /// (the caller that wins the race owns teardown); false if already closing
    /// or terminal.
    pub fn begin_closing(&self) -> bool {
        let mut phase = self.lock_phase();
        if matches!(*phase, TtyPhase::Running) {
            *phase = TtyPhase::Closing;
            true
        } else {
            false
        }
    }

    /// Mark the session terminal (after the session has been reaped).
    pub fn mark_terminal(&self) {
        *self.lock_phase() = TtyPhase::Terminal;
    }

    /// The session reaper handle (also used by the entry's killer).
    pub fn reaper(&self) -> Arc<dyn SessionReaper> {
        Arc::clone(&self.reaper)
    }

    /// Current next-expected stdin offset (for inspect / responses).
    pub fn stdin_next_offset(&self) -> u64 {
        self.lock_stdin().next_offset()
    }

    /// True once stdin has been closed (VEOF injected).
    pub fn stdin_closed(&self) -> bool {
        self.lock_stdin().is_closed()
    }

    /// The current control sequence (highest admitted resize/signal seq) for the
    /// inspect snapshot.
    pub fn last_control_seq(&self) -> u64 {
        self.lock_seq().last_seq()
    }

    /// Take the control surface, dropping the master clone it holds. Idempotent.
    pub fn take_control(&self) -> Option<Arc<dyn PtyControl>> {
        self.lock_control().take()
    }

    /// Admit + dispatch a resize. Rejects when closing (no side effect), on a
    /// stale/dup seq, or on an invalid geometry. The control surface resolves
    /// `SIGWINCH` to the foreground PG at delivery.
    pub fn resize(&self, seq: u64, rows: u32, cols: u32) -> Result<(), ExecError> {
        // Hold the phase lock across the phase-check AND the side-effect
        // dispatch so admission is atomic w.r.t. `begin_closing`: either this
        // wins the lock, observes `Running`, and applies the resize before
        // `begin_closing` can transition (and only then `take_control`), or
        // `begin_closing` wins, sets `Closing`, and this returns `ExecClosing`
        // with no side effect. `begin_closing` and `take_control` are ordered
        // (take_control runs only after begin_closing wins), so no `SIGWINCH`
        // can be delivered once `Closing` is set.
        let phase = self.lock_phase();
        if !matches!(*phase, TtyPhase::Running) {
            return Err(ExecError::ExecClosing);
        }
        self.lock_seq().admit(seq)?;
        let size = TerminalSize::checked(rows, cols)?;
        if let Some(control) = self.lock_control().as_ref() {
            control.resize(size);
        }
        Ok(())
    }

    /// Admit + dispatch a foreground-PG signal. TTY-only; the target must be the
    /// foreground process group (validated by the caller). Rejects when closing,
    /// on a stale/dup seq, or on a signal outside the allowlist.
    pub fn signal(&self, seq: u64, signal: u32) -> Result<(), ExecError> {
        // Hold the phase lock across the phase-check AND the side-effect
        // dispatch (see `resize`): once `Closing` is set this returns
        // `ExecClosing` with no signal delivered, and a signal that wins the
        // lock completes before `begin_closing` can transition + `take_control`.
        let phase = self.lock_phase();
        if !matches!(*phase, TtyPhase::Running) {
            return Err(ExecError::ExecClosing);
        }
        // Validate the signal against the allowlist BEFORE consuming the control
        // sequence: an out-of-allowlist signal must not advance the dispatcher
        // (nothing is delivered, so the seq stays available for a valid retry).
        let signal = TtySignal::from_raw(signal).ok_or(ExecError::InvalidSignal)?;
        self.lock_seq().admit(seq)?;
        if let Some(control) = self.lock_control().as_ref() {
            control.signal_foreground(signal);
        }
        Ok(())
    }

    /// Write `data` to the master at `offset`, optionally injecting `VEOF`
    /// afterwards (`close_after`). The whole accept → write → advance runs in the
    /// session's writer task (serialized there), so this method never holds a
    /// lock across the blocking PTY write: it submits a [`WriteOp`] and awaits
    /// the result. A closed channel or a dropped ack (teardown aborted the task)
    /// surfaces as `StdinClosed`, so no write is reported as applied after
    /// teardown.
    pub async fn write_stdin(
        &self,
        offset: u64,
        data: &[u8],
        close_after: bool,
    ) -> Result<StdinWriteOk, ExecError> {
        if self.is_closing() {
            return Err(ExecError::StdinClosed);
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        let op = WriteOp::Write {
            offset,
            data: data.to_vec(),
            close_after,
            ack: ack_tx,
        };
        if self.writer_tx.send(op).await.is_err() {
            // Writer task gone (teardown aborted it): nothing was written.
            return Err(ExecError::StdinClosed);
        }
        match ack_rx.await {
            Ok(result) => result,
            // The task was aborted mid-op (teardown): the write did not complete.
            Err(_) => Err(ExecError::StdinClosed),
        }
    }

    /// Inject `VEOF` and mark stdin closed, keeping the master open. Idempotent:
    /// a second close is a no-op duplicate. `offset` must match the current
    /// next-offset. Routed through the same writer task as `WriteStdin` so the
    /// VEOF is ordered after any pending writes. Returns `(final_offset,
    /// duplicate)`.
    pub async fn close_stdin(&self, offset: u64) -> Result<(u64, bool), ExecError> {
        if self.is_closing() {
            return Err(ExecError::StdinClosed);
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        let op = WriteOp::Close {
            offset,
            ack: ack_tx,
        };
        if self.writer_tx.send(op).await.is_err() {
            return Err(ExecError::StdinClosed);
        }
        match ack_rx.await {
            Ok(result) => result,
            Err(_) => Err(ExecError::StdinClosed),
        }
    }

    /// Abort + await the writer task, dropping the master write clone it owns.
    /// Used by teardown to release one of the master references WITHOUT waiting
    /// on any lock the (possibly blocked) write holds; awaiting the handle
    /// guarantees the clone is gone before the SIGHUP→grace→KILL window.
    pub async fn release_writer(&self) {
        let handle = self
            .writer_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }
    }
}

/// The per-session writer task. Owns the PTY master write half and the stdin
/// offset machine; processes one [`WriteOp`] at a time so admit → write →
/// advance is serialized without a handler-held lock. Refuses any op once the
/// session has left `Running` (so a write queued just before `Closing` cannot
/// apply afterwards), and is dropped — abandoning any in-flight write and its
/// master clone — when teardown aborts it.
async fn writer_loop(
    mut sink: Box<dyn AsyncWrite + Send + Unpin>,
    mut rx: mpsc::Receiver<WriteOp>,
    stdin: Arc<Mutex<StdinLogic>>,
    phase: Arc<Mutex<TtyPhase>>,
) {
    while let Some(op) = rx.recv().await {
        match op {
            WriteOp::Write {
                offset,
                data,
                close_after,
                ack,
            } => {
                let result =
                    process_write(&mut sink, &stdin, &phase, offset, &data, close_after).await;
                let _ = ack.send(result);
            }
            WriteOp::Close { offset, ack } => {
                let result = process_close(&mut sink, &stdin, &phase, offset).await;
                let _ = ack.send(result);
            }
        }
    }
}

/// Lock the stdin machine briefly (the writer task is the only mutator, so this
/// never contends with another writer).
fn lock_stdin_logic(stdin: &Arc<Mutex<StdinLogic>>) -> std::sync::MutexGuard<'_, StdinLogic> {
    stdin
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// True iff the session phase is still `Running`. The admission/commit gate for
/// the writer task; read under the SAME phase lock `begin_closing` writes so a
/// write is admitted and committed only while the session has not entered
/// `Closing`.
fn phase_is_running(phase: &Arc<Mutex<TtyPhase>>) -> bool {
    matches!(
        *phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        TtyPhase::Running
    )
}

/// Validate the offset, write what can be written, and advance the cursor by
/// exactly the bytes that landed (partial-write accounting). Admission and
/// the offset-machine commit are both gated on the session still being
/// `Running`, so the committed side effect (offset advance + success ack) is
/// atomic w.r.t. `begin_closing`: a write that loses the race to `Closing`
/// surfaces `StdinClosed` and does NOT advance the offset (H1).
async fn process_write(
    sink: &mut Box<dyn AsyncWrite + Send + Unpin>,
    stdin: &Arc<Mutex<StdinLogic>>,
    phase: &Arc<Mutex<TtyPhase>>,
    offset: u64,
    data: &[u8],
    close_after: bool,
) -> Result<StdinWriteOk, ExecError> {
    // Admission: refuse before touching the master once the session has left
    // `Running`, then validate the offset. `begin_closing` never mutates the
    // stdin machine, so the phase check + admit need not share one lock.
    if !phase_is_running(phase) {
        return Err(ExecError::StdinClosed);
    }
    lock_stdin_logic(stdin).admit(offset)?;
    let (written, write_res) = write_counting(sink, data).await;
    let full = written == data.len();
    // Inject VEOF + mark closed only once the full payload has landed.
    let mut closed = false;
    if full && close_after {
        let (veof_written, _veof_res) = write_counting(sink, &[VEOF]).await;
        closed = veof_written == 1;
    }
    // A write that made zero progress on a non-empty payload is a hard failure
    // (the master is gone); a partial or full write is accepted for the bytes
    // that landed, and the client retries any remainder from `next_offset`.
    if written == 0 && !data.is_empty() {
        return Err(write_res.err().unwrap_or(ExecError::StdinClosed));
    }
    // Commit: advance the offset machine + report success ONLY while still
    // `Running`, holding the phase lock across the check + advance so it is
    // atomic w.r.t. `begin_closing`. If `Closing` raced in during the (awaited)
    // write, the offset is left untouched and the op surfaces `StdinClosed` —
    // no offset advance / success ack after `Closing`.
    let next_offset = {
        let phase_guard = phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(*phase_guard, TtyPhase::Running) {
            return Err(ExecError::StdinClosed);
        }
        let mut logic = lock_stdin_logic(stdin);
        logic.advance(written as u64);
        if closed {
            logic.close();
        }
        logic.next_offset()
    };
    Ok(StdinWriteOk {
        accepted_len: written as u64,
        next_offset,
        closed,
    })
}

/// Inject VEOF and mark closed (idempotent). `offset` must match the current
/// next-offset. The cursor is NOT advanced (VEOF is a control byte, not part of
/// the stdin byte stream). Admission and the close commit are gated on the
/// session still being `Running` (atomic w.r.t. `begin_closing` — H1).
async fn process_close(
    sink: &mut Box<dyn AsyncWrite + Send + Unpin>,
    stdin: &Arc<Mutex<StdinLogic>>,
    phase: &Arc<Mutex<TtyPhase>>,
    offset: u64,
) -> Result<(u64, bool), ExecError> {
    {
        if !phase_is_running(phase) {
            return Err(ExecError::StdinClosed);
        }
        let logic = lock_stdin_logic(stdin);
        if logic.is_closed() {
            return Ok((logic.next_offset(), true));
        }
        if offset != logic.next_offset() {
            return Err(ExecError::StdinOffsetMismatch);
        }
    }
    let (veof_written, _res) = write_counting(sink, &[VEOF]).await;
    if veof_written != 1 {
        return Err(ExecError::StdinClosed);
    }
    // Commit: mark closed only while still `Running` (atomic w.r.t.
    // `begin_closing` via the phase lock).
    let next_offset = {
        let phase_guard = phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(*phase_guard, TtyPhase::Running) {
            return Err(ExecError::StdinClosed);
        }
        let mut logic = lock_stdin_logic(stdin);
        logic.close();
        logic.next_offset()
    };
    Ok((next_offset, false))
}

/// Write as many bytes of `data` as reach the sink, returning the count that
/// actually landed and the terminal result. A mid-stream I/O error (or a
/// zero-length write, signalling a closed master) stops the loop with the bytes
/// written so far, so the caller can advance the offset machine by exactly that
/// count and a client retry never re-delivers already-written bytes.
async fn write_counting(
    sink: &mut Box<dyn AsyncWrite + Send + Unpin>,
    data: &[u8],
) -> (usize, Result<(), ExecError>) {
    use tokio::io::AsyncWriteExt;
    let mut written = 0;
    while written < data.len() {
        match sink.write(&data[written..]).await {
            Ok(0) => return (written, Err(ExecError::StdinClosed)),
            Ok(n) => written += n,
            Err(_) => return (written, Err(ExecError::StdinClosed)),
        }
    }
    (written, Ok(()))
}

/// Allocates a PTY pair and spawns the first-party TTY helper as the session
/// leader with the slave as its controlling terminal, returning the connected
/// [`SpawnedPtyProcess`]. A fake duplex implementation backs the deterministic
/// tests; the production implementation is [`linux::LinuxPtyProcessSpawner`].
#[async_trait]
pub trait PtyProcessSpawner: Send + Sync + 'static {
    async fn spawn(
        &self,
        command: ValidatedCommand,
        initial_size: TerminalSize,
    ) -> Result<SpawnedPtyProcess, ExecError>;
}

/// Default spawner used until a real PTY spawner is wired in: always reports the
/// interactive mode unsupported. The `exec_tty` capability is not advertised
/// when this is in effect, so a TTY create fails closed with a typed error.
#[derive(Default)]
pub struct NullPtySpawner;

#[async_trait]
impl PtyProcessSpawner for NullPtySpawner {
    async fn spawn(
        &self,
        _command: ValidatedCommand,
        _initial_size: TerminalSize,
    ) -> Result<SpawnedPtyProcess, ExecError> {
        Err(ExecError::UnsupportedMode)
    }
}

/// Production Linux PTY spawner: allocates the master via `posix_openpt`, hands
/// the slave to the first-party `--tty-exec` helper over the safe
/// `Stdio::from(OwnedFd)` contract, and adopts the master as async read/write
/// halves plus a control surface and a `/proc`-scanning session reaper.
pub mod linux {
    use std::os::fd::OwnedFd;
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::process::Stdio;
    use std::sync::Arc;
    use std::task::{Context, Poll, ready};
    use std::time::Duration;

    use async_trait::async_trait;
    use rustix::fs::{Mode, OFlags};
    use rustix::io::{ioctl_fionbio, read, write};
    use rustix::process::{Pid, Signal, kill_process, kill_process_group};
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    use rustix::termios::{Winsize, tcgetpgrp, tcsetwinsize};
    use tokio::io::unix::AsyncFd;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::process::{Child, Command};

    use super::{
        PtyControl, PtyProcessSpawner, SessionReaper, SpawnedPtyProcess, TerminalSize, TtySignal,
    };
    use crate::exec::{ExecError, ExitOutcome, ProcessWaiter, ValidatedCommand};
    use crate::login_session::systemctl_kill_unit;

    /// Bounded SIGKILL sweep rounds for the session reaper.
    const REAP_MAX_ROUNDS: u32 = 50;
    /// Sleep between reaper sweep rounds.
    const REAP_ROUND_SLEEP_MS: u64 = 5;
    /// `EIO` raw value (Linux): a PTY master read returns it once the last slave
    /// closes. Treated as a clean EOF. Pinned to avoid a libc dependency.
    const EIO: i32 = 5;

    /// Production PTY spawner. Constructed with the absolute path to the
    /// `nixling-exec-runner` binary (invoked in `--tty-exec` mode), the
    /// `systemd-run` binary, the workload user's login shell, and the
    /// host-fixed workload user. The helper sets up the controlling TTY and
    /// then execs `systemd-run --pty --property=PAMName=login --uid=<user>`,
    /// so the interactive session is a real PAM login for the workload user
    /// (never root): `pam_systemd` provisions `XDG_RUNTIME_DIR`, the login
    /// shell sources the profile (`WAYLAND_DISPLAY`, …), and the requested
    /// command runs inside that session — reproducing an interactive login
    /// (the surface `vm exec -it` drives).
    pub struct LinuxPtyProcessSpawner {
        helper_path: PathBuf,
        systemd_run_path: PathBuf,
        login_shell_path: PathBuf,
        exec_user: String,
    }

    impl LinuxPtyProcessSpawner {
        pub fn new(
            helper_path: PathBuf,
            systemd_run_path: PathBuf,
            login_shell_path: PathBuf,
            exec_user: String,
        ) -> Self {
            Self {
                helper_path,
                systemd_run_path,
                login_shell_path,
                exec_user,
            }
        }
    }

    #[async_trait]
    impl PtyProcessSpawner for LinuxPtyProcessSpawner {
        async fn spawn(
            &self,
            command: ValidatedCommand,
            initial_size: TerminalSize,
        ) -> Result<SpawnedPtyProcess, ExecError> {
            // Allocate the master with O_NOCTTY|O_CLOEXEC: guestd never acquires
            // a controlling terminal, and the master never leaks across exec.
            let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)
                .map_err(|_| ExecError::SpawnFailed)?;
            grantpt(&master).map_err(|_| ExecError::SpawnFailed)?;
            unlockpt(&master).map_err(|_| ExecError::SpawnFailed)?;
            let slave_path = ptsname(&master, Vec::new()).map_err(|_| ExecError::SpawnFailed)?;
            // Open the slave O_NOCTTY|O_CLOEXEC: NOCTTY so this open does not make
            // guestd acquire a controlling terminal (the helper's TIOCSCTTY does
            // that in the child session), and CLOEXEC so a concurrent fork/exec
            // elsewhere in guestd cannot inherit the slave and keep the PTY alive
            // (breaking HUP/EOF). `Stdio::from(slave)` still hands fd0 to the
            // helper correctly: the Command machinery dup2s the slave onto the
            // child's fd0, and dup2 clears CLOEXEC on the duplicate.
            let slave = rustix::fs::open(
                &slave_path,
                OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ExecError::SpawnFailed)?;
            // CLOEXEC status pipe: the write end is handed to the helper as
            // stdout and closes on a successful exec (guestd reads EOF).
            let (status_r, status_w) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
                .map_err(|_| ExecError::SpawnFailed)?;

            let mut cmd = Command::new(&self.helper_path);
            // Interactive sessions run the requested command as the host-fixed
            // workload user (never root) inside a real PAM login session: the
            // helper sets up the controlling TTY, then execs
            // `systemd-run --pty --property=PAMName=login --uid=<user> -- <login
            // shell> -l -c 'exec "$@"' <argv...>`. pam_systemd provisions
            // XDG_RUNTIME_DIR and the login shell sources the profile
            // (WAYLAND_DISPLAY, …), so graphical clients work; the client argv
            // is passed as shell positional params (no injection). TERM is
            // forwarded as a session override; the rest of the environment is
            // established by login/PAM/profile.
            let term = command
                .env
                .iter()
                .find(|(k, _)| k == "TERM")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| "xterm".to_owned());
            // Name the transient unit so teardown can SIGKILL the workload's
            // cgroup directly: the workload runs in a PID 1-owned `systemd-run
            // --pty` unit, NOT in the helper's TTY session, so the `/proc`
            // session sweep alone would not reach it.
            let unit_name = crate::login_session::unique_exec_unit_name();
            let systemctl_path =
                crate::login_session::sibling_systemctl_path(&self.systemd_run_path);
            let session_args = crate::login_session::login_session_systemd_run_args(
                &self.login_shell_path,
                &self.exec_user,
                &unit_name,
                crate::login_session::SessionMode::Pty,
                &command,
            );
            cmd.arg("--tty-exec")
                .arg("--rows")
                .arg(initial_size.rows.to_string())
                .arg("--cols")
                .arg(initial_size.cols.to_string())
                .arg("--")
                .arg(&self.systemd_run_path)
                .args(&session_args)
                .current_dir("/")
                .env_clear()
                .env("TERM", term)
                // Safe fd handoff: no arbitrary pass_fds, no process_group(0),
                // no pre_exec. The slave is stdin; the status pipe is stdout.
                .stdin(Stdio::from(slave))
                .stdout(Stdio::from(status_w))
                .stderr(Stdio::null())
                .kill_on_drop(false);

            let child = cmd.spawn().map_err(|_| ExecError::SpawnFailed)?;
            // Drop the parent `Command` IMMEDIATELY after a successful spawn, and
            // BEFORE awaiting the status handshake. `Command::spawn(&mut self)`
            // leaves `cmd` owning the parent's copies of the slave (the child's
            // fd 0) and the status-pipe write end (the child's fd 1) until the
            // Command is dropped. The status pipe only reaches EOF — the
            // success signal `read_status_byte` awaits — once EVERY write end is
            // closed, so the parent MUST release its `status_w` copy here or
            // `ExecCreate` would hang forever on a successful exec (the child
            // closes its CLOEXEC status copy on `execve`, but the parent's copy
            // would keep the pipe open). Dropping `cmd` also releases the
            // parent's slave copy, leaving the child as the sole holder of each
            // fd (the HUP/EOF fd-hygiene contract).
            drop(cmd);
            let pid = child.id().ok_or(ExecError::SpawnFailed)? as i32;
            // Arm a no-orphan drop guard immediately. `kill_on_drop(false)` is
            // required for the success path (the supervisor owns reaping via the
            // waiter), so until ownership of the child transfers into the
            // `SpawnedPtyProcess` below, ANY early return or async cancellation
            // (a failed nonblocking/`AsyncFd` setup, a setup-failure handshake,
            // a dropped future) must kill + reap the helper, or an interactive
            // root session would be orphaned.
            let guard = SpawnGuard {
                child: Some(child),
                sid: pid,
                systemctl_path: systemctl_path.clone(),
                unit_name: unit_name.clone(),
            };

            // Await the helper handshake: EOF == exec succeeded; one byte == a
            // typed setup/exec failure. The guard kills + reaps on every error
            // path here (including future cancellation).
            ioctl_fionbio(&status_r, true).map_err(|_| ExecError::SpawnFailed)?;
            let status_async = AsyncFd::new(status_r).map_err(|_| ExecError::SpawnFailed)?;
            let handshake = read_status_byte(&status_async).await;
            drop(status_async);
            match handshake {
                Ok(None) => {}
                Ok(Some(_)) | Err(_) => {
                    // The guard SIGKILLs + reaps the helper on drop.
                    return Err(ExecError::SpawnFailed);
                }
            }

            ioctl_fionbio(&master, true).map_err(|_| ExecError::SpawnFailed)?;
            let io = Arc::new(AsyncFd::new(master).map_err(|_| ExecError::SpawnFailed)?);
            // Ownership transfers to the supervisor now: disarm the guard and
            // move the child into the waiter.
            let child = guard.disarm();
            Ok(SpawnedPtyProcess {
                reader: Box::new(PtyReadHalf {
                    io: Arc::clone(&io),
                }),
                writer: Box::new(PtyWriteHalf {
                    io: Arc::clone(&io),
                }),
                control: Arc::new(ProcPtyControl {
                    io: Arc::clone(&io),
                }),
                waiter: Box::new(PtyChildWaiter { child: Some(child) }),
                // The helper called setsid() before exec, so its pid is the
                // session id of the whole interactive session.
                reaper: Arc::new(ProcSessionReaper {
                    sid: pid,
                    systemctl_path,
                    unit_name,
                }),
            })
        }
    }

    /// Drop guard that kills + reaps the spawned helper (and any process that
    /// has joined the helper's TTY session, if `setsid` already ran) on every
    /// error / cancellation path until the child's ownership transfers into the
    /// [`SpawnedPtyProcess`]. Without it, a post-spawn failure under
    /// `kill_on_drop(false)` would leave an unmanaged interactive root session.
    struct SpawnGuard {
        child: Option<Child>,
        sid: i32,
        systemctl_path: PathBuf,
        unit_name: String,
    }

    impl SpawnGuard {
        /// Transfer ownership of the child out of the guard, disarming it.
        fn disarm(mut self) -> Child {
            self.child.take().expect("spawn guard is armed")
        }
    }

    impl Drop for SpawnGuard {
        fn drop(&mut self) {
            let Some(mut child) = self.child.take() else {
                return;
            };
            // SIGKILL the direct child by pid (NOT its process group: before the
            // helper's setsid runs it still shares guestd's process group, so a
            // group kill could hit guestd itself).
            let _ = child.start_kill();
            let sid = self.sid;
            let systemctl_path = self.systemctl_path.clone();
            let unit_name = self.unit_name.clone();
            // Reap the direct child and sweep any process remaining in the
            // helper-created TTY session (a no-op if setsid never ran, since no
            // process then has session id == pid), then SIGKILL the workload's
            // named transient unit cgroup. Spawned only when a runtime is
            // available; the synchronous SIGKILL above already fired.
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = child.wait().await;
                    ProcSessionReaper {
                        sid,
                        systemctl_path,
                        unit_name,
                    }
                    .kill_session();
                });
            }
        }
    }

    /// Read up to one status byte from the helper handshake pipe. `Ok(None)` on
    /// EOF (exec succeeded), `Ok(Some(byte))` on a typed failure.
    async fn read_status_byte(io: &AsyncFd<OwnedFd>) -> std::io::Result<Option<u8>> {
        let mut buf = [0_u8; 1];
        loop {
            let mut guard = io.readable().await?;
            match guard
                .try_io(|inner| read(inner.get_ref(), &mut buf).map_err(std::io::Error::from))
            {
                Ok(Ok(0)) => return Ok(None),
                Ok(Ok(_)) => return Ok(Some(buf[0])),
                Ok(Err(error)) => return Err(error),
                Err(_would_block) => continue,
            }
        }
    }

    /// Async read half of the PTY master (merged stdout+stderr).
    struct PtyReadHalf {
        io: Arc<AsyncFd<OwnedFd>>,
    }

    impl AsyncRead for PtyReadHalf {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();
            loop {
                let mut guard = ready!(this.io.poll_read_ready(cx))?;
                let unfilled = buf.initialize_unfilled();
                match guard
                    .try_io(|inner| read(inner.get_ref(), unfilled).map_err(std::io::Error::from))
                {
                    Ok(Ok(n)) => {
                        buf.advance(n);
                        return Poll::Ready(Ok(()));
                    }
                    // A PTY master read returns EIO once the last slave closes;
                    // surface it as a clean EOF so the reader loop stops.
                    Ok(Err(error)) if error.raw_os_error() == Some(EIO) => {
                        return Poll::Ready(Ok(()));
                    }
                    Ok(Err(error)) => return Poll::Ready(Err(error)),
                    Err(_would_block) => continue,
                }
            }
        }
    }

    /// Async write half of the PTY master (stdin sink).
    struct PtyWriteHalf {
        io: Arc<AsyncFd<OwnedFd>>,
    }

    impl AsyncWrite for PtyWriteHalf {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let this = self.get_mut();
            loop {
                let mut guard = ready!(this.io.poll_write_ready(cx))?;
                match guard
                    .try_io(|inner| write(inner.get_ref(), buf).map_err(std::io::Error::from))
                {
                    Ok(result) => return Poll::Ready(result),
                    Err(_would_block) => continue,
                }
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// Master-backed control surface.
    struct ProcPtyControl {
        io: Arc<AsyncFd<OwnedFd>>,
    }

    impl PtyControl for ProcPtyControl {
        fn resize(&self, size: TerminalSize) {
            let winsize = Winsize {
                ws_row: size.rows,
                ws_col: size.cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let _ = tcsetwinsize(self.io.get_ref(), winsize);
        }

        fn signal_foreground(&self, signal: TtySignal) {
            // Resolve the foreground process group at delivery time: a
            // job-control shell moves the foreground PG as it runs children.
            if let Ok(pgid) = tcgetpgrp(self.io.get_ref())
                && let Some(sig) = rustix_signal(signal)
            {
                let _ = kill_process_group(pgid, sig);
            }
        }
    }

    /// Map the allowlisted [`TtySignal`] to a rustix [`Signal`].
    fn rustix_signal(signal: TtySignal) -> Option<Signal> {
        Signal::from_raw(signal.raw())
    }

    /// Owns the direct child (helper → target) and reaps it.
    struct PtyChildWaiter {
        child: Option<Child>,
    }

    #[async_trait]
    impl ProcessWaiter for PtyChildWaiter {
        async fn wait(&mut self) -> ExitOutcome {
            match self.child.as_mut() {
                Some(child) => match child.wait().await {
                    Ok(status) => {
                        if let Some(code) = status.code() {
                            ExitOutcome::Exited(code)
                        } else if let Some(signal) = status.signal() {
                            ExitOutcome::Signaled(signal as u32)
                        } else {
                            ExitOutcome::Exited(-1)
                        }
                    }
                    Err(_) => ExitOutcome::Exited(-1),
                },
                None => ExitOutcome::Exited(-1),
            }
        }
    }

    /// `/proc`-scanning session reaper: SIGKILLs every process whose session id
    /// equals the helper-created session leader, then SIGKILLs the workload's
    /// named transient unit cgroup (the workload runs in a PID 1-owned
    /// `systemd-run --pty` unit, NOT in the helper's session, so the `/proc`
    /// sweep alone never reaches it), repeating the session sweep until empty
    /// (bounded).
    struct ProcSessionReaper {
        sid: i32,
        /// Path to `systemctl`, used to SIGKILL the named transient unit's cgroup.
        systemctl_path: PathBuf,
        /// The `--unit=` name of the workload's transient unit.
        unit_name: String,
    }

    impl SessionReaper for ProcSessionReaper {
        fn kill_session(&self) {
            // 1. LOCAL first: one SIGKILL pass over the helper's TTY session (the
            //    helper + the `systemd-run --pty` wrapper share it) so no further
            //    StartTransientUnit can be issued and teardown never depends
            //    solely on systemd.
            for pid in pids_in_session(self.sid) {
                if let Some(pid) = Pid::from_raw(pid) {
                    let _ = kill_process(pid, Signal::Kill);
                }
            }
            // 2. SYSTEMD: SIGKILL the whole transient-unit cgroup by name. This is
            //    the only step that reaches the actual workload (a quiet non-TTY
            //    child such as `sleep 3600` survives owner-disconnect otherwise).
            //    Always runs, even if the helper session is already empty — never
            //    gate it behind the empty-session early-out below. Bounded +
            //    idempotent (see `systemctl_kill_unit`).
            systemctl_kill_unit(&self.systemctl_path, &self.unit_name);
            // 3. Finish: bounded sweep until the helper session is empty, reaping
            //    any late-joining session members. The unit-cgroup kill above
            //    already handled the workload.
            for _ in 0..REAP_MAX_ROUNDS {
                let pids = pids_in_session(self.sid);
                if pids.is_empty() {
                    return;
                }
                for pid in pids {
                    if let Some(pid) = Pid::from_raw(pid) {
                        let _ = kill_process(pid, Signal::Kill);
                    }
                }
                std::thread::sleep(Duration::from_millis(REAP_ROUND_SLEEP_MS));
            }
        }
    }

    /// Enumerate every pid in `/proc` whose session id is `sid`.
    fn pids_in_session(sid: i32) -> Vec<i32> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return out;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Ok(pid) = name.parse::<i32>() else {
                continue;
            };
            if session_of(pid) == Some(sid) {
                out.push(pid);
            }
        }
        out
    }

    /// Parse the session id (field 6) from `/proc/<pid>/stat`. The `comm` field
    /// may contain spaces/parens, so fields are read after the final `)`.
    fn session_of(pid: i32) -> Option<i32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rparen = stat.rfind(')')?;
        let rest = &stat[rparen + 1..];
        // After ')': state(0) ppid(1) pgrp(2) session(3) ...
        rest.split_whitespace().nth(3).and_then(|s| s.parse().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- TtyState writer-task coverage -------------------------

    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;
    use tokio::io::AsyncWrite;

    /// Programmable fake PTY master write half. Accepts at most `chunk` bytes per
    /// `poll_write` (0 = unlimited), can be `paused` to model a full PTY (returns
    /// `Pending` and parks the waker), and can be given a finite `budget` after
    /// which writes error with zero progress (models the master going away
    /// mid-write). Records everything written for offset-accounting assertions.
    #[derive(Default)]
    struct FakeSinkState {
        written: Vec<u8>,
        chunk: usize,
        paused: bool,
        waker: Option<Waker>,
        budget: Option<usize>,
    }

    #[derive(Clone)]
    struct FakeSink {
        state: Arc<Mutex<FakeSinkState>>,
    }

    impl FakeSink {
        fn new(chunk: usize, budget: Option<usize>) -> (Self, Arc<Mutex<FakeSinkState>>) {
            let state = Arc::new(Mutex::new(FakeSinkState {
                chunk,
                budget,
                ..Default::default()
            }));
            (
                Self {
                    state: Arc::clone(&state),
                },
                state,
            )
        }
    }

    fn lock_sink(state: &Arc<Mutex<FakeSinkState>>) -> std::sync::MutexGuard<'_, FakeSinkState> {
        state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Unpause a paused [`FakeSink`] and wake any parked writer so the stuck
    /// `poll_write` re-polls and completes.
    fn resume_sink(state: &Arc<Mutex<FakeSinkState>>) {
        let waker = {
            let mut st = lock_sink(state);
            st.paused = false;
            st.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    impl AsyncWrite for FakeSink {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            data: &[u8],
        ) -> Poll<io::Result<usize>> {
            let mut st = lock_sink(&self.state);
            if st.paused {
                st.waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
            let want = data.len();
            let mut allow = if st.chunk == 0 {
                want
            } else {
                st.chunk.min(want)
            };
            if let Some(budget) = st.budget {
                allow = allow.min(budget);
            }
            if allow == 0 && want != 0 {
                // Budget exhausted (or zero-budget): the master is gone.
                return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
            }
            st.written.extend_from_slice(&data[..allow]);
            if let Some(budget) = st.budget.as_mut() {
                *budget -= allow;
            }
            Poll::Ready(Ok(allow))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct NoopControl;
    impl PtyControl for NoopControl {
        fn resize(&self, _size: TerminalSize) {}
        fn signal_foreground(&self, _signal: TtySignal) {}
    }

    struct NoopReaper;
    impl SessionReaper for NoopReaper {
        fn kill_session(&self) {}
    }

    fn tty_state_with_sink(
        chunk: usize,
        budget: Option<usize>,
    ) -> (TtyState, Arc<Mutex<FakeSinkState>>) {
        let (sink, state) = FakeSink::new(chunk, budget);
        let tty = TtyState::new(Box::new(sink), Arc::new(NoopControl), Arc::new(NoopReaper));
        (tty, state)
    }

    #[tokio::test]
    async fn write_stdin_reports_full_write_delivered_in_chunks() {
        // chunk=2 forces the 5-byte payload to land in pieces, but a fully
        // delivered write still reports accepted_len == requested length.
        let (tty, sink) = tty_state_with_sink(2, None);
        let out = tty.write_stdin(0, b"abcde", false).await.unwrap();
        assert_eq!(out.accepted_len, 5);
        assert_eq!(out.next_offset, 5);
        assert!(!out.closed);
        assert_eq!(lock_sink(&sink).written, b"abcde");
    }

    #[tokio::test]
    async fn write_stdin_partial_write_advances_only_by_bytes_written() {
        // The sink accepts 2 bytes then errors. The offset machine must advance
        // by exactly the 2 bytes that landed; a retry at the new offset writes
        // the remainder with NO re-delivery of the first 2 bytes (dup-on-retry).
        let (tty, sink) = tty_state_with_sink(2, Some(2));
        let out = tty.write_stdin(0, b"abcde", false).await.unwrap();
        assert_eq!(out.accepted_len, 2);
        assert_eq!(out.next_offset, 2);
        assert!(!out.closed);
        assert_eq!(lock_sink(&sink).written, b"ab");

        // Heal the sink and retry the remainder from next_offset.
        {
            let mut st = lock_sink(&sink);
            st.budget = None;
            st.chunk = 0;
        }
        let out2 = tty.write_stdin(2, b"cde", false).await.unwrap();
        assert_eq!(out2.accepted_len, 3);
        assert_eq!(out2.next_offset, 5);
        // Exactly "abcde" — no byte re-delivered.
        assert_eq!(lock_sink(&sink).written, b"abcde");
    }

    #[tokio::test]
    async fn write_stdin_zero_progress_on_dead_master_is_error_and_does_not_advance() {
        // budget=0: the first poll_write errors with zero progress on a non-empty
        // payload → a hard StdinClosed, and the offset must NOT advance.
        let (tty, sink) = tty_state_with_sink(0, Some(0));
        assert_eq!(
            tty.write_stdin(0, b"x", false).await.unwrap_err(),
            ExecError::StdinClosed
        );
        assert!(lock_sink(&sink).written.is_empty());
        // The offset machine never advanced, so a fresh write at 0 is admitted.
        {
            let mut st = lock_sink(&sink);
            st.budget = None;
        }
        let out = tty.write_stdin(0, b"y", false).await.unwrap();
        assert_eq!(out.next_offset, 1);
    }

    #[tokio::test]
    async fn close_after_injects_single_veof_only_when_full_payload_lands() {
        let (tty, sink) = tty_state_with_sink(0, None);
        let out = tty.write_stdin(0, b"hi", true).await.unwrap();
        assert_eq!(out.accepted_len, 2);
        assert_eq!(out.next_offset, 2);
        assert!(out.closed);
        // Payload + exactly one trailing VEOF, and VEOF is NOT counted in offset.
        assert_eq!(lock_sink(&sink).written, b"hi\x04");
        assert!(tty.stdin_closed());
    }

    #[tokio::test]
    async fn teardown_release_writer_aborts_a_write_blocked_on_a_full_pty() {
        // THE deadlock case: a child that stops reading stdin fills the PTY,
        // so the writer task blocks indefinitely on poll_write. Teardown's
        // release_writer() MUST drop the master write clone by aborting the task
        // — never by contending for a lock the blocked write holds — so it
        // returns promptly and the blocked WriteStdin surfaces StdinClosed.
        let (tty, sink) = tty_state_with_sink(0, None);
        let tty = Arc::new(tty);
        lock_sink(&sink).paused = true;
        let writer = {
            let tty = Arc::clone(&tty);
            tokio::spawn(async move { tty.write_stdin(0, b"blocked", false).await })
        };
        // Let the writer task pick up the op and park on the paused sink.
        tokio::time::sleep(Duration::from_millis(20)).await;
        // Must not deadlock: bounded by a generous timeout.
        tokio::time::timeout(Duration::from_secs(5), tty.release_writer())
            .await
            .expect("release_writer must not block on the stuck write");
        let result = tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("blocked write must be released")
            .unwrap();
        assert_eq!(result, Err(ExecError::StdinClosed));
        // Nothing reached the (paused) sink.
        assert!(lock_sink(&sink).written.is_empty());
    }

    #[tokio::test]
    async fn writes_and_control_after_begin_closing_are_refused_with_no_side_effect() {
        let (tty, sink) = tty_state_with_sink(0, None);
        assert!(tty.begin_closing());
        assert_eq!(
            tty.write_stdin(0, b"x", false).await.unwrap_err(),
            ExecError::StdinClosed
        );
        assert_eq!(
            tty.close_stdin(0).await.unwrap_err(),
            ExecError::StdinClosed
        );
        assert_eq!(tty.resize(1, 40, 80).unwrap_err(), ExecError::ExecClosing);
        assert_eq!(tty.signal(1, 2).unwrap_err(), ExecError::ExecClosing);
        // No stdin bytes, and the control seq was never advanced.
        assert!(lock_sink(&sink).written.is_empty());
        assert_eq!(tty.last_control_seq(), 0);
    }

    #[tokio::test]
    async fn write_that_lands_after_begin_closing_does_not_commit_offset() {
        // H1 commit-gate: a write admitted while Running parks mid-flight on a
        // paused master; begin_closing then wins the race and the parked write
        // resumes and physically lands its bytes. The committed protocol state
        // (offset advance + success ack) MUST still be atomic w.r.t.
        // begin_closing: the op surfaces StdinClosed and the offset machine does
        // NOT advance, even though the bytes reached the master before the abort
        // could fire.
        let (tty, sink) = tty_state_with_sink(0, None);
        let tty = Arc::new(tty);
        lock_sink(&sink).paused = true;
        let writer = {
            let tty = Arc::clone(&tty);
            tokio::spawn(async move { tty.write_stdin(0, b"late", false).await })
        };
        // Let the writer task pass admission (still Running) and park on the
        // paused sink mid-write.
        tokio::time::sleep(Duration::from_millis(20)).await;
        // Enter Closing while the write is parked in flight.
        assert!(tty.begin_closing());
        // Resume the sink so the parked write completes and reaches the commit
        // gate, which now observes Closing.
        resume_sink(&sink);
        let result = tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("write must complete")
            .unwrap();
        assert_eq!(result, Err(ExecError::StdinClosed));
        // The bytes physically landed on the master (the write was in flight)...
        assert_eq!(lock_sink(&sink).written, b"late");
        // ...but the committed offset machine did NOT advance: no protocol-level
        // side effect is reported after Closing.
        assert_eq!(tty.stdin_next_offset(), 0);
    }

    #[tokio::test]
    async fn out_of_allowlist_signal_does_not_consume_the_control_seq() {
        // An out-of-allowlist signal must be rejected BEFORE admit(), so the
        // sequence stays available for a subsequent valid control message.
        let (tty, _sink) = tty_state_with_sink(0, None);
        assert_eq!(tty.signal(5, 11).unwrap_err(), ExecError::InvalidSignal);
        assert_eq!(tty.last_control_seq(), 0, "seq must not be consumed");
        // The same seq is still usable for a valid signal.
        tty.signal(5, 2).unwrap();
        assert_eq!(tty.last_control_seq(), 5);
    }

    #[test]
    fn w14_types_do_not_leak_payload_in_debug() {
        // Every ExecError variant is a unit variant: Debug is just the name.
        for (err, name) in [
            (ExecError::InvalidTerminalSize, "InvalidTerminalSize"),
            (ExecError::TtyStderrUnavailable, "TtyStderrUnavailable"),
            (ExecError::TtyRequired, "TtyRequired"),
            (ExecError::StdinClosed, "StdinClosed"),
            (ExecError::StdinOffsetMismatch, "StdinOffsetMismatch"),
            (
                ExecError::StdinByteBudgetExhausted,
                "StdinByteBudgetExhausted",
            ),
            (ExecError::StdinBackpressure, "StdinBackpressure"),
            (ExecError::ControlSeqMismatch, "ControlSeqMismatch"),
            (ExecError::InvalidSignal, "InvalidSignal"),
            (ExecError::ExecClosing, "ExecClosing"),
        ] {
            assert_eq!(format!("{err:?}"), name);
        }
    }
}
