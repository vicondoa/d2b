//! CLI-side `d2b vm exec` failure classification + host terminal safety.
//!
//! `d2b vm exec` drives the typed v2 `ComponentSession` terminal stream (see
//! `cmd_vm_exec_v2` / `run_terminal_stream_v2` in `lib.rs`); the guest owns
//! the PTY (helper-exec) and the CLI never allocates a host PTY. This module
//! provides: the reserved exec CLI exit-code contract + wire-`kind` →
//! exit-code/source mapping (`ExecClientError`/`exit_for_kind`), the signal
//! set forwarded into the guest (`ExecSignal`), and the host-termios/signal
//! safety primitives (`FdStateGuard`, blocked-signal-mask helpers,
//! `RealHostIo`/`CapturingHostIo`) shared with the typed v2 exec path.

use std::collections::VecDeque;
use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};

use crate::terminal_client::{TerminalHostIo, TerminalSignalSource};

// Reserved exec CLI exit codes. Guest WIFEXITED 0-255 codes pass through
// and CAN collide with these reserved numbers (e.g. a guest that exits 70 vs.
// the old-generation transport class); `--json` disambiguates via
// `source`/`reason`/`guestExitCode`/`transportExitCode`. These deliberately
// avoid the pre-existing CLI exit codes 2/3/33/78.
/// Transport unreachable or a per-op/establishment deadline elapsed.
pub const EXIT_EXEC_TRANSPORT: i32 = 69;
/// The VM generation does not support guest-control exec, or it lacks a
/// required exec capability. Reuses the guest-control-config class (70).
pub const EXIT_EXEC_OLD_GENERATION: i32 = 70;
/// The exec session table is at capacity, or Start was rate limited.
pub const EXIT_EXEC_CAPACITY: i32 = 75;
/// The guest returned a malformed/out-of-contract response, or rejected the op.
pub const EXIT_EXEC_PROTOCOL: i32 = 76;
/// The authenticated guest-control handshake was rejected.
pub const EXIT_EXEC_AUTH: i32 = 77;
/// Daemon-internal or CLI-internal failure driving the session.
pub const EXIT_EXEC_INTERNAL: i32 = 42;

/// Where a terminal failure originated. Surfaced in the `--json` envelope so a
/// consumer can disambiguate a guest exit code from a transport exit code that
/// happens to share a shell status number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecFailureSource {
    /// vsock connect / handshake transport, or a deadline.
    Transport,
    /// The guest authenticated but the VM/guest rejected the request
    /// (old-generation, capability, capacity, rate-limit, auth), or the
    /// daemon's guest-control admin gate refused the caller (not-admin).
    GuestControl,
    /// Malformed or out-of-contract response.
    Protocol,
    /// CLI/daemon-internal failure.
    Internal,
}

impl ExecFailureSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::GuestControl => "guest-control",
            Self::Protocol => "protocol",
            Self::Internal => "internal",
        }
    }
}

/// A terminal exec-client failure (transport/auth/old-gen/protocol/internal).
/// Carries the redaction-safe wire `kind` slug, the mapped CLI exit code, and
/// a human message + remediation. NEVER carries argv/env/output bytes.
#[derive(Debug, Clone)]
pub struct ExecClientError {
    pub kind: String,
    pub exit_code: i32,
    pub source: ExecFailureSource,
    pub message: String,
    pub remediation: String,
}

impl ExecClientError {
    fn new(
        kind: impl Into<String>,
        exit_code: i32,
        source: ExecFailureSource,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            exit_code,
            source,
            message: message.into(),
            remediation: remediation.into(),
        }
    }

    pub fn transport(message: impl Into<String>) -> Self {
        Self::new(
            "guest-control-transport-unavailable",
            EXIT_EXEC_TRANSPORT,
            ExecFailureSource::Transport,
            message,
            "confirm the VM is running and guest-control-health is ready (`d2b vm status <vm>`), then retry",
        )
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(
            "guest-control-protocol-error",
            EXIT_EXEC_PROTOCOL,
            ExecFailureSource::Protocol,
            message,
            "the guest-control protocol is skewed; rebuild the guest with a matching d2b generation",
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            "guest-control-exec-internal",
            EXIT_EXEC_INTERNAL,
            ExecFailureSource::Internal,
            message,
            "retry; if the failure persists inspect the daemon log for the typed exec-session record",
        )
    }

    /// Map a daemon `error` frame (wire `kind` slug + message + remediation +
    /// daemon exit code) to the CLI exec exit-code contract. The CLI owns the
    /// exit code so the contract is stable regardless of the daemon fallback.
    pub fn from_daemon_error(
        kind: &str,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        let (exit_code, source) = exit_for_kind(kind);
        Self::new(kind.to_owned(), exit_code, source, message, remediation)
    }
}

