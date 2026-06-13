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
    // Unsent host stdin carried across iterations: a zero-accepted
    // (backpressured) write keeps the remainder here and retries it before
    // reading fresh host stdin, so interactive input is never dropped (F3).
    let mut pending_stdin: Vec<u8> = Vec::new();
    // Stable client-assigned idempotency token for control ops (Signal/Resize).
    // A single monotonic counter keeps every control op id unique within the
    // session so the daemon's replay cache can dedup a re-delivered op (F3b).
    let mut next_control_op_id: u64 = 1;

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
                            let op_id = next_control_op_id;
                            next_control_op_id += 1;
                            transport.round_trip(&ExecOp::Resize(ExecResizeArgs {
                                session: session.to_owned(),
                                rows,
                                cols,
                                op_id,
                            }))?;
                        }
                    }
                }
                other => {
                    let op_id = next_control_op_id;
                    next_control_op_id += 1;
                    transport.round_trip(&ExecOp::Signal(ExecSignalArgs {
                        session: session.to_owned(),
                        signo: guest_signo(other),
                        op_id,
                    }))?;
                }
            }
        }

        // 2. Forward host stdin. Drain any pending (unsent) bytes from a prior
        //    backpressured write FIRST, then read fresh host stdin only when the
        //    pending buffer is empty — so a zero-accepted write never drops the
        //    already-read remainder (F3).
        if config.interactive && !stdin_done {
            if pending_stdin.is_empty() {
                match host.read_stdin(&mut buf) {
                    Ok(0) => {
                        if !stdin_closed {
                            transport.round_trip(&close_op(session))?;
                            stdin_closed = true;
                        }
                        stdin_done = true;
                    }
                    Ok(read) => {
                        pending_stdin.extend_from_slice(&buf[..read]);
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

            // Push as much of the pending buffer as the guest accepts this turn.
            // A zero-accepted (backpressure) write stops the push and KEEPS the
            // unsent remainder in `pending_stdin` for the next iteration, after
            // the output drain lets the guest budget recover.
            let mut sent = 0_usize;
            while sent < pending_stdin.len() {
                let end = (sent + chunk).min(pending_stdin.len());
                let resp = transport.round_trip(&ExecOp::WriteStdin(ExecWriteStdinArgs {
                    session: session.to_owned(),
                    offset: stdin_offset,
                    chunk_base64: base64_codec::encode(&pending_stdin[sent..end]),
                    eof: false,
                }))?;
                let written = expect_write(resp)?;
                stdin_offset = written.next_offset;
                let accepted = written.accepted_len as usize;
                sent += accepted;
                if written.stdin_closed {
                    stdin_done = true;
                    stdin_closed = true;
                    pending_stdin.clear();
                    sent = 0;
                    break;
                }
                if accepted == 0 {
                    break;
                }
            }
            if sent > 0 {
                pending_stdin.drain(..sent);
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
/// `{ "op": …, "args": … }` body with a `type: "exec"` discriminator and an
/// envelope-level `opId` correlation id (F1/WR6). The daemon echoes `opId` on
/// the matching response so a pending long-poll and an urgent control reply can
/// be matched out of order.
pub fn encode_exec_op_frame(op: &ExecOp, op_id: u64) -> Result<Vec<u8>, ExecClientError> {
    let mut value = serde_json::to_value(op)
        .map_err(|error| ExecClientError::internal(format!("encoding exec op failed: {error}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| ExecClientError::internal("encoded exec op was not a JSON object"))?;
    object.insert("type".to_owned(), Value::String("exec".to_owned()));
    object.insert("opId".to_owned(), Value::from(op_id));
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
                // `opId` is an envelope-level correlation id, not a field of the
                // adjacently-tagged response; strip it before deserializing.
                object.remove("opId");
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
            let _ = rustix::termios::tcsetattr(
                fd,
                rustix::termios::OptionalActions::Flush,
                original,
            );
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
    /// instead of leaving stdin stuck raw (F4).
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

// ===========================================================================
// Tests (WR16 matrices): (c) FSM one-session/no-reconnect, Wait-timeout keeps
// polling, drain-to-EOF, interactive closes stdin up front; (d) exit-code
// table + JSON-disambiguation primitives; (e) CLI-side backpressure / offset /
// stdout-stderr separation / tty merge; (g) signal mapping + FdStateGuard
// no-op/restore-idempotent; (h) redaction (no stdio/argv bytes in errors).
//
// The FSM is driven entirely through the `ExecOwnerTransport` / `ExecHostIo` /
// `ExecSignalSource` seams against in-memory fakes — no socket, no real tty.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use nixling_ipc::public_wire::ExecControlResult;
    use std::collections::VecDeque;

    /// A scripted owner transport: records every op, answers reads from
    /// in-memory stdout/stderr buffers, and drives `Wait` from a small running
    /// budget. Models a single established session (no reconnect seam exists).
    #[derive(Default)]
    struct FakeTransport {
        ops: Vec<ExecOp>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        /// Hold stdout at EOF=false (empty long-poll) until `Close` is seen.
        stdout_hold_until_close: bool,
        close_seen: bool,
        /// Force the next stdout `ReadOutput` to return this malformed base64
        /// (drives a protocol error for the redaction test).
        stdout_malformed: Option<String>,
        /// Number of `Wait` polls that report `running` before going terminal.
        wait_running_remaining: usize,
        terminal: Option<ExecTerminalStatus>,
        /// Scripted `WriteStdin` replies (accepted_len, backpressured,
        /// stdin_closed); when exhausted, accept the whole chunk.
        write_script: VecDeque<(u64, bool, bool)>,
    }

    impl FakeTransport {
        fn terminal(status: ExecTerminalStatus) -> Self {
            Self {
                terminal: Some(status),
                ..Self::default()
            }
        }

        fn count_ops(&self, label: &str) -> usize {
            self.ops.iter().filter(|op| op_label(op) == label).count()
        }

        fn signals(&self) -> Vec<u32> {
            self.ops
                .iter()
                .filter_map(|op| match op {
                    ExecOp::Signal(args) => Some(args.signo),
                    _ => None,
                })
                .collect()
        }

        fn resizes(&self) -> Vec<(u32, u32)> {
            self.ops
                .iter()
                .filter_map(|op| match op {
                    ExecOp::Resize(args) => Some((args.rows, args.cols)),
                    _ => None,
                })
                .collect()
        }
    }

    fn op_label(op: &ExecOp) -> &'static str {
        match op {
            ExecOp::Start(_) => "start",
            ExecOp::WriteStdin(_) => "writeStdin",
            ExecOp::ReadOutput(_) => "readOutput",
            ExecOp::Signal(_) => "signal",
            ExecOp::Resize(_) => "resize",
            ExecOp::Wait(_) => "wait",
            ExecOp::Close(_) => "close",
        }
    }

    impl ExecOwnerTransport for FakeTransport {
        fn round_trip(&mut self, op: &ExecOp) -> Result<ExecOpResponse, ExecClientError> {
            self.ops.push(op.clone());
            match op {
                // A real exec session never re-sends Start through the FSM.
                ExecOp::Start(_) => Err(ExecClientError::protocol("unexpected Start in FSM")),
                ExecOp::Close(_) => {
                    self.close_seen = true;
                    Ok(ExecOpResponse::Close(nixling_ipc::public_wire::ExecCloseResult {
                        stdin_closed: true,
                    }))
                }
                ExecOp::WriteStdin(args) => {
                    let chunk_len = base64_codec::decode(&args.chunk_base64)
                        .map(|bytes| bytes.len() as u64)
                        .unwrap_or(0);
                    let (accepted, backpressured, stdin_closed) = self
                        .write_script
                        .pop_front()
                        .unwrap_or((chunk_len, false, false));
                    Ok(ExecOpResponse::WriteStdin(ExecWriteStdinResult {
                        accepted_len: accepted,
                        next_offset: args.offset + accepted,
                        backpressured,
                        stdin_closed,
                    }))
                }
                ExecOp::ReadOutput(args) => {
                    let is_stdout = matches!(args.stream, ExecStream::Stdout);
                    if is_stdout {
                        if let Some(bad) = self.stdout_malformed.take() {
                            return Ok(ExecOpResponse::ReadOutput(ExecReadOutputResult {
                                data_base64: bad,
                                next_offset: args.offset,
                                eof: false,
                                dropped_bytes: 0,
                                truncated: false,
                                timed_out: false,
                            }));
                        }
                        if self.stdout_hold_until_close && !self.close_seen {
                            return Ok(ExecOpResponse::ReadOutput(ExecReadOutputResult {
                                data_base64: String::new(),
                                next_offset: args.offset,
                                eof: false,
                                dropped_bytes: 0,
                                truncated: false,
                                timed_out: true,
                            }));
                        }
                    }
                    let buf = if is_stdout { &self.stdout } else { &self.stderr };
                    let off = (args.offset as usize).min(buf.len());
                    let data = &buf[off..];
                    Ok(ExecOpResponse::ReadOutput(ExecReadOutputResult {
                        data_base64: base64_codec::encode(data),
                        next_offset: buf.len() as u64,
                        eof: true,
                        dropped_bytes: 0,
                        truncated: false,
                        timed_out: false,
                    }))
                }
                ExecOp::Signal(_) => Ok(ExecOpResponse::Signal(ExecControlResult { delivered: true })),
                ExecOp::Resize(_) => Ok(ExecOpResponse::Resize(ExecControlResult { delivered: true })),
                ExecOp::Wait(_) => {
                    if self.wait_running_remaining > 0 {
                        self.wait_running_remaining -= 1;
                        Ok(ExecOpResponse::Wait(ExecWaitResult {
                            running: true,
                            terminal_status: None,
                        }))
                    } else {
                        Ok(ExecOpResponse::Wait(ExecWaitResult {
                            running: false,
                            terminal_status: self.terminal.clone(),
                        }))
                    }
                }
            }
        }
    }

    #[derive(Default)]
    struct FakeHostIo {
        stdin: VecDeque<Vec<u8>>,
        eof_sent: bool,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        window: Option<(u32, u32)>,
    }

    impl ExecHostIo for FakeHostIo {
        fn read_stdin(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if let Some(chunk) = self.stdin.pop_front() {
                let n = chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                Ok(n)
            } else if !self.eof_sent {
                self.eof_sent = true;
                Ok(0)
            } else {
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            }
        }
        fn write_stdout(&mut self, data: &[u8]) -> io::Result<()> {
            self.stdout.extend_from_slice(data);
            Ok(())
        }
        fn write_stderr(&mut self, data: &[u8]) -> io::Result<()> {
            self.stderr.extend_from_slice(data);
            Ok(())
        }
        fn window_size(&self) -> Option<(u32, u32)> {
            self.window
        }
    }

    #[derive(Default)]
    struct FakeSignals {
        pending: VecDeque<Vec<ExecSignal>>,
    }

    impl FakeSignals {
        fn once(signals: Vec<ExecSignal>) -> Self {
            let mut pending = VecDeque::new();
            pending.push_back(signals);
            Self { pending }
        }
    }

    impl ExecSignalSource for FakeSignals {
        fn drain(&mut self) -> Vec<ExecSignal> {
            self.pending.pop_front().unwrap_or_default()
        }
    }

    fn start_result() -> ExecStartResult {
        ExecStartResult {
            session: "h-abc123".to_owned(),
            tty: false,
            stdout_offset: 0,
            stderr_offset: 0,
        }
    }

    fn cfg(tty: bool, interactive: bool) -> ExecFsmConfig {
        ExecFsmConfig {
            tty,
            interactive,
            poll_timeout_ms: 5,
            max_chunk: 64,
        }
    }

    /// Assert every op the FSM emitted references the single Start session
    /// handle (one session, no per-op reconnect / handle drift).
    fn assert_single_session(transport: &FakeTransport, session: &str) {
        for op in &transport.ops {
            let handle = match op {
                ExecOp::WriteStdin(a) => &a.session,
                ExecOp::ReadOutput(a) => &a.session,
                ExecOp::Signal(a) => &a.session,
                ExecOp::Resize(a) => &a.session,
                ExecOp::Wait(a) => &a.session,
                ExecOp::Close(a) => &a.session,
                ExecOp::Start(_) => panic!("FSM must never emit Start"),
            };
            assert_eq!(handle, session, "op {} drifted the session handle", op_label(op));
        }
    }

    // ---- (c) FSM lifecycle ------------------------------------------------

    #[test]
    fn non_interactive_closes_stdin_up_front_and_drains_to_terminal() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        transport.stdout = b"output-bytes".to_vec();
        let mut host = FakeHostIo::default();
        let mut signals = FakeSignals::default();
        let start = start_result();
        let outcome =
            run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, false))
                .expect("fsm");
        assert_eq!(outcome.terminal, ExecTerminalStatus::Exited { code: 0 });
        // Exactly one up-front Close, and the output was drained before exit.
        assert_eq!(transport.count_ops("close"), 1);
        assert_eq!(host.stdout, b"output-bytes");
        assert_single_session(&transport, &start.session);
    }

    #[test]
    fn interactive_closes_stdin_once_on_host_eof() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        transport.stdout = b"done".to_vec();
        transport.stdout_hold_until_close = true;
        let mut host = FakeHostIo::default();
        host.stdin.push_back(b"hi".to_vec());
        let mut signals = FakeSignals::default();
        let start = start_result();
        let outcome =
            run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, true))
                .expect("fsm");
        assert_eq!(outcome.terminal, ExecTerminalStatus::Exited { code: 0 });
        // Interactive: NO up-front close; exactly one Close on stdin EOF.
        assert_eq!(transport.count_ops("close"), 1);
        assert_eq!(transport.count_ops("writeStdin"), 1);
        assert_eq!(host.stdout, b"done");
    }

    #[test]
    fn wait_timeout_keeps_polling_until_terminal() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 7 });
        transport.wait_running_remaining = 3;
        let mut host = FakeHostIo::default();
        let mut signals = FakeSignals::default();
        let start = start_result();
        let outcome =
            run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, false))
                .expect("fsm");
        assert_eq!(outcome.terminal, ExecTerminalStatus::Exited { code: 7 });
        // 3 running polls + 1 terminal poll.
        assert_eq!(transport.count_ops("wait"), 4);
    }

    // ---- (e) backpressure / offset / stream separation --------------------

    #[test]
    fn partial_write_advances_offset_and_retries_remainder() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        // First WriteStdin accepts 3 of 5; second accepts the remaining 2.
        transport.write_script.push_back((3, false, false));
        transport.write_script.push_back((2, false, false));
        transport.stdout_hold_until_close = true;
        let mut host = FakeHostIo::default();
        host.stdin.push_back(b"hello".to_vec());
        let mut signals = FakeSignals::default();
        let start = start_result();
        run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, true))
            .expect("fsm");
        // Two WriteStdin ops; offsets advance 0 -> 3 -> 5.
        let offsets: Vec<u64> = transport
            .ops
            .iter()
            .filter_map(|op| match op {
                ExecOp::WriteStdin(a) => Some(a.offset),
                _ => None,
            })
            .collect();
        assert_eq!(offsets, vec![0, 3]);
        assert_eq!(transport.count_ops("writeStdin"), 2);
    }

    #[test]
    fn zero_accepted_write_surfaces_backpressure_without_error() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        transport.write_script.push_back((0, true, false));
        let mut host = FakeHostIo::default();
        host.stdin.push_back(b"hi".to_vec());
        let mut signals = FakeSignals::default();
        let start = start_result();
        // stdout drains immediately, so the FSM terminates cleanly even though
        // the guest stdin budget was full (zero accepted).
        let outcome =
            run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, true))
                .expect("fsm survives backpressure");
        assert_eq!(outcome.terminal, ExecTerminalStatus::Exited { code: 0 });
        assert_eq!(transport.count_ops("writeStdin"), 1);
    }

    #[test]
    fn backpressured_stdin_is_retried_not_dropped() {
        // A zero-accepted write must NOT lose the already-read host stdin: the
        // CLI keeps it pending and retries the SAME bytes at the SAME offset
        // before reading fresh host stdin (F3).
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        transport.stdout = b"ok".to_vec();
        transport.stdout_hold_until_close = true;
        // First write is fully backpressured (0 accepted); second accepts all 5.
        transport.write_script.push_back((0, true, false));
        transport.write_script.push_back((5, false, false));
        let mut host = FakeHostIo::default();
        host.stdin.push_back(b"hello".to_vec());
        let mut signals = FakeSignals::default();
        let start = start_result();
        run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, true))
            .expect("fsm");

        let writes: Vec<(u64, Vec<u8>)> = transport
            .ops
            .iter()
            .filter_map(|op| match op {
                ExecOp::WriteStdin(a) => {
                    Some((a.offset, base64_codec::decode(&a.chunk_base64).unwrap()))
                }
                _ => None,
            })
            .collect();
        // Exactly two writes; both carry the SAME "hello" bytes at offset 0 —
        // the backpressured remainder was retried, not replaced by fresh stdin.
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0], (0, b"hello".to_vec()));
        assert_eq!(writes[1], (0, b"hello".to_vec()));
    }

    #[test]
    fn stdout_and_stderr_are_written_to_separate_host_streams() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        transport.stdout = b"to-stdout".to_vec();
        transport.stderr = b"to-stderr".to_vec();
        let mut host = FakeHostIo::default();
        let mut signals = FakeSignals::default();
        let start = start_result();
        run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, false))
            .expect("fsm");
        assert_eq!(host.stdout, b"to-stdout");
        assert_eq!(host.stderr, b"to-stderr");
    }

    #[test]
    fn tty_mode_never_reads_stderr_stream() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        transport.stdout = b"merged".to_vec();
        transport.stderr = b"should-not-be-read".to_vec();
        let mut host = FakeHostIo::default();
        let mut signals = FakeSignals::default();
        let start = start_result();
        run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(true, true))
            .expect("fsm");
        // In tty mode the guest PTY merges stderr; the CLI only reads stdout.
        let stderr_reads = transport
            .ops
            .iter()
            .filter(|op| matches!(op, ExecOp::ReadOutput(a) if matches!(a.stream, ExecStream::Stderr)))
            .count();
        assert_eq!(stderr_reads, 0);
        assert_eq!(host.stdout, b"merged");
        assert!(host.stderr.is_empty());
    }

    // ---- (g) signal mapping ----------------------------------------------

    #[test]
    fn sigwinch_forwards_a_resize_in_tty_mode_only() {
        // tty: Winch -> Resize with the host window size.
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        let mut host = FakeHostIo {
            window: Some((40, 120)),
            ..FakeHostIo::default()
        };
        let mut signals = FakeSignals::once(vec![ExecSignal::Winch]);
        let start = start_result();
        run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(true, true))
            .expect("fsm");
        assert_eq!(transport.resizes(), vec![(40, 120)]);

        // non-tty: Winch is dropped (no host PTY to resize).
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        let mut host = FakeHostIo {
            window: Some((40, 120)),
            ..FakeHostIo::default()
        };
        let mut signals = FakeSignals::once(vec![ExecSignal::Winch]);
        run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, false))
            .expect("fsm");
        assert!(transport.resizes().is_empty());
    }

    #[test]
    fn host_signals_map_to_guest_signal_numbers() {
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        let mut host = FakeHostIo::default();
        let mut signals = FakeSignals::once(vec![
            ExecSignal::Interrupt,
            ExecSignal::Terminate,
            ExecSignal::Stop,
            ExecSignal::Hangup,
            ExecSignal::Quit,
        ]);
        let start = start_result();
        run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, false))
            .expect("fsm");
        assert_eq!(transport.signals(), vec![2, 15, 20, 1, 3]);
    }

    #[test]
    fn guest_signo_mapping_is_stable() {
        assert_eq!(guest_signo(ExecSignal::Interrupt), 2);
        assert_eq!(guest_signo(ExecSignal::Quit), 3);
        assert_eq!(guest_signo(ExecSignal::Hangup), 1);
        assert_eq!(guest_signo(ExecSignal::Terminate), 15);
        assert_eq!(guest_signo(ExecSignal::Stop), 20);
        assert_eq!(guest_signo(ExecSignal::Winch), 28);
    }

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
        // be restored out of raw mode before `enter` returns Err (F4).
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
            vec!["raw_set", "nonblock_set", "raw_restored", "nonblock_cleared"],
        );
    }

    // ---- (d) exit-code table ---------------------------------------------

    #[test]
    fn exit_code_for_terminal_passes_through_wifexited() {
        for code in [0, 1, 70, 125, 126, 127, 255] {
            assert_eq!(
                exit_code_for_terminal(&ExecTerminalStatus::Exited { code }),
                code
            );
        }
        // Out-of-range guest codes are clamped into 0..=255.
        assert_eq!(
            exit_code_for_terminal(&ExecTerminalStatus::Exited { code: 300 }),
            255
        );
        assert_eq!(
            exit_code_for_terminal(&ExecTerminalStatus::Exited { code: -1 }),
            0
        );
    }

    #[test]
    fn exit_code_for_terminal_maps_signals_to_128_plus_signo() {
        for (signo, expected) in [(1u32, 129), (2, 130), (9, 137), (15, 143)] {
            assert_eq!(
                exit_code_for_terminal(&ExecTerminalStatus::Signaled { signal: signo }),
                expected
            );
        }
    }

    #[test]
    fn exit_code_for_terminal_maps_abnormal_slugs() {
        assert_eq!(
            exit_code_for_terminal(&ExecTerminalStatus::Error {
                slug: "lost-guestd".to_owned()
            }),
            EXIT_EXEC_TRANSPORT
        );
        for slug in ["cancelled", "reaped", "slow-consumer-cancelled"] {
            assert_eq!(
                exit_code_for_terminal(&ExecTerminalStatus::Error {
                    slug: slug.to_owned()
                }),
                EXIT_EXEC_CAPACITY
            );
        }
        assert_eq!(
            exit_code_for_terminal(&ExecTerminalStatus::Error {
                slug: "anything-else".to_owned()
            }),
            EXIT_EXEC_PROTOCOL
        );
    }

    #[test]
    fn exit_for_kind_covers_every_wire_slug() {
        use ExecFailureSource::*;
        let cases = [
            ("guest-control-transport-unavailable", EXIT_EXEC_TRANSPORT, Transport),
            ("guest-control-timeout", EXIT_EXEC_TRANSPORT, Transport),
            ("guest-control-unavailable-old-generation", EXIT_EXEC_OLD_GENERATION, GuestControl),
            ("guest-control-capability-unavailable", EXIT_EXEC_OLD_GENERATION, GuestControl),
            ("exec-session-capacity", EXIT_EXEC_CAPACITY, GuestControl),
            ("exec-session-rate-limited", EXIT_EXEC_CAPACITY, GuestControl),
            ("guest-control-protocol-error", EXIT_EXEC_PROTOCOL, Protocol),
            ("guest-control-exec-error", EXIT_EXEC_PROTOCOL, Protocol),
            ("guest-control-auth-failed", EXIT_EXEC_AUTH, GuestControl),
            ("guest-control-exec-internal", EXIT_EXEC_INTERNAL, Internal),
            ("totally-unknown-slug", EXIT_EXEC_INTERNAL, Internal),
        ];
        for (slug, code, source) in cases {
            assert_eq!(exit_for_kind(slug), (code, source), "slug {slug}");
        }
    }

    #[test]
    fn old_generation_and_guest_exit_70_are_distinguishable() {
        // 70-vs-70: a guest command that exits 70 yields a *terminal* (source
        // = guest, not an error); an old-generation guest yields an *error*
        // with the same numeric exit. The two are disambiguated by source.
        let guest_70 = exit_code_for_terminal(&ExecTerminalStatus::Exited { code: 70 });
        let (old_gen_70, old_gen_source) = exit_for_kind("guest-control-unavailable-old-generation");
        assert_eq!(guest_70, 70);
        assert_eq!(old_gen_70, 70);
        assert_eq!(old_gen_source, ExecFailureSource::GuestControl);
    }

    // ---- (h) redaction: no stdio / argv bytes in error surfaces -----------

    #[test]
    fn malformed_output_error_never_echoes_the_guest_bytes() {
        const SENTINEL: &str = "NIXLING_SECRET_LEAK_CANARY";
        let mut transport = FakeTransport::terminal(ExecTerminalStatus::Exited { code: 0 });
        // The guest returns a malformed base64 chunk carrying the sentinel.
        transport.stdout_malformed = Some(format!("{SENTINEL}***not-base64"));
        let mut host = FakeHostIo::default();
        let mut signals = FakeSignals::default();
        let start = start_result();
        let err = run_exec_fsm(&mut transport, &mut host, &mut signals, &start, &cfg(false, false))
            .expect_err("malformed output is a protocol error");
        let rendered = format!("{err:?} {} {} {}", err.kind, err.message, err.remediation);
        assert!(
            !rendered.contains(SENTINEL),
            "error surface leaked guest bytes: {rendered}"
        );
        assert_eq!(err.exit_code, EXIT_EXEC_PROTOCOL);
    }

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
