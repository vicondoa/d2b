//! Interactive TTY exec (W14): PTY plumbing for the connection-owned,
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
//! and in the runner helper, never in the W12 attached spawner
//! (`exec.rs`/`exec_linux.rs`) — see `tests/guest-exec-runtime-static.sh`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::exec::{ExecError, ProcessWaiter, ValidatedCommand};

/// Default terminal geometry applied when a TTY create omits an
/// `initial_terminal_size`. A present size must validate (1..=65535); only an
/// absent size defaults.
pub const DEFAULT_TERMINAL_ROWS: u16 = 24;
pub const DEFAULT_TERMINAL_COLS: u16 = 80;
/// Inclusive bounds for a terminal dimension. Matches the existing wire
/// contract (no new schema bound), so a 0 or out-of-range dimension is rejected
/// rather than silently clamped.
pub const MIN_TERMINAL_DIM: u32 = 1;
pub const MAX_TERMINAL_DIM: u32 = 65535;

/// `VEOF` control byte (Ctrl-D). Injected on `CloseStdin` / `WriteStdin`
/// `close_after` to signal end-of-input to the foreground reader while the PTY
/// master stays open (half-close is modelled as VEOF, never a master close).
pub const VEOF: u8 = 0x04;

/// Validated terminal geometry. Both dimensions are within 1..=65535.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl TerminalSize {
    /// The default geometry (24x80) used only when an initial size is absent.
    pub const fn defaulted() -> Self {
        Self {
            rows: DEFAULT_TERMINAL_ROWS,
            cols: DEFAULT_TERMINAL_COLS,
        }
    }

    /// Validate a wire-supplied geometry against the existing 1..=65535 bound.
    pub fn checked(rows: u32, cols: u32) -> Result<Self, ExecError> {
        let valid = |d: u32| (MIN_TERMINAL_DIM..=MAX_TERMINAL_DIM).contains(&d);
        if !valid(rows) || !valid(cols) {
            return Err(ExecError::InvalidTerminalSize);
        }
        Ok(Self {
            rows: rows as u16,
            cols: cols as u16,
        })
    }

    /// Resolve an optional initial size: absent defaults to 24x80, present must
    /// validate (a present 0/out-of-range geometry is rejected, never
    /// defaulted).
    pub fn resolve_initial(initial: Option<(u32, u32)>) -> Result<Self, ExecError> {
        match initial {
            None => Ok(Self::defaulted()),
            Some((rows, cols)) => Self::checked(rows, cols),
        }
    }
}

/// Frozen signal allowlist for `ExecSignal` against a TTY foreground process
/// group. Any signal outside this set is rejected before delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtySignal {
    Int,
    Term,
    Hup,
    Quit,
    Winch,
    Usr1,
    Usr2,
    Kill,
    Tstp,
    Cont,
}

impl TtySignal {
    /// Map a wire signal number to the allowlist, rejecting any other value.
    pub fn from_raw(signal: u32) -> Option<Self> {
        // Standard Linux signal numbers; the allowlist is frozen in the
        // guest-control exec reference.
        Some(match signal {
            1 => Self::Hup,
            2 => Self::Int,
            3 => Self::Quit,
            9 => Self::Kill,
            10 => Self::Usr1,
            12 => Self::Usr2,
            15 => Self::Term,
            18 => Self::Cont,
            20 => Self::Tstp,
            28 => Self::Winch,
            _ => return None,
        })
    }

    /// The raw Linux signal number for this allowlisted signal.
    pub fn raw(self) -> i32 {
        match self {
            Self::Hup => 1,
            Self::Int => 2,
            Self::Quit => 3,
            Self::Kill => 9,
            Self::Usr1 => 10,
            Self::Usr2 => 12,
            Self::Term => 15,
            Self::Cont => 18,
            Self::Tstp => 20,
            Self::Winch => 28,
        }
    }
}

/// Pure stdin offset machine for a TTY session. WriteStdin must arrive in
/// monotonic, gap-free offset order; a duplicate/out-of-order offset is
/// rejected, and any write after a close (VEOF) is rejected. Mutated only under
/// the per-session writer lock so accept→write→advance is atomic per exec.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StdinLogic {
    next_offset: u64,
    closed: bool,
}

impl StdinLogic {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_offset(&self) -> u64 {
        self.next_offset
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Validate a WriteStdin at `offset`. Rejects writes after close and any
    /// non-contiguous offset. Does NOT advance — call [`advance`](Self::advance)
    /// only after the bytes are durably written to the master.
    pub fn admit(&self, offset: u64) -> Result<(), ExecError> {
        if self.closed {
            return Err(ExecError::StdinClosed);
        }
        if offset != self.next_offset {
            return Err(ExecError::StdinOffsetMismatch);
        }
        Ok(())
    }

