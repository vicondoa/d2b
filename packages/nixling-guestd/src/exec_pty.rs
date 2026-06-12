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
//! This module owns only the PTY *mechanism* behind fakeable traits. The
//! low-level PTY allocation/control syscalls (`openpt`/`grantpt`/`unlockpt`/
//! `ptsname`/`TIOCSWINSZ`/`tcgetpgrp`) live here and in the runner helper, never
//! in the W12 attached spawner (`exec.rs`/`exec_linux.rs`) — see
//! `tests/guest-exec-runtime-static.sh`.

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
    pub control: std::sync::Arc<dyn PtyControl>,
    /// Reaps the direct child (helper → target), owned by the supervisor.
    pub waiter: Box<dyn ProcessWaiter>,
    /// SIGKILLs every process remaining in the TTY session on teardown.
    pub reaper: std::sync::Arc<dyn SessionReaper>,
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

/// Allocates a PTY pair and spawns the first-party TTY helper as the session
/// leader with the slave as its controlling terminal, returning the connected
/// [`SpawnedPtyProcess`]. The production implementation lands with the guestd
/// PTY runtime; a fake duplex implementation backs the deterministic tests.
#[async_trait]
pub trait PtyProcessSpawner: Send + Sync + 'static {
    async fn spawn(
        &self,
        command: ValidatedCommand,
        initial_size: TerminalSize,
    ) -> Result<SpawnedPtyProcess, ExecError>;
}