/// The CLI exit code + failure source for a daemon wire `kind` slug.
pub fn exit_for_kind(kind: &str) -> (i32, ExecFailureSource) {
    match kind {
        "guest-control-transport-unavailable" | "guest-control-timeout" => {
            (EXIT_EXEC_TRANSPORT, ExecFailureSource::Transport)
        }
        "guest-control-unavailable-old-generation"
        | "guest-control-capability-unavailable"
        | "guest-control-exec-detached-unavailable" => {
            (EXIT_EXEC_OLD_GENERATION, ExecFailureSource::GuestControl)
        }
        "exec-session-capacity" | "exec-session-rate-limited" => {
            (EXIT_EXEC_CAPACITY, ExecFailureSource::GuestControl)
        }
        "guest-control-protocol-error"
        | "guest-control-exec-error"
        | "guest-control-exec-not-found"
        | "guest-control-exec-expired" => (EXIT_EXEC_PROTOCOL, ExecFailureSource::Protocol),
        "guest-control-invalid-program" => (2, ExecFailureSource::GuestControl),
        "guest-control-auth-failed" => (EXIT_EXEC_AUTH, ExecFailureSource::GuestControl),
        "guest-control-stale-session" => (EXIT_EXEC_AUTH, ExecFailureSource::GuestControl),
        // The daemon's admin gate refused the caller before any guest contact
        // (caller not in `d2b.site.adminUsers`). It is an authorization
        // failure, NOT an internal bug — map it to the AUTH reserved code so
        // it does not fall through to the internal (42) default.
        "authz-not-admin" => (EXIT_EXEC_AUTH, ExecFailureSource::GuestControl),
        "guest-control-exec-internal" => (EXIT_EXEC_INTERNAL, ExecFailureSource::Internal),
        _ => (EXIT_EXEC_INTERNAL, ExecFailureSource::Internal),
    }
}

/// Host signal events the CLI forwards into the guest exec (enqueue-only; the
/// real signal source never touches termios or syscalls in a handler).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecSignal {
    /// Terminal window resized → forward a guest PTY `Resize` (tty mode only).
    Winch,
    /// SIGINT → guest signal 2.
    Interrupt,
    /// SIGTERM → guest signal 15.
    Terminate,
    /// SIGTSTP → guest signal 20.
    Stop,
    /// SIGHUP → guest signal 1.
    Hangup,
    /// SIGQUIT → guest signal 3.
    Quit,
}

// ---------------------------------------------------------------------------
// Host terminal safety: a guard that restores termios + O_NONBLOCK on
// EVERY exit/error/disconnect/panic via Drop. Uses only the safe rustix
// termios/fcntl wrappers — no `unsafe`.
// ---------------------------------------------------------------------------

/// Host stdin terminal operations behind a trait so the ordering logic in
/// `FdStateGuard::enter` can be exercised hermetically (the production impl
/// targets the real stdin fd, which is not a tty under test).
trait HostTtyOps {
    /// Switch the terminal to raw mode, remembering the original state for a
    /// later restore.
    fn enter_raw(&self) -> io::Result<()>;
    /// Restore the original termios saved by `enter_raw`.
    fn restore_termios(&self);
    /// Add `O_NONBLOCK` to stdin if absent. Returns `true` if it was newly set.
    fn try_add_nonblock(&self) -> io::Result<bool>;
    /// Clear an `O_NONBLOCK` flag this guard added.
    fn clear_nonblock(&self);
}

/// Production `HostTtyOps` over the real stdin fd. The original termios is held
/// in interior storage so `enter_raw`/`restore_termios` can take `&self`.
struct RealStdinTty {
    original: std::cell::RefCell<Option<rustix::termios::Termios>>,
}