    /// Advance the offset cursor after `len` bytes were written.
    pub fn advance(&mut self, len: u64) {
        self.next_offset = self.next_offset.saturating_add(len);
    }

    /// Mark stdin closed (idempotent). Returns true if this call performed the
    /// transition (false if it was already closed).
    pub fn close(&mut self) -> bool {
        if self.closed {
            return false;
        }
        self.closed = true;
        true
    }
}

/// Pure control-seq dispatcher shared by resize AND signal. Control messages
/// carry a strictly-increasing `control_seq`; a stale, duplicate, or
/// out-of-order seq is rejected with `ControlSeqMismatch`. Gaps are allowed
/// (the client owns seq allocation).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ControlSeqState {
    last_seq: u64,
}

impl ControlSeqState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// Admit a control message at `seq`, requiring strict monotonic increase.
    pub fn admit(&mut self, seq: u64) -> Result<(), ExecError> {
        if seq <= self.last_seq {
            return Err(ExecError::ControlSeqMismatch);
        }
        self.last_seq = seq;
        Ok(())
    }
}

/// Teardown lifecycle for a TTY session: `Running → Closing → Terminal`.
/// Entering `Closing` atomically rejects new stdin/control RPCs (typed no-op)
/// and is the single-shot gate that drives master release + session reap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtyPhase {
    Running,
    Closing,
    Terminal,
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

/// A spawned PTY-backed interactive exec. Distinct from the W12
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

/// Per-session interactive state, held by the owning [`crate::exec::ExecEntry`].
/// Mutable protocol state lives behind short, non-await std mutexes; the master
/// write half lives behind a tokio mutex so a WriteStdin serializes its
/// accept→write→advance under one lock without ever blocking the runtime.
pub struct TtyState {
    /// Master write half (stdin sink). `None` once teardown releases it.
    writer: tokio::sync::Mutex<Option<Box<dyn AsyncWrite + Send + Unpin>>>,
    /// Master control surface (resize / foreground signal). `None` after
    /// teardown.
    control: Mutex<Option<Arc<dyn PtyControl>>>,
    /// Session reaper, retained for teardown (and for the entry's killer).
    reaper: Arc<dyn SessionReaper>,
    /// Pure stdin offset machine.
    stdin: Mutex<StdinLogic>,
    /// Pure control-seq dispatcher (resize + signal share it).
    seq: Mutex<ControlSeqState>,
    /// Teardown phase gate.
    phase: Mutex<TtyPhase>,
}

