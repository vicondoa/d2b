//! CLI-side `nixling vm exec` owner-connection FSM + host terminal safety.
//!
//! `nixling vm exec` routes one owner connection over the daemon `public.sock`
//! `exec` verb: a single `Start` op establishes the daemon-held authenticated
//! guest-control session, then the remaining ops
//! (`WriteStdin`/`ReadOutput`/`Signal`/`Resize`/`Wait`/`Close`) drive it. The
//! CLI never opens a new connection per op and never allocates a host PTY —
//! the guest owns the PTY (W14 helper-exec). This module is the pure FSM +
//! host-termios safety; the real socket/signal/host wiring lives in `lib.rs`
//! so the FSM is unit-testable against injected fakes.

use std::collections::VecDeque;
use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};

use nixling_core::base64_codec;
use nixling_ipc::public_wire::{
    ExecCloseArgs, ExecOp, ExecOpResponse, ExecReadOutputArgs, ExecReadOutputResult,
    ExecResizeArgs, ExecSignalArgs, ExecStartResult, ExecStream, ExecTerminalStatus, ExecWaitArgs,
    ExecWaitResult, ExecWriteStdinArgs, ExecWriteStdinResult,
};
use serde_json::Value;

// Reserved exec CLI exit codes (WR9). Guest WIFEXITED 0-255 codes pass through
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

/// Maximum decoded stdin/stdout chunk the CLI moves per op. Mirrors the daemon
/// `EXEC_MAX_CHUNK_BYTES` cap so a single op never approaches the frame cap.
pub const EXEC_CLI_CHUNK_BYTES: u64 = nixling_ipc::public_wire::EXEC_MAX_CHUNK_BYTES;