impl RealStdinTty {
    fn new() -> Self {
        Self {
            original: std::cell::RefCell::new(None),
        }
    }
}

impl HostTtyOps for RealStdinTty {
    fn enter_raw(&self) -> io::Result<()> {
        let fd = rustix::stdio::stdin();
        let original = rustix::termios::tcgetattr(fd).map_err(errno_to_io)?;
        let mut raw_termios = original.clone();
        raw_termios.make_raw();
        rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Flush, &raw_termios)
            .map_err(errno_to_io)?;
        *self.original.borrow_mut() = Some(original);
        Ok(())
    }

    fn restore_termios(&self) {
        let fd = rustix::stdio::stdin();
        if let Some(original) = self.original.borrow().as_ref() {
            let _ =
                rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Flush, original);
        }
    }

    fn try_add_nonblock(&self) -> io::Result<bool> {
        let fd = rustix::stdio::stdin();
        let flags = rustix::fs::fcntl_getfl(fd).map_err(errno_to_io)?;
        if flags.contains(rustix::fs::OFlags::NONBLOCK) {
            return Ok(false);
        }
        rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(errno_to_io)?;
        Ok(true)
    }

    fn clear_nonblock(&self) {
        let fd = rustix::stdio::stdin();
        if let Ok(flags) = rustix::fs::fcntl_getfl(fd) {
            let _ = rustix::fs::fcntl_setfl(fd, flags & !rustix::fs::OFlags::NONBLOCK);
        }
    }
}

/// RAII guard over host stdin terminal state. On `enter` it optionally puts the
/// terminal in raw mode and/or marks stdin non-blocking; on drop it restores
/// the saved termios and clears any O_NONBLOCK it set. Restoration is
/// idempotent and never panics.
pub struct FdStateGuard {
    ops: Box<dyn HostTtyOps>,
    raw_active: bool,
    nonblock_added: bool,
    restored: bool,
}

impl FdStateGuard {
    /// Enter the requested host stdin state. `raw` puts the terminal in raw
    /// mode (the guest owns echo/line discipline via its PTY); `nonblock`
    /// marks stdin O_NONBLOCK so the FSM can poll it without blocking.
    pub fn enter(raw: bool, nonblock: bool) -> io::Result<Self> {
        Self::enter_with(Box::new(RealStdinTty::new()), raw, nonblock)
    }

    /// Core ordering shared by production and tests. The guard is constructed
    /// (and owns its restore state) BEFORE the fallible O_NONBLOCK step, so a
    /// failure there restores the already-applied raw mode via the guard
    /// instead of leaving stdin stuck raw.
    fn enter_with(ops: Box<dyn HostTtyOps>, raw: bool, nonblock: bool) -> io::Result<Self> {
        let mut guard = Self {
            ops,
            raw_active: false,
            nonblock_added: false,
            restored: false,
        };
        if raw {
            // If raw-mode entry itself fails there is nothing to restore yet.
            guard.ops.enter_raw()?;
            guard.raw_active = true;
        }
        if nonblock {
            match guard.ops.try_add_nonblock() {
                Ok(added) => guard.nonblock_added = added,
                Err(error) => {
                    // Raw mode is already applied: restore it before bubbling
                    // the error so the terminal is never left in raw mode.
                    guard.restore();
                    return Err(error);
                }
            }
        }
        Ok(guard)
    }

    /// Restore the saved terminal state. Safe to call more than once.
    pub fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        if self.raw_active {
            self.ops.restore_termios();
        }
        if self.nonblock_added {
            self.ops.clear_nonblock();
        }
    }
}

impl Drop for FdStateGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn errno_to_io(errno: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(errno.raw_os_error())
}

/// Read available host stdin bytes without blocking, on the real stdin fd.
fn read_stdin_nonblocking(buf: &mut [u8]) -> io::Result<usize> {
    match rustix::io::read(rustix::stdio::stdin(), buf) {
        Ok(read) => Ok(read),
        Err(errno)
            if errno == rustix::io::Errno::AGAIN || errno == rustix::io::Errno::WOULDBLOCK =>
        {
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        }
        Err(errno) if errno == rustix::io::Errno::INTR => {
            Err(io::Error::from(io::ErrorKind::Interrupted))
        }
        Err(errno) => Err(errno_to_io(errno)),
    }
}