impl TtyState {
    pub fn new(
        writer: Box<dyn AsyncWrite + Send + Unpin>,
        control: Arc<dyn PtyControl>,
        reaper: Arc<dyn SessionReaper>,
    ) -> Self {
        Self {
            writer: tokio::sync::Mutex::new(Some(writer)),
            control: Mutex::new(Some(control)),
            reaper,
            stdin: Mutex::new(StdinLogic::new()),
            seq: Mutex::new(ControlSeqState::new()),
            phase: Mutex::new(TtyPhase::Running),
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

    /// Take the control surface, dropping the master clone it holds. Idempotent.
    pub fn take_control(&self) -> Option<Arc<dyn PtyControl>> {
        self.lock_control().take()
    }

    /// Admit + dispatch a resize. Rejects when closing (no side effect), on a
    /// stale/dup seq, or on an invalid geometry. The control surface resolves
    /// `SIGWINCH` to the foreground PG at delivery.
    pub fn resize(&self, seq: u64, rows: u32, cols: u32) -> Result<(), ExecError> {
        if self.is_closing() {
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
        if self.is_closing() {
            return Err(ExecError::ExecClosing);
        }
        self.lock_seq().admit(seq)?;
        let signal = TtySignal::from_raw(signal).ok_or(ExecError::InvalidSignal)?;
        if let Some(control) = self.lock_control().as_ref() {
            control.signal_foreground(signal);
        }
        Ok(())
    }

    /// Write `data` to the master at `offset`, optionally injecting VEOF
    /// afterwards (`close_after`). Serializes the whole accept→write→advance
    /// under the writer lock so concurrent WriteStdin handlers cannot interleave
    /// the offset machine. Returns the new next-offset.
    pub async fn write_stdin(
        &self,
        offset: u64,
        data: &[u8],
        close_after: bool,
    ) -> Result<u64, ExecError> {
        if self.is_closing() {
            return Err(ExecError::StdinClosed);
        }
        // Hold the writer lock for the full operation: the offset machine is
        // validated, the bytes are written, and the cursor advances atomically.
        let mut writer = self.writer.lock().await;
        // Re-check under the writer lock; teardown takes this lock to drop the
        // writer, so observing Some here means the master is still live.
        let Some(sink) = writer.as_mut() else {
            return Err(ExecError::StdinClosed);
        };
        self.lock_stdin().admit(offset)?;
        write_all(sink, data).await?;
        if close_after {
            write_all(sink, &[VEOF]).await?;
        }
        let mut stdin = self.lock_stdin();
        stdin.advance(data.len() as u64);
        if close_after {
            stdin.close();
        }
        Ok(stdin.next_offset())
    }

    /// Inject VEOF and mark stdin closed, keeping the master open. Idempotent:
    /// a second close is a no-op duplicate. `offset` must match the current
    /// next-offset. Returns `(final_offset, duplicate)`.
    pub async fn close_stdin(&self, offset: u64) -> Result<(u64, bool), ExecError> {
        if self.is_closing() {
            return Err(ExecError::StdinClosed);
        }
        let mut writer = self.writer.lock().await;
        {
            let stdin = self.lock_stdin();
            if stdin.is_closed() {
                // Idempotent: already closed, report the frozen final offset.
                return Ok((stdin.next_offset(), true));
            }
            if offset != stdin.next_offset() {
                return Err(ExecError::StdinOffsetMismatch);
            }
        }
        let Some(sink) = writer.as_mut() else {
            return Err(ExecError::StdinClosed);
        };
        write_all(sink, &[VEOF]).await?;
        let mut stdin = self.lock_stdin();
        stdin.close();
        Ok((stdin.next_offset(), false))
    }

    /// Take the master write half, dropping it. Used by teardown to release one
    /// of the master references (the last reference dropped sends `SIGHUP`).
    pub async fn release_writer(&self) {
        let _ = self.writer.lock().await.take();
    }
}

/// Bounded `write_all` over a boxed async writer. Maps any I/O failure to
/// `StdinClosed` (the master is gone / the reader hung up).
async fn write_all(
    sink: &mut Box<dyn AsyncWrite + Send + Unpin>,
    data: &[u8],
) -> Result<(), ExecError> {
    use tokio::io::AsyncWriteExt;
    sink.write_all(data)
        .await
        .map_err(|_| ExecError::StdinClosed)
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
    use std::task::{ready, Context, Poll};
    use std::time::Duration;

    use async_trait::async_trait;
    use rustix::fs::{Mode, OFlags};
    use rustix::io::{ioctl_fionbio, read, write};
    use rustix::process::{kill_process, kill_process_group, Pid, Signal};
    use rustix::pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags};
    use rustix::termios::{tcgetpgrp, tcsetwinsize, Winsize};
    use tokio::io::unix::AsyncFd;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::process::{Child, Command};

    use super::{
        PtyControl, PtyProcessSpawner, SessionReaper, SpawnedPtyProcess, TerminalSize, TtySignal,
    };
    use crate::exec::{ExecError, ExitOutcome, ProcessWaiter, ValidatedCommand};

    /// Bounded SIGKILL sweep rounds for the session reaper.
    const REAP_MAX_ROUNDS: u32 = 50;
    /// Sleep between reaper sweep rounds.
    const REAP_ROUND_SLEEP_MS: u64 = 5;
    /// `EIO` raw value (Linux): a PTY master read returns it once the last slave
    /// closes. Treated as a clean EOF. Pinned to avoid a libc dependency.
    const EIO: i32 = 5;

    /// Production PTY spawner. Constructed with the absolute path to the
    /// `nixling-exec-runner` binary, which it invokes in `--tty-exec` mode.
    pub struct LinuxPtyProcessSpawner {
        helper_path: PathBuf,
    }

    impl LinuxPtyProcessSpawner {
        pub fn new(helper_path: PathBuf) -> Self {
            Self { helper_path }
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
            // Open the slave O_NOCTTY so this open does not make guestd acquire a
            // controlling terminal; the helper's TIOCSCTTY does that in the
            // child session. Not CLOEXEC: it is handed to the helper as stdin.
            let slave = rustix::fs::open(&slave_path, OFlags::RDWR | OFlags::NOCTTY, Mode::empty())
                .map_err(|_| ExecError::SpawnFailed)?;
            // CLOEXEC status pipe: the write end is handed to the helper as
            // stdout and closes on a successful exec (guestd reads EOF).
            let (status_r, status_w) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
                .map_err(|_| ExecError::SpawnFailed)?;

            let mut cmd = Command::new(&self.helper_path);
            cmd.arg("--tty-exec")
                .arg("--rows")
                .arg(initial_size.rows.to_string())
                .arg("--cols")
                .arg(initial_size.cols.to_string())
                .arg("--")
                .arg(&command.program)
                .args(&command.args)
                .current_dir(&command.cwd)
                .env_clear()
                .envs(command.env.iter().map(|(k, v)| (k.clone(), v.clone())))
                // Safe fd handoff: no arbitrary pass_fds, no process_group(0),
                // no pre_exec. The slave is stdin; the status pipe is stdout.
                .stdin(Stdio::from(slave))
                .stdout(Stdio::from(status_w))
                .stderr(Stdio::null())
                .kill_on_drop(false);

            let mut child = cmd.spawn().map_err(|_| ExecError::SpawnFailed)?;
            let pid = child.id().ok_or(ExecError::SpawnFailed)? as i32;

            // Await the helper handshake: EOF == exec succeeded; one byte == a
            // typed setup/exec failure.
            ioctl_fionbio(&status_r, true).map_err(|_| ExecError::SpawnFailed)?;
            let status_async = AsyncFd::new(status_r).map_err(|_| ExecError::SpawnFailed)?;
            let handshake = read_status_byte(&status_async).await;
            drop(status_async);
            match handshake {
                Ok(None) => {}
                Ok(Some(_failure_byte)) => {
                    let _ = child.wait().await;
                    return Err(ExecError::SpawnFailed);
                }
                Err(_) => {
                    let _ = child.wait().await;
                    return Err(ExecError::SpawnFailed);
                }
            }

            ioctl_fionbio(&master, true).map_err(|_| ExecError::SpawnFailed)?;
            let io = Arc::new(AsyncFd::new(master).map_err(|_| ExecError::SpawnFailed)?);
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
                reaper: Arc::new(ProcSessionReaper { sid: pid }),
            })
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
                match guard.try_io(|inner| write(inner.get_ref(), buf).map_err(std::io::Error::from))
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
            if let Ok(pgid) = tcgetpgrp(self.io.get_ref()) {
                if let Some(sig) = rustix_signal(signal) {
                    let _ = kill_process_group(pgid, sig);
                }
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
    /// equals the helper-created session leader, repeating until the session is
    /// empty (bounded).
    struct ProcSessionReaper {
        sid: i32,
    }

    impl SessionReaper for ProcSessionReaper {
        fn kill_session(&self) {
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

    #[test]
    fn terminal_size_defaults_when_absent() {
        assert_eq!(
            TerminalSize::resolve_initial(None).unwrap(),
            TerminalSize::defaulted()
        );
        assert_eq!(TerminalSize::defaulted().rows, 24);
        assert_eq!(TerminalSize::defaulted().cols, 80);
    }

    #[test]
    fn terminal_size_present_must_validate() {
        assert!(TerminalSize::resolve_initial(Some((0, 80))).is_err());
        assert!(TerminalSize::resolve_initial(Some((24, 0))).is_err());
        assert!(TerminalSize::resolve_initial(Some((70000, 80))).is_err());
        let ok = TerminalSize::resolve_initial(Some((40, 120))).unwrap();
        assert_eq!((ok.rows, ok.cols), (40, 120));
    }

    #[test]
    fn stdin_logic_rejects_dup_and_out_of_order() {
        let mut logic = StdinLogic::new();
        assert!(logic.admit(0).is_ok());
        logic.advance(5);
        assert_eq!(logic.next_offset(), 5);
        // Replay of an old offset.
        assert_eq!(logic.admit(0), Err(ExecError::StdinOffsetMismatch));
        // Gap / out-of-order future offset.
        assert_eq!(logic.admit(7), Err(ExecError::StdinOffsetMismatch));
        assert!(logic.admit(5).is_ok());
    }

    #[test]
    fn stdin_logic_close_is_idempotent_and_rejects_later_writes() {
        let mut logic = StdinLogic::new();
        logic.advance(3);
        assert!(logic.close());
        assert!(!logic.close());
        assert_eq!(logic.admit(3), Err(ExecError::StdinClosed));
    }

    #[test]
    fn control_seq_requires_strict_increase() {
        let mut seq = ControlSeqState::new();
        assert!(seq.admit(1).is_ok());
        assert!(seq.admit(2).is_ok());
        // Duplicate.
        assert_eq!(seq.admit(2), Err(ExecError::ControlSeqMismatch));
        // Stale.
        assert_eq!(seq.admit(1), Err(ExecError::ControlSeqMismatch));
        // Gaps are allowed.
        assert!(seq.admit(10).is_ok());
    }

    #[test]
    fn tty_signal_allowlist_round_trips() {
        for raw in [1, 2, 3, 9, 10, 12, 15, 18, 20, 28] {
            let sig = TtySignal::from_raw(raw).expect("allowlisted");
            assert_eq!(sig.raw(), raw as i32);
        }
        // Outside the allowlist.
        assert!(TtySignal::from_raw(11).is_none());
        assert!(TtySignal::from_raw(0).is_none());
    }
}