/// Where a terminal failure originated. Surfaced in the `--json` envelope so a
/// consumer can disambiguate a guest exit code from a transport exit code that
/// happens to share a shell status number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecFailureSource {
    /// vsock connect / handshake transport, or a deadline.
    Transport,
    /// The guest authenticated but the VM/guest rejected the request
    /// (old-generation, capability, capacity, rate-limit, auth).
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
            "confirm the VM is running and guest-control-health is ready (`nixling vm status <vm>`), then retry",
        )
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(
            "guest-control-protocol-error",
            EXIT_EXEC_PROTOCOL,
            ExecFailureSource::Protocol,
            message,
            "the guest-control protocol is skewed; rebuild the guest with a matching nixling generation",
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

/// The CLI exit code + failure source for a daemon wire `kind` slug (WR9).
pub fn exit_for_kind(kind: &str) -> (i32, ExecFailureSource) {
    match kind {
        "guest-control-transport-unavailable" | "guest-control-timeout" => {
            (EXIT_EXEC_TRANSPORT, ExecFailureSource::Transport)
        }
        "guest-control-unavailable-old-generation" | "guest-control-capability-unavailable" => {
            (EXIT_EXEC_OLD_GENERATION, ExecFailureSource::GuestControl)
        }
        "exec-session-capacity" | "exec-session-rate-limited" => {
            (EXIT_EXEC_CAPACITY, ExecFailureSource::GuestControl)
        }
        "guest-control-protocol-error" | "guest-control-exec-error" => {
            (EXIT_EXEC_PROTOCOL, ExecFailureSource::Protocol)
        }
        "guest-control-auth-failed" => (EXIT_EXEC_AUTH, ExecFailureSource::GuestControl),
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

/// Guest signal number for a forwarded host signal (WR11 mapping).
pub fn guest_signo(signal: ExecSignal) -> u32 {
    match signal {
        ExecSignal::Interrupt => 2,
        ExecSignal::Quit => 3,
        ExecSignal::Hangup => 1,
        ExecSignal::Terminate => 15,
        ExecSignal::Stop => 20,
        // Winch is handled as a Resize, never as a Signal op.
        ExecSignal::Winch => 28,
    }
}

/// Transport seam: one owner-connection round trip (one op, one response).
pub trait ExecOwnerTransport {
    fn round_trip(&mut self, op: &ExecOp) -> Result<ExecOpResponse, ExecClientError>;
}

/// Host I/O seam: non-blocking stdin, blocking stdout/stderr, window size.
pub trait ExecHostIo {
    /// Read available stdin bytes. MUST be non-blocking: return an
    /// `io::ErrorKind::WouldBlock` error when no data is ready, `Ok(0)` on EOF.
    fn read_stdin(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write_stdout(&mut self, data: &[u8]) -> io::Result<()>;
    fn write_stderr(&mut self, data: &[u8]) -> io::Result<()>;
    /// Current terminal window size as `(rows, cols)`, if a terminal.
    fn window_size(&self) -> Option<(u32, u32)>;
}

/// Signal seam: drain the events enqueued since the last poll.
pub trait ExecSignalSource {
    fn drain(&mut self) -> Vec<ExecSignal>;
}

/// FSM configuration resolved from the parsed CLI args + the `Start` response.
#[derive(Debug, Clone, Copy)]
pub struct ExecFsmConfig {
    /// The guest allocated a PTY (`-t`): output is a single merged stdout
    /// stream and the host terminal is in raw mode.
    pub tty: bool,
    /// Forward host stdin into the guest (`-i`).
    pub interactive: bool,
    /// Bounded long-poll timeout (ms) for `ReadOutput`/`Wait` so stdin and
    /// signals are serviced promptly between polls (WR11 no-starve).
    pub poll_timeout_ms: u64,
    /// Maximum decoded bytes per stdin/output chunk.
    pub max_chunk: u64,
}

impl Default for ExecFsmConfig {
    fn default() -> Self {
        Self {
            tty: false,
            interactive: false,
            poll_timeout_ms: 50,
            max_chunk: EXEC_CLI_CHUNK_BYTES,
        }
    }
}

/// The terminal disposition the FSM resolved for the guest command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    pub terminal: ExecTerminalStatus,
}

/// The CLI process exit code for a resolved terminal status (WR9):
/// WIFEXITED → code (0-255), WIFSIGNALED → 128+signo, abnormal → reserved.
pub fn exit_code_for_terminal(status: &ExecTerminalStatus) -> i32 {
    match status {
        ExecTerminalStatus::Exited { code } => (*code).clamp(0, 255),
        ExecTerminalStatus::Signaled { signal } => 128 + (*signal as i32),
        ExecTerminalStatus::Error { slug } => match slug.as_str() {
            "lost-guestd" => EXIT_EXEC_TRANSPORT,
            "cancelled" | "reaped" | "slow-consumer-cancelled" => EXIT_EXEC_CAPACITY,
            _ => EXIT_EXEC_PROTOCOL,
        },
    }
}

fn expect_write(resp: ExecOpResponse) -> Result<ExecWriteStdinResult, ExecClientError> {
    match resp {
        ExecOpResponse::WriteStdin(result) => Ok(result),
        other => Err(ExecClientError::protocol(format!(
            "expected a WriteStdin response, got {}",
            response_label(&other)
        ))),
    }
}

fn expect_read(resp: ExecOpResponse) -> Result<ExecReadOutputResult, ExecClientError> {
    match resp {
        ExecOpResponse::ReadOutput(result) => Ok(result),
        other => Err(ExecClientError::protocol(format!(
            "expected a ReadOutput response, got {}",
            response_label(&other)
        ))),
    }
}

fn expect_wait(resp: ExecOpResponse) -> Result<ExecWaitResult, ExecClientError> {
    match resp {
        ExecOpResponse::Wait(result) => Ok(result),
        other => Err(ExecClientError::protocol(format!(
            "expected a Wait response, got {}",
            response_label(&other)
        ))),
    }
}

fn response_label(resp: &ExecOpResponse) -> &'static str {
    match resp {
        ExecOpResponse::Start(_) => "start",
        ExecOpResponse::WriteStdin(_) => "writeStdin",
        ExecOpResponse::ReadOutput(_) => "readOutput",
        ExecOpResponse::Signal(_) => "signal",
        ExecOpResponse::Resize(_) => "resize",
        ExecOpResponse::Wait(_) => "wait",
        ExecOpResponse::Close(_) => "close",
    }
}

fn close_op(session: &str) -> ExecOp {
    ExecOp::Close(ExecCloseArgs {
        session: session.to_owned(),
    })
}

/// Drive a single established exec session to terminal completion. One session,
/// one exec, no per-op reconnect (WR11). Long-polls use a short bounded timeout
/// so stdin and signals are never starved behind a `ReadOutput`/`Wait` poll;
/// output is drained to EOF on every stream before the FSM returns.
pub fn run_exec_fsm<T, H, S>(
    transport: &mut T,
    host: &mut H,
    signals: &mut S,
    start: &ExecStartResult,
    config: &ExecFsmConfig,
) -> Result<ExecOutcome, ExecClientError>
where
    T: ExecOwnerTransport,
    H: ExecHostIo,
    S: ExecSignalSource,
{
    let session = start.session.as_str();
    let mut stdin_offset: u64 = 0;
    let mut stdout_offset = start.stdout_offset;
    let mut stderr_offset = start.stderr_offset;
    let mut stdin_done = !config.interactive;
    let mut stdin_closed = false;
    // In tty mode the guest PTY merges stderr into stdout; the stderr stream is
    // never read, so treat it as drained from the start.
    let mut stdout_eof = false;
    let mut stderr_eof = config.tty;
    let chunk = config.max_chunk.max(1) as usize;
    let mut buf = vec![0_u8; chunk];

    // Non-interactive: close the guest stdin up front so a command reading
    // stdin observes EOF immediately (idempotent on the daemon side).
    if !config.interactive {
        let _ = transport.round_trip(&close_op(session))?;
        stdin_closed = true;
    }

    loop {
        // 1. Forward enqueued host signals (Resize for SIGWINCH in tty mode,
        //    Signal for the rest). Signals are drained, never handled in-band.
        for signal in signals.drain() {
            match signal {
                ExecSignal::Winch => {
                    if config.tty {
                        if let Some((rows, cols)) = host.window_size() {
                            transport.round_trip(&ExecOp::Resize(ExecResizeArgs {
                                session: session.to_owned(),
                                rows,
                                cols,
                            }))?;
                        }
                    }
                }
                other => {
                    transport.round_trip(&ExecOp::Signal(ExecSignalArgs {
                        session: session.to_owned(),
                        signo: guest_signo(other),
                    }))?;
                }
            }
        }

        // 2. Forward whatever host stdin is ready (non-blocking).
        if config.interactive && !stdin_done {
            match host.read_stdin(&mut buf) {
                Ok(0) => {
                    if !stdin_closed {
                        transport.round_trip(&close_op(session))?;
                        stdin_closed = true;
                    }
                    stdin_done = true;
                }
                Ok(read) => {
                    let mut sent = 0_usize;
                    while sent < read {
                        let end = (sent + chunk).min(read);
                        let resp = transport.round_trip(&ExecOp::WriteStdin(ExecWriteStdinArgs {
                            session: session.to_owned(),
                            offset: stdin_offset,
                            chunk_base64: base64_codec::encode(&buf[sent..end]),
                            eof: false,
                        }))?;
                        let written = expect_write(resp)?;
                        stdin_offset = written.next_offset;
                        let accepted = written.accepted_len as usize;
                        sent += accepted;
                        if written.stdin_closed {
                            stdin_done = true;
                            stdin_closed = true;
                            break;
                        }
                        // Zero-accepted (backpressure / full guest budget):
                        // stop pushing this batch and re-poll output so the
                        // guest drains, then retry the remainder next loop.
                        if accepted == 0 {
                            break;
                        }
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => {
                    return Err(ExecClientError::internal(format!(
                        "reading host stdin failed: {error}"
                    )));
                }
            }
        }

        // 3. Drain stdout (bounded long-poll: returns early on data).
        if !stdout_eof {
            let resp = transport.round_trip(&ExecOp::ReadOutput(ExecReadOutputArgs {
                session: session.to_owned(),
                stream: ExecStream::Stdout,
                offset: stdout_offset,
                max_len: config.max_chunk,
                wait: true,
                timeout_ms: config.poll_timeout_ms,
            }))?;
            let output = expect_read(resp)?;
            write_output(host, false, &output)?;
            stdout_offset = output.next_offset;
            if output.eof {
                stdout_eof = true;
            }
        }

        // 4. Drain stderr without blocking (tty merges into stdout, skip).
        if !config.tty && !stderr_eof {
            let resp = transport.round_trip(&ExecOp::ReadOutput(ExecReadOutputArgs {
                session: session.to_owned(),
                stream: ExecStream::Stderr,
                offset: stderr_offset,
                max_len: config.max_chunk,
                wait: false,
                timeout_ms: 0,
            }))?;
            let output = expect_read(resp)?;
            write_output(host, true, &output)?;
            stderr_offset = output.next_offset;
            if output.eof {
                stderr_eof = true;
            }
        }

        // 5. Both streams drained to EOF → the command is terminal. Fetch the
        //    disposition and return (output is fully flushed at this point).
        if stdout_eof && stderr_eof {
            let resp = transport.round_trip(&ExecOp::Wait(ExecWaitArgs {
                session: session.to_owned(),
                timeout_ms: config.poll_timeout_ms,
            }))?;
            let wait = expect_wait(resp)?;
            if let Some(status) = wait.terminal_status {
                return Ok(ExecOutcome { terminal: status });
            }
            if !wait.running {
                return Ok(ExecOutcome {
                    terminal: ExecTerminalStatus::Exited { code: 0 },
                });
            }
            // Still running but output EOF reported — keep polling Wait.
        }
    }
}

fn write_output<H: ExecHostIo>(
    host: &mut H,
    stderr: bool,
    output: &ExecReadOutputResult,
) -> Result<(), ExecClientError> {
    if output.data_base64.is_empty() {
        return Ok(());
    }
    let data = base64_codec::decode(&output.data_base64)
        .map_err(|_| ExecClientError::protocol("guest sent a malformed base64 output chunk"))?;
    let result = if stderr {
        host.write_stderr(&data)
    } else {
        host.write_stdout(&data)
    };
    result.map_err(|error| ExecClientError::internal(format!("writing host output failed: {error}")))
}

/// Encode an [`ExecOp`] as the `exec` daemon wire frame: the adjacently-tagged
/// `{ "op": …, "args": … }` body with a `type: "exec"` discriminator.
pub fn encode_exec_op_frame(op: &ExecOp) -> Result<Vec<u8>, ExecClientError> {
    let mut value = serde_json::to_value(op)
        .map_err(|error| ExecClientError::internal(format!("encoding exec op failed: {error}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| ExecClientError::internal("encoded exec op was not a JSON object"))?;
    object.insert("type".to_owned(), Value::String("exec".to_owned()));
    serde_json::to_vec(&value)
        .map_err(|error| ExecClientError::internal(format!("serializing exec op failed: {error}")))
}

/// Decode an `execResponse` (or `error`) daemon wire frame into an
/// [`ExecOpResponse`], mapping a daemon error frame to a typed
/// [`ExecClientError`] with the CLI exec exit-code contract.
pub fn decode_exec_response_frame(bytes: &[u8]) -> Result<ExecOpResponse, ExecClientError> {
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|error| ExecClientError::protocol(format!("malformed daemon reply: {error}")))?;
    let type_name = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ExecClientError::protocol("daemon reply was missing a type discriminator"))?
        .to_owned();
    match type_name.as_str() {
        "execResponse" => {
            if let Some(object) = value.as_object_mut() {
                object.remove("type");
            }
            serde_json::from_value(value).map_err(|error| {
                ExecClientError::protocol(format!("malformed execResponse body: {error}"))
            })
        }
        "error" => Err(decode_error_frame(&value)),
        other => Err(ExecClientError::protocol(format!(
            "daemon returned an unexpected reply type '{other}'"
        ))),
    }
}

fn decode_error_frame(value: &Value) -> ExecClientError {
    let error = value.get("error").unwrap_or(value);
    let kind = error
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("guest-control-exec-internal");
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("the daemon rejected the exec operation")
        .to_owned();
    let remediation = error
        .get("remediation")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    ExecClientError::from_daemon_error(kind, message, remediation)
}

// ---------------------------------------------------------------------------
// Host terminal safety (WR11): a guard that restores termios + O_NONBLOCK on
// EVERY exit/error/disconnect/panic via Drop. Uses only the safe rustix
// termios/fcntl wrappers — no `unsafe`.
// ---------------------------------------------------------------------------

/// RAII guard over host stdin terminal state. On `enter` it optionally puts the
/// terminal in raw mode and/or marks stdin non-blocking; on drop it restores
/// the saved termios and clears any O_NONBLOCK it set. Restoration is
/// idempotent and never panics.
pub struct FdStateGuard {
    original_termios: Option<rustix::termios::Termios>,
    nonblock_added: bool,
    restored: bool,
}

impl FdStateGuard {
    /// Enter the requested host stdin state. `raw` puts the terminal in raw
    /// mode (the guest owns echo/line discipline via its PTY); `nonblock`
    /// marks stdin O_NONBLOCK so the FSM can poll it without blocking.
    pub fn enter(raw: bool, nonblock: bool) -> io::Result<Self> {
        let fd = rustix::stdio::stdin();
        let original_termios = if raw {
            let original = rustix::termios::tcgetattr(fd).map_err(errno_to_io)?;
            let mut raw_termios = original.clone();
            raw_termios.make_raw();
            rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Flush, &raw_termios)
                .map_err(errno_to_io)?;
            Some(original)
        } else {
            None
        };

        let mut nonblock_added = false;
        if nonblock {
            let flags = rustix::fs::fcntl_getfl(fd).map_err(errno_to_io)?;
            if !flags.contains(rustix::fs::OFlags::NONBLOCK) {
                rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)
                    .map_err(errno_to_io)?;
                nonblock_added = true;
            }
        }

        Ok(Self {
            original_termios,
            nonblock_added,
            restored: false,
        })
    }

    /// Restore the saved terminal state. Safe to call more than once.
    pub fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        let fd = rustix::stdio::stdin();
        if let Some(original) = &self.original_termios {
            let _ = rustix::termios::tcsetattr(
                fd,
                rustix::termios::OptionalActions::Flush,
                original,
            );
        }
        if self.nonblock_added {
            if let Ok(flags) = rustix::fs::fcntl_getfl(fd) {
                let _ = rustix::fs::fcntl_setfl(fd, flags & !rustix::fs::OFlags::NONBLOCK);
            }
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

impl ExecHostIo for RealHostIo {
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

impl ExecHostIo for CapturingHostIo {
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
// Signal source (WR11): block the forwarded signals process-wide and let a
// dedicated thread sigwait + enqueue. Enqueue-only — no termios/syscalls in a
// handler, and no `unsafe` (nix `SigSet` wrappers are safe).
// ---------------------------------------------------------------------------

/// Installed signal source backed by a sigwait thread. `drain` returns the
/// events enqueued since the last poll.
pub struct InstalledSignals {
    pending: Arc<Mutex<VecDeque<ExecSignal>>>,
}

impl ExecSignalSource for InstalledSignals {
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
pub fn install_signals() -> io::Result<InstalledSignals> {
    use nix::sys::signal::{SigSet, Signal};

    let mut set = SigSet::empty();
    for signal in [
        Signal::SIGWINCH,
        Signal::SIGINT,
        Signal::SIGTERM,
        Signal::SIGTSTP,
        Signal::SIGHUP,
        Signal::SIGQUIT,
    ] {
        set.add(signal);
    }
    set.thread_block().map_err(nix_errno_to_io)?;

    let pending = Arc::new(Mutex::new(VecDeque::new()));
    let pending_thread = Arc::clone(&pending);
    let wait_set = set;
    std::thread::Builder::new()
        .name("nixling-exec-sig".to_owned())
        .spawn(move || loop {
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
        })?;

    Ok(InstalledSignals { pending })
}

fn nix_errno_to_io(errno: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(errno as i32)
}