/// Production host I/O: stdin via the real fd 0 (non-blocking), stdout/stderr
/// via the std locked handles, window size from the controlling terminal.
pub struct RealHostIo;

impl TerminalHostIo for RealHostIo {
    fn read_stdin(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read_stdin_nonblocking(buf)
    }

    fn write_stdout(&mut self, data: &[u8]) -> io::Result<()> {
        let mut handle = io::stdout().lock();
        handle.write_all(data)?;
        handle.flush()
    }

    fn write_stderr(&mut self, data: &[u8]) -> io::Result<()> {
        let mut handle = io::stderr().lock();
        handle.write_all(data)?;
        handle.flush()
    }

    fn window_size(&self) -> Option<(u32, u32)> {
        current_window_size()
    }
}

/// The controlling terminal window size as `(rows, cols)`, if stdout is a tty.
pub fn current_window_size() -> Option<(u32, u32)> {
    let winsize = rustix::termios::tcgetwinsize(rustix::stdio::stdout()).ok()?;
    Some((u32::from(winsize.ws_row), u32::from(winsize.ws_col)))
}

/// Bounded capturing host I/O for `--json` mode: stdout/stderr go to in-memory
/// buffers (charset-safe base64 in the envelope), stdin still reads the real
/// fd when interactive.
pub struct CapturingHostIo {
    interactive: bool,
    cap: usize,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl CapturingHostIo {
    pub fn new(interactive: bool, cap: usize) -> Self {
        Self {
            interactive,
            cap,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }

    pub fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }

    fn capture(buffer: &mut Vec<u8>, truncated: &mut bool, cap: usize, data: &[u8]) {
        if buffer.len() >= cap {
            *truncated = true;
            return;
        }
        let room = cap - buffer.len();
        if data.len() > room {
            buffer.extend_from_slice(&data[..room]);
            *truncated = true;
        } else {
            buffer.extend_from_slice(data);
        }
    }
}

impl TerminalHostIo for CapturingHostIo {
    fn read_stdin(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.interactive {
            read_stdin_nonblocking(buf)
        } else {
            Ok(0)
        }
    }

    fn write_stdout(&mut self, data: &[u8]) -> io::Result<()> {
        Self::capture(&mut self.stdout, &mut self.stdout_truncated, self.cap, data);
        Ok(())
    }

    fn write_stderr(&mut self, data: &[u8]) -> io::Result<()> {
        Self::capture(&mut self.stderr, &mut self.stderr_truncated, self.cap, data);
        Ok(())
    }

    fn window_size(&self) -> Option<(u32, u32)> {
        current_window_size()
    }
}

// ---------------------------------------------------------------------------
// Signal source: block the forwarded signals process-wide and let a
// dedicated thread sigwait + enqueue. Enqueue-only — no termios/syscalls in a
// handler, and no `unsafe` (nix `SigSet` wrappers are safe).
// ---------------------------------------------------------------------------

/// Installed signal source backed by a sigwait thread. `drain` returns the
/// events enqueued since the last poll.
pub struct InstalledSignals {
    pending: Arc<Mutex<VecDeque<ExecSignal>>>,
}

pub struct ForwardedSignalMask {
    previous: nix::sys::signal::SigSet,
    forwarded: nix::sys::signal::SigSet,
}

impl std::fmt::Debug for ForwardedSignalMask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ForwardedSignalMask([restored-on-drop])")
    }
}

impl Drop for ForwardedSignalMask {
    fn drop(&mut self) {
        let _ = nix::sys::signal::pthread_sigmask(
            nix::sys::signal::SigmaskHow::SIG_SETMASK,
            Some(&self.previous),
            None,
        );
    }
}

impl TerminalSignalSource for InstalledSignals {
    type Signal = ExecSignal;

    fn drain(&mut self) -> Vec<ExecSignal> {
        let mut queue = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }
}

/// Block SIGWINCH/SIGINT/SIGTERM/SIGTSTP/SIGHUP/SIGQUIT on the calling (main)
/// thread — so spawned threads inherit the block and the terminal-driven
/// signals are forwarded into the guest rather than acting on the host CLI —
/// and spawn a sigwait thread that enqueues each as an [`ExecSignal`].
pub fn install_signals() -> io::Result<(ForwardedSignalMask, InstalledSignals)> {
    let guard = block_forwarded_signals()?;
    let installed = install_blocked_signals(&guard)?;
    Ok((guard, installed))
}

pub fn block_forwarded_signals() -> io::Result<ForwardedSignalMask> {
    use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};

    let mut forwarded = SigSet::empty();
    for signal in [
        Signal::SIGWINCH,
        Signal::SIGINT,
        Signal::SIGTERM,
        Signal::SIGTSTP,
        Signal::SIGHUP,
        Signal::SIGQUIT,
    ] {
        forwarded.add(signal);
    }
    let mut previous = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&forwarded), Some(&mut previous))
        .map_err(nix_errno_to_io)?;
    Ok(ForwardedSignalMask {
        previous,
        forwarded,
    })
}

pub fn install_blocked_signals(mask: &ForwardedSignalMask) -> io::Result<InstalledSignals> {
    use nix::sys::signal::Signal;

    let pending = Arc::new(Mutex::new(VecDeque::new()));
    let pending_thread = Arc::clone(&pending);
    let wait_set = mask.forwarded;
    std::thread::Builder::new()
        .name("d2b-exec-sig".to_owned())
        .spawn(move || {
            loop {
                match wait_set.wait() {
                    Ok(signal) => {
                        let mapped = match signal {
                            Signal::SIGWINCH => ExecSignal::Winch,
                            Signal::SIGINT => ExecSignal::Interrupt,
                            Signal::SIGTERM => ExecSignal::Terminate,
                            Signal::SIGTSTP => ExecSignal::Stop,
                            Signal::SIGHUP => ExecSignal::Hangup,
                            Signal::SIGQUIT => ExecSignal::Quit,
                            _ => continue,
                        };
                        pending_thread
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push_back(mapped);
                    }
                    Err(_) => continue,
                }
            }
        })?;

    Ok(InstalledSignals { pending })
}

fn nix_errno_to_io(errno: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(errno as i32)
}

// ===========================================================================
// Tests: (d) exit-code table / wire-slug mapping primitives; (g) FdStateGuard
// no-op/restore-idempotent + blocked-signal-mask restore-on-drop; (h)
// redaction (error constructors carry only static remediation text).
//
// The legacy hand-rolled exec-op FSM (`ExecOwnerTransport`/`ExecHostIo`/
// `ExecSignalSource` + `run_exec_fsm`) that these tests used to exercise was
// removed with the legacy `cmd_vm_exec` wire protocol: `d2b vm exec` now
// drives the typed v2 `ComponentSession` terminal stream exclusively (see
// `cmd_vm_exec_v2` / `run_terminal_stream_v2` in `lib.rs`).
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    // ---- (g) FdStateGuard no-op + idempotent restore ----------------------

    #[test]
    fn fd_state_guard_no_op_enter_restores_idempotently() {
        // enter(raw=false, nonblock=false) touches neither termios nor flags,
        // so it succeeds even when stdin is not a tty (test harness).
        let mut guard = FdStateGuard::enter(false, false).expect("no-op enter");
        guard.restore();
        guard.restore(); // idempotent, never panics
        drop(guard); // Drop restore is also safe after explicit restore
    }

    #[derive(Default)]
    struct RecordingTty {
        fail_nonblock: bool,
        events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    }

    impl HostTtyOps for RecordingTty {
        fn enter_raw(&self) -> io::Result<()> {
            self.events.borrow_mut().push("raw_set");
            Ok(())
        }
        fn restore_termios(&self) {
            self.events.borrow_mut().push("raw_restored");
        }
        fn try_add_nonblock(&self) -> io::Result<bool> {
            if self.fail_nonblock {
                self.events.borrow_mut().push("nonblock_failed");
                return Err(io::Error::from_raw_os_error(9));
            }
            self.events.borrow_mut().push("nonblock_set");
            Ok(true)
        }
        fn clear_nonblock(&self) {
            self.events.borrow_mut().push("nonblock_cleared");
        }
    }

    #[test]
    fn fd_state_guard_restores_raw_mode_when_nonblock_setup_fails() {
        // raw=true succeeds, then the O_NONBLOCK step fails: the terminal MUST
        // be restored out of raw mode before `enter` returns Err.
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let ops = RecordingTty {
            fail_nonblock: true,
            events: events.clone(),
        };
        let err = match FdStateGuard::enter_with(Box::new(ops), true, true) {
            Ok(_) => panic!("nonblock failure must propagate"),
            Err(err) => err,
        };
        assert_eq!(err.raw_os_error(), Some(9));
        assert_eq!(
            *events.borrow(),
            vec!["raw_set", "nonblock_failed", "raw_restored"],
            "raw mode must be restored on the nonblock error path"
        );
    }

    #[test]
    fn fd_state_guard_restores_raw_and_nonblock_on_drop() {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let ops = RecordingTty {
            fail_nonblock: false,
            events: events.clone(),
        };
        {
            let _guard = FdStateGuard::enter_with(Box::new(ops), true, true).expect("enter ok");
            assert_eq!(*events.borrow(), vec!["raw_set", "nonblock_set"]);
        }

        // Drop restores both the raw mode and the O_NONBLOCK flag it set.
        assert_eq!(
            *events.borrow(),
            vec![
                "raw_set",
                "nonblock_set",
                "raw_restored",
                "nonblock_cleared"
            ],
        );
    }

    #[test]
    fn forwarded_signal_mask_restores_on_drop() {
        use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};

        let read_mask = || {
            let mut current = SigSet::empty();
            pthread_sigmask(SigmaskHow::SIG_BLOCK, None, Some(&mut current)).unwrap();
            current
        };
        let signals = [
            Signal::SIGWINCH,
            Signal::SIGINT,
            Signal::SIGTERM,
            Signal::SIGTSTP,
            Signal::SIGHUP,
            Signal::SIGQUIT,
        ];
        let before = read_mask();
        {
            let _guard = block_forwarded_signals().unwrap();
            let blocked = read_mask();
            assert!(signals.iter().all(|signal| blocked.contains(*signal)));
        }
        let after = read_mask();
        for signal in signals {
            assert_eq!(before.contains(signal), after.contains(signal));
        }
    }

    // ---- (d) exit-code table ---------------------------------------------

    #[test]
    fn exit_for_kind_covers_every_wire_slug() {
        use ExecFailureSource::*;
        let cases = [
            (
                "guest-control-transport-unavailable",
                EXIT_EXEC_TRANSPORT,
                Transport,
            ),
            ("guest-control-timeout", EXIT_EXEC_TRANSPORT, Transport),
            (
                "guest-control-unavailable-old-generation",
                EXIT_EXEC_OLD_GENERATION,
                GuestControl,
            ),
            (
                "guest-control-capability-unavailable",
                EXIT_EXEC_OLD_GENERATION,
                GuestControl,
            ),
            ("exec-session-capacity", EXIT_EXEC_CAPACITY, GuestControl),
            (
                "exec-session-rate-limited",
                EXIT_EXEC_CAPACITY,
                GuestControl,
            ),
            ("guest-control-protocol-error", EXIT_EXEC_PROTOCOL, Protocol),
            ("guest-control-exec-error", EXIT_EXEC_PROTOCOL, Protocol),
            ("guest-control-auth-failed", EXIT_EXEC_AUTH, GuestControl),
            ("authz-not-admin", EXIT_EXEC_AUTH, GuestControl),
            ("guest-control-exec-internal", EXIT_EXEC_INTERNAL, Internal),
            ("totally-unknown-slug", EXIT_EXEC_INTERNAL, Internal),
        ];
        for (slug, code, source) in cases {
            assert_eq!(exit_for_kind(slug), (code, source), "slug {slug}");
        }
    }
    // ---- (h) redaction: no stdio / argv bytes in error surfaces -----------

    #[test]
    fn error_constructors_carry_only_static_remediation_text() {
        // None of the typed constructors embed caller-supplied stdio/argv.
        for err in [
            ExecClientError::transport("ctx"),
            ExecClientError::protocol("ctx"),
            ExecClientError::internal("ctx"),
        ] {
            assert!(!err.kind.is_empty());
            assert!(!err.remediation.is_empty());
        }
    }
}
