use std::{
    ffi::OsString,
    fmt,
    fs::File,
    io::Read,
    os::fd::OwnedFd,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Mutex, OnceLock},
};

use command_fds::{CommandFdExt, FdMapping};
use nix::sys::signal::{SigSet, Signal};

use crate::provider::VerifiedExecutable;

#[cfg(unix)]
use rustix::pipe::{PipeFlags, pipe_with};

const PRIVATE_STATUS_FD: i32 = 8;
const PRIVATE_EXECUTABLE_FD: i32 = 9;
pub const SUPERVISOR_ENVIRONMENT: &str = "D2B_BAZEL_EXEC_SUPERVISOR";
const IMMUTABLE_SUPERVISOR_PATH: Option<&str> = option_env!("D2B_BAZEL_EXEC_SUPERVISOR");

pub const STATUS_BUFFER_CAPACITY: usize = 27;
pub const STATUS_MAGIC: [u8; 4] = *b"D2BS";
pub const STATUS_VERSION: u8 = 1;
pub const PTRACE_EVENT_EXEC: u32 = 4;

pub const RUST_PARENT_STAGE_CODES: &[&str] = &[
    "D2B-BZLEXEC-PARENT-PREPARE",
    "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF",
    "D2B-BZLEXEC-PARENT-SPAWN",
    "D2B-BZLEXEC-PARENT-HELPER-IDENTITY",
    "D2B-BZLEXEC-PARENT-CLOSE",
    "D2B-BZLEXEC-PARENT-READY",
    "D2B-BZLEXEC-PARENT-EXECUTED",
    "D2B-BZLEXEC-PARENT-TERMINAL",
    "D2B-BZLEXEC-PARENT-WAIT",
    "D2B-BZLEXEC-PARENT-PROTOCOL",
    "D2B-BZLEXEC-PARENT-TARGET",
    "D2B-BZLEXEC-PARENT-STATUS",
    "D2B-BZLEXEC-PARENT-CLEANUP",
];

pub const SUPERVISOR_STAGE_CODES: &[&str] = &[
    "D2B-BZLEXEC-HELPER-SIGNAL-INHERITED-IGNORED",
    "D2B-BZLEXEC-HELPER-SIGNAL-HANDOFF",
    "D2B-BZLEXEC-HELPER-ADOPT",
    "D2B-BZLEXEC-HELPER-SIGNAL-NORMALIZE",
    "D2B-BZLEXEC-HELPER-EXEC-PIPE",
    "D2B-BZLEXEC-HELPER-FORK",
    "D2B-BZLEXEC-HELPER-GROUP-ESRCH",
    "D2B-BZLEXEC-HELPER-GROUP-EPERM",
    "D2B-BZLEXEC-HELPER-GROUP-ERROR",
    "D2B-BZLEXEC-HELPER-GROUP-EARLY-EXIT",
    "D2B-BZLEXEC-HELPER-PTRACE-STOP",
    "D2B-BZLEXEC-HELPER-PTRACE-OPTIONS",
    "D2B-BZLEXEC-HELPER-PTRACE-CONT",
    "D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION",
    "D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH",
    "D2B-BZLEXEC-HELPER-PTRACE-EVENT",
    "D2B-BZLEXEC-HELPER-PTRACE-DETACH",
    "D2B-BZLEXEC-HELPER-EXEC-TIMEOUT",
    "D2B-BZLEXEC-HELPER-EXEC-PARTIAL",
    "D2B-BZLEXEC-HELPER-EXEC-OVERLONG",
    "D2B-BZLEXEC-HELPER-EXEC-UNKNOWN",
    "D2B-BZLEXEC-HELPER-EXEC-EPIPE",
    "D2B-BZLEXEC-HELPER-EXEC-IO",
    "D2B-BZLEXEC-HELPER-SIGNAL-FORWARD",
    "D2B-BZLEXEC-HELPER-DEADLINE",
    "D2B-BZLEXEC-HELPER-WAIT",
    "D2B-BZLEXEC-HELPER-REAP",
    "D2B-BZLEXEC-HELPER-TERMINAL-WRITE",
    "D2B-BZLEXEC-HELPER-STATUS-MIRROR",
    "D2B-BZLEXEC-HELPER-CLEANUP",
];

pub const CHILD_STAGE_CODES: &[&str] = &[
    "D2B-BZLEXEC-CHILD-GROUP",
    "D2B-BZLEXEC-CHILD-SIGNAL",
    "D2B-BZLEXEC-CHILD-STDIO",
    "D2B-BZLEXEC-CHILD-CLOEXEC",
    "D2B-BZLEXEC-CHILD-CLOSE",
    "D2B-BZLEXEC-CHILD-PTRACE",
    "D2B-BZLEXEC-CHILD-STOP",
    "D2B-BZLEXEC-CHILD-EXECVEAT",
];

/// The standard streams are deliberately inherited unchanged by the helper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdioPolicy {
    Inherit,
    Null,
}

/// The request fields available to the safe execution owner.
///
/// `target_argv[0]` is passed to the target as its argv0. The helper itself is
/// never used as the target argv0.
#[derive(Clone, Eq, PartialEq)]
pub struct ExecutionRequest {
    pub stdin: StdioPolicy,
    pub stdout: StdioPolicy,
    pub stderr: StdioPolicy,
    pub target_argv: Vec<OsString>,
}

impl fmt::Debug for ExecutionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExecutionRequest(..)")
    }
}

impl Default for ExecutionRequest {
    fn default() -> Self {
        Self {
            stdin: StdioPolicy::Inherit,
            stdout: StdioPolicy::Inherit,
            stderr: StdioPolicy::Inherit,
            target_argv: vec![OsString::from("target")],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionResult {
    pub helper_started: bool,
    pub terminal: TerminalStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupervisorIdentity {
    label: &'static str,
    immutable: bool,
}

impl SupervisorIdentity {
    const fn immutable() -> Self {
        Self {
            label: "d2b-bazel-exec-supervisor",
            immutable: true,
        }
    }

    #[cfg(feature = "test-support")]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[cfg(feature = "test-support")]
    pub const fn is_immutable(self) -> bool {
        self.immutable
    }
}

/// A launch plan is deliberately compiled into the explicit test-support
/// surface only. Production callers cannot implement a backend that receives
/// an executable descriptor or inspect the plan.
#[cfg(feature = "test-support")]
pub struct LaunchPlan {
    #[cfg(unix)]
    private_fd: OwnedFd,
    #[cfg(not(unix))]
    private_fd: (),
    request: ExecutionRequest,
    supervisor: SupervisorIdentity,
}

#[cfg(feature = "test-support")]
impl LaunchPlan {
    #[cfg(unix)]
    pub fn private_fd_number(&self) -> i32 {
        use std::os::fd::AsRawFd;

        self.private_fd.as_raw_fd()
    }

    pub fn request(&self) -> &ExecutionRequest {
        &self.request
    }

    pub const fn supervisor(&self) -> SupervisorIdentity {
        self.supervisor
    }

    pub fn preserves_standard_streams(&self) -> bool {
        self.request.stdin == StdioPolicy::Inherit
            && self.request.stdout == StdioPolicy::Inherit
            && self.request.stderr == StdioPolicy::Inherit
    }
}

struct InternalLaunchPlan {
    #[cfg(unix)]
    private_fd: OwnedFd,
    #[cfg(not(unix))]
    private_fd: (),
    request: ExecutionRequest,
    supervisor: SupervisorIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    Capture,
    Block,
    Restore,
    Spawn,
    Mapping,
    HelperIdentity,
    StatusPipe,
    TargetArguments,
}

impl BackendError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Capture | Self::Block | Self::Restore => "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF",
            Self::Spawn => "D2B-BZLEXEC-PARENT-SPAWN",
            Self::Mapping | Self::StatusPipe => "D2B-BZLEXEC-PARENT-PREPARE",
            Self::HelperIdentity => "D2B-BZLEXEC-PARENT-HELPER-IDENTITY",
            Self::TargetArguments => "D2B-BZLEXEC-PARENT-TARGET",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffError {
    GuardPoisoned,
    Backend(BackendError),
    RestoreAfterSpawn,
    RestoreAfterSpawnFailure,
    Protocol(ProtocolError),
    Wait,
    StatusMismatch,
    Target(TerminalStatus),
}

impl HandoffError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::GuardPoisoned | Self::RestoreAfterSpawn | Self::RestoreAfterSpawnFailure => {
                "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF"
            }
            Self::Backend(error) => error.code(),
            Self::Protocol(error) => error.code(),
            Self::Wait => "D2B-BZLEXEC-PARENT-WAIT",
            Self::StatusMismatch => "D2B-BZLEXEC-PARENT-STATUS",
            Self::Target(_) => "D2B-BZLEXEC-PARENT-TARGET",
        }
    }
}

impl fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for HandoffError {}

/// An opaque captured mask. Test values exist only in the explicit
/// `test-support` feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaskSnapshot {
    Native(SigSet),
    #[cfg(feature = "test-support")]
    Test(u64),
}

/// One process-wide lock for the complete capture, block, spawn, and restore
/// handoff. It is intentionally reusable in tests so overlap is deterministic.
#[derive(Debug)]
pub struct LaunchCoordinator {
    gate: Mutex<()>,
}

impl LaunchCoordinator {
    pub const fn new() -> Self {
        Self {
            gate: Mutex::new(()),
        }
    }

    /// Poison only an injected coordinator; this is unavailable to production
    /// callers.
    #[cfg(feature = "test-support")]
    pub fn poison_for_test(&self) {
        let _guard = self.gate.lock().expect("coordinator must be unpoisoned");
        panic!("injected poisoned launch coordinator");
    }
}

impl Default for LaunchCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

static PROCESS_LAUNCH_COORDINATOR: OnceLock<LaunchCoordinator> = OnceLock::new();

fn process_launch_coordinator() -> &'static LaunchCoordinator {
    PROCESS_LAUNCH_COORDINATOR.get_or_init(LaunchCoordinator::new)
}

pub fn managed_signals() -> SigSet {
    let mut signals = SigSet::empty();
    signals.add(Signal::SIGHUP);
    signals.add(Signal::SIGINT);
    signals.add(Signal::SIGTERM);
    signals.add(Signal::SIGQUIT);
    signals
}

trait LaunchBackend {
    fn capture_mask(&self) -> Result<MaskSnapshot, BackendError>;
    fn block_managed(&self) -> Result<(), BackendError>;
    fn restore_mask(&self, snapshot: MaskSnapshot) -> Result<(), BackendError>;
    fn spawn(&self, plan: InternalLaunchPlan) -> Result<InternalSpawnReceipt, BackendError>;
}

fn launch_with_signal_handoff<B: LaunchBackend>(
    coordinator: &LaunchCoordinator,
    backend: &B,
    plan: InternalLaunchPlan,
) -> Result<InternalSpawnReceipt, HandoffError> {
    let _guard = coordinator
        .gate
        .lock()
        .map_err(|_| HandoffError::GuardPoisoned)?;
    let snapshot = backend.capture_mask().map_err(HandoffError::Backend)?;
    if let Err(error) = backend.block_managed() {
        let restore = backend.restore_mask(snapshot);
        if let Err(restore_error) = restore {
            return Err(HandoffError::Backend(restore_error));
        }
        return Err(HandoffError::Backend(error));
    }

    // `spawn` returns immediately after the helper is created. Waiting for
    // status is intentionally outside this closure so restoration happens
    // immediately after spawn and before the process-wide guard is released.
    let result = backend.spawn(plan);
    let restored = backend.restore_mask(snapshot);
    match (result, restored) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(_)) => Err(HandoffError::RestoreAfterSpawn),
        (Err(error), Ok(())) => Err(HandoffError::Backend(error)),
        (Err(_), Err(_)) => Err(HandoffError::RestoreAfterSpawnFailure),
    }
}

/// The only production API that consumes `VerifiedExecutable`.
pub fn execute_verified(
    executable: VerifiedExecutable,
    request: ExecutionRequest,
) -> Result<ExecutionResult, HandoffError> {
    if request.target_argv.is_empty()
        || request
            .target_argv
            .first()
            .is_some_and(|value| value.as_os_str().is_empty())
    {
        return Err(HandoffError::Backend(BackendError::TargetArguments));
    }
    #[cfg(unix)]
    {
        let private_fd = executable
            .duplicate_for_mapping()
            .map_err(|_| HandoffError::Backend(BackendError::Mapping))?;
        let plan = InternalLaunchPlan {
            private_fd,
            request,
            supervisor: SupervisorIdentity::immutable(),
        };
        let receipt =
            launch_with_signal_handoff(process_launch_coordinator(), &ProductionBackend, plan)?;
        receipt.finish()
    }
    #[cfg(not(unix))]
    {
        let _ = (executable, request);
        Err(HandoffError::Backend(BackendError::HelperIdentity))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ProductionBackend;

impl LaunchBackend for ProductionBackend {
    fn capture_mask(&self) -> Result<MaskSnapshot, BackendError> {
        SigSet::thread_get_mask()
            .map(MaskSnapshot::Native)
            .map_err(|_| BackendError::Capture)
    }

    fn block_managed(&self) -> Result<(), BackendError> {
        managed_signals()
            .thread_block()
            .map_err(|_| BackendError::Block)
    }

    fn restore_mask(&self, snapshot: MaskSnapshot) -> Result<(), BackendError> {
        match snapshot {
            MaskSnapshot::Native(mask) => mask.thread_set_mask().map_err(|_| BackendError::Restore),
            #[cfg(feature = "test-support")]
            MaskSnapshot::Test(_) => Err(BackendError::Restore),
        }
    }

    fn spawn(&self, plan: InternalLaunchPlan) -> Result<InternalSpawnReceipt, BackendError> {
        #[cfg(unix)]
        {
            if !plan.supervisor.immutable {
                return Err(BackendError::HelperIdentity);
            }
            if plan.request.target_argv.is_empty()
                || plan
                    .request
                    .target_argv
                    .first()
                    .is_some_and(|value| value.as_os_str().is_empty())
            {
                return Err(BackendError::TargetArguments);
            }
            let helper = immutable_supervisor_path().ok_or(BackendError::HelperIdentity)?;
            let (status_reader, status_writer) =
                pipe_with(PipeFlags::CLOEXEC).map_err(|_| BackendError::StatusPipe)?;

            let mut command = Command::new(helper);
            command.stdin(stdio(plan.request.stdin));
            command.stdout(stdio(plan.request.stdout));
            command.stderr(stdio(plan.request.stderr));
            command.args(&plan.request.target_argv);
            command
                .fd_mappings(vec![
                    FdMapping {
                        parent_fd: plan.private_fd,
                        child_fd: PRIVATE_EXECUTABLE_FD,
                    },
                    FdMapping {
                        parent_fd: status_writer,
                        child_fd: PRIVATE_STATUS_FD,
                    },
                ])
                .map_err(|_| BackendError::Mapping)?;
            let child = command.spawn().map_err(|_| BackendError::Spawn)?;
            Ok(InternalSpawnReceipt::Child {
                child,
                status_reader: File::from(status_reader),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = plan;
            Err(BackendError::HelperIdentity)
        }
    }
}

fn immutable_supervisor_path() -> Option<&'static Path> {
    let value = IMMUTABLE_SUPERVISOR_PATH?;
    let path = Path::new(value);
    let valid_store_path = path.is_absolute()
        && value.starts_with("/nix/store/")
        && path
            .file_name()
            .is_some_and(|name| name == "d2b-bazel-exec-supervisor");
    valid_store_path.then_some(path)
}

fn stdio(policy: StdioPolicy) -> Stdio {
    match policy {
        StdioPolicy::Inherit => Stdio::inherit(),
        StdioPolicy::Null => Stdio::null(),
    }
}

enum InternalSpawnReceipt {
    #[cfg(unix)]
    Child { child: Child, status_reader: File },
    #[cfg(feature = "test-support")]
    Test { helper_started: bool },
}

impl InternalSpawnReceipt {
    fn finish(self) -> Result<ExecutionResult, HandoffError> {
        match self {
            #[cfg(unix)]
            Self::Child {
                mut child,
                status_reader,
            } => {
                let protocol = read_status(status_reader);
                let waited = child.wait().map_err(|_| HandoffError::Wait);
                let terminal = match (protocol, waited) {
                    (Err(error), _) => return Err(HandoffError::Protocol(error)),
                    (Ok(_), Err(error)) => return Err(error),
                    (Ok(terminal), Ok(status)) => {
                        ensure_helper_status(status, terminal)?;
                        terminal
                    }
                };
                if terminal != TerminalStatus::Exited(0) {
                    return Err(HandoffError::Target(terminal));
                }
                Ok(ExecutionResult {
                    helper_started: true,
                    terminal,
                })
            }
            #[cfg(feature = "test-support")]
            Self::Test { helper_started } => Ok(ExecutionResult {
                helper_started,
                terminal: TerminalStatus::Exited(0),
            }),
        }
    }
}

#[cfg(unix)]
fn read_status(mut reader: File) -> Result<TerminalStatus, ProtocolError> {
    let mut protocol = ProtocolReader::new();
    let mut buffer = [0_u8; STATUS_BUFFER_CAPACITY];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return protocol.eof(),
            Ok(length) => {
                protocol.feed(&buffer[..length])?;
            }
            Err(_) => return Err(ProtocolError::StatusRead),
        }
    }
}

#[cfg(unix)]
fn ensure_helper_status(status: ExitStatus, terminal: TerminalStatus) -> Result<(), HandoffError> {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    let matches = match terminal {
        TerminalStatus::Exited(code) => status.code() == Some(i32::from(code)),
        TerminalStatus::Signaled(signal) => {
            status.code() == Some(128_i32.saturating_add(i32::from(signal)))
        }
    };
    if matches && status.signal().is_none() {
        Ok(())
    } else {
        Err(HandoffError::StatusMismatch)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusFrame {
    Ready,
    Executed,
    Exited(u8),
    Signaled(u8),
}

pub type ProtocolFrame = StatusFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolState {
    Start,
    Ready,
    Executed,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalStatus {
    Exited(u8),
    Signaled(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    BadMagic,
    BadVersion,
    UnknownType,
    InvalidLength,
    InvalidSignal,
    BufferOverflow,
    PartialEof,
    EofBeforeTerminal,
    TrailingFrame,
    DuplicateFrame,
    OutOfOrder,
    MissingInitialStop,
    WrongInitialStop,
    GroupMismatch,
    ChildExitedEarly,
    OptionsFailed,
    ContinueFailed,
    ExecEventMissing,
    PreExecDeath,
    DetachFailed,
    PreExecTermination,
    StatusNotExecuted,
    StatusMismatch,
    ExecErrorPartial,
    ExecErrorOverlong,
    ExecErrorUnknown,
    ExecErrorHeldOpen,
    EmptyExecErrorEof,
    HelperBeforeExecuted,
    StatusEpipe,
    StatusRead,
    UnknownChildCode,
}

impl ProtocolError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingInitialStop => "D2B-BZLEXEC-HELPER-PTRACE-STOP",
            Self::WrongInitialStop => "D2B-BZLEXEC-HELPER-PTRACE-STOP",
            Self::GroupMismatch => "D2B-BZLEXEC-HELPER-GROUP-ERROR",
            Self::ChildExitedEarly => "D2B-BZLEXEC-HELPER-GROUP-EARLY-EXIT",
            Self::OptionsFailed => "D2B-BZLEXEC-HELPER-PTRACE-OPTIONS",
            Self::ContinueFailed => "D2B-BZLEXEC-HELPER-PTRACE-CONT",
            Self::ExecEventMissing => "D2B-BZLEXEC-HELPER-PTRACE-EVENT",
            Self::PreExecDeath => "D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH",
            Self::DetachFailed => "D2B-BZLEXEC-HELPER-PTRACE-DETACH",
            Self::PreExecTermination => "D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION",
            Self::HelperBeforeExecuted => "D2B-BZLEXEC-PARENT-EXECUTED",
            Self::StatusEpipe => "D2B-BZLEXEC-HELPER-EXEC-EPIPE",
            Self::StatusRead => "D2B-BZLEXEC-PARENT-STATUS",
            Self::UnknownChildCode => "D2B-BZLEXEC-HELPER-EXEC-UNKNOWN",
            _ => "D2B-BZLEXEC-PARENT-PROTOCOL",
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProtocolError {}

/// A fixed-capacity status decoder. It retains at most one complete frame
/// plus coalesced frames and never treats EOF before the terminal frame as
/// success.
pub struct StatusDecoder {
    buffer: [u8; STATUS_BUFFER_CAPACITY],
    length: usize,
    state: ProtocolState,
    terminal: Option<TerminalStatus>,
}

impl StatusDecoder {
    pub const fn new() -> Self {
        Self {
            buffer: [0; STATUS_BUFFER_CAPACITY],
            length: 0,
            state: ProtocolState::Start,
            terminal: None,
        }
    }

    pub const fn state(&self) -> ProtocolState {
        self.state
    }

    pub const fn buffered_len(&self) -> usize {
        self.length
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<StatusFrame>, ProtocolError> {
        let new_length = self
            .length
            .checked_add(bytes.len())
            .ok_or(ProtocolError::BufferOverflow)?;
        if new_length > STATUS_BUFFER_CAPACITY {
            return Err(ProtocolError::BufferOverflow);
        }
        if self.state == ProtocolState::Terminal && !bytes.is_empty() {
            return Err(ProtocolError::TrailingFrame);
        }
        self.buffer[self.length..new_length].copy_from_slice(bytes);
        self.length = new_length;

        let mut frames = Vec::new();
        loop {
            if self.length < 8 {
                break;
            }
            if self.buffer[..4] != STATUS_MAGIC {
                return Err(ProtocolError::BadMagic);
            }
            if self.buffer[4] != STATUS_VERSION {
                return Err(ProtocolError::BadVersion);
            }
            let kind = self.buffer[5];
            let payload_length = u16::from_be_bytes([self.buffer[6], self.buffer[7]]) as usize;
            let expected = match kind {
                1 | 2 if payload_length == 0 => 0,
                3 | 4 if payload_length == 1 => 1,
                1..=4 => return Err(ProtocolError::InvalidLength),
                _ => return Err(ProtocolError::UnknownType),
            };
            let frame_length = 8 + expected;
            if self.length < frame_length {
                break;
            }
            let frame = match kind {
                1 => StatusFrame::Ready,
                2 => StatusFrame::Executed,
                3 => StatusFrame::Exited(self.buffer[8]),
                4 if (1..=64).contains(&self.buffer[8]) => StatusFrame::Signaled(self.buffer[8]),
                4 => return Err(ProtocolError::InvalidSignal),
                _ => return Err(ProtocolError::UnknownType),
            };
            self.accept_order(frame)?;
            frames.push(frame);
            self.buffer.copy_within(frame_length..self.length, 0);
            self.length -= frame_length;
            if self.state == ProtocolState::Terminal && self.length != 0 {
                return Err(ProtocolError::TrailingFrame);
            }
        }
        Ok(frames)
    }

    pub fn finish_eof(&self) -> Result<TerminalStatus, ProtocolError> {
        if self.length != 0 {
            return Err(ProtocolError::PartialEof);
        }
        if self.state != ProtocolState::Terminal {
            return Err(ProtocolError::EofBeforeTerminal);
        }
        self.terminal.ok_or(ProtocolError::EofBeforeTerminal)
    }

    fn accept_order(&mut self, frame: StatusFrame) -> Result<(), ProtocolError> {
        match (self.state, frame) {
            (ProtocolState::Start, StatusFrame::Ready) => {
                self.state = ProtocolState::Ready;
                Ok(())
            }
            (ProtocolState::Ready, StatusFrame::Executed) => {
                self.state = ProtocolState::Executed;
                Ok(())
            }
            (ProtocolState::Executed, StatusFrame::Exited(code)) => {
                self.terminal = Some(TerminalStatus::Exited(code));
                self.state = ProtocolState::Terminal;
                Ok(())
            }
            (ProtocolState::Executed, StatusFrame::Signaled(signal)) => {
                self.terminal = Some(TerminalStatus::Signaled(signal));
                self.state = ProtocolState::Terminal;
                Ok(())
            }
            (ProtocolState::Start, _) | (ProtocolState::Ready, _) => Err(ProtocolError::OutOfOrder),
            (ProtocolState::Executed, _) => Err(ProtocolError::DuplicateFrame),
            (ProtocolState::Terminal, _) => Err(ProtocolError::TrailingFrame),
        }
    }
}

impl Default for StatusDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Stateful reader wrapper used by the parent-side transport seam.
pub struct ProtocolReader {
    decoder: StatusDecoder,
}

impl ProtocolReader {
    pub const fn new() -> Self {
        Self {
            decoder: StatusDecoder::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<StatusFrame>, ProtocolError> {
        self.decoder.feed(bytes)
    }

    pub fn eof(&self) -> Result<TerminalStatus, ProtocolError> {
        self.decoder.finish_eof()
    }
}

impl Default for ProtocolReader {
    fn default() -> Self {
        Self::new()
    }
}

pub fn encode_status(frame: StatusFrame) -> Vec<u8> {
    let mut encoded = vec![0_u8; 8];
    encoded[..4].copy_from_slice(&STATUS_MAGIC);
    encoded[4] = STATUS_VERSION;
    match frame {
        StatusFrame::Ready => {
            encoded[5] = 1;
        }
        StatusFrame::Executed => {
            encoded[5] = 2;
        }
        StatusFrame::Exited(code) => {
            encoded[5] = 3;
            encoded[7] = 1;
            encoded.push(code);
        }
        StatusFrame::Signaled(signal) => {
            encoded[5] = 4;
            encoded[7] = 1;
            encoded.push(signal);
        }
    }
    encoded
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ChildIdentity(u64);

impl ChildIdentity {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Debug for ChildIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildIdentity(..)")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct GroupIdentity(u64);

impl GroupIdentity {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Debug for GroupIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GroupIdentity(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitialStop {
    pub child: ChildIdentity,
    pub group: GroupIdentity,
    pub direct_parent: bool,
    pub stop_signal: u8,
    pub event: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecStop {
    pub child: ChildIdentity,
    pub stop_signal: u8,
    pub event: u32,
}

pub trait ContainmentBackend {
    fn terminate_confirmed_group(&mut self, group: GroupIdentity) -> Result<(), ProtocolError>;
    fn reap_direct_child(&mut self, child: ChildIdentity) -> Result<(), ProtocolError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupervisorEvent {
    pub frame: Option<StatusFrame>,
    pub audit_executed: bool,
}

pub struct SupervisorProtocol {
    state: ProtocolState,
    child: Option<ChildIdentity>,
    group: Option<GroupIdentity>,
    ready_emitted: bool,
    continued: bool,
    termination_requested: bool,
    audit_executed: bool,
    terminal: Option<TerminalStatus>,
    events: Vec<SupervisorEvent>,
}

impl SupervisorProtocol {
    pub const fn new() -> Self {
        Self {
            state: ProtocolState::Start,
            child: None,
            group: None,
            ready_emitted: false,
            continued: false,
            termination_requested: false,
            audit_executed: false,
            terminal: None,
            events: Vec::new(),
        }
    }

    pub const fn state(&self) -> ProtocolState {
        self.state
    }

    pub const fn termination_requested(&self) -> bool {
        self.termination_requested
    }

    pub const fn audit_executed(&self) -> bool {
        self.audit_executed
    }

    pub fn events(&self) -> &[SupervisorEvent] {
        &self.events
    }

    pub fn confirm_initial_stop(&mut self, stop: InitialStop) -> Result<(), ProtocolError> {
        if !stop.direct_parent || stop.child == ChildIdentity::new(0) {
            return Err(ProtocolError::GroupMismatch);
        }
        if stop.group == GroupIdentity::new(0) {
            return Err(ProtocolError::GroupMismatch);
        }
        if stop.stop_signal != 19 || stop.event != 0 {
            return Err(ProtocolError::WrongInitialStop);
        }
        self.child = Some(stop.child);
        self.group = Some(stop.group);
        Ok(())
    }

    pub fn install_trace_options(&mut self, success: bool) -> Result<(), ProtocolError> {
        if self.child.is_none() {
            return Err(ProtocolError::MissingInitialStop);
        }
        if !success {
            return Err(ProtocolError::OptionsFailed);
        }
        Ok(())
    }

    pub fn emit_ready(&mut self) -> Result<StatusFrame, ProtocolError> {
        if self.child.is_none() {
            return Err(ProtocolError::MissingInitialStop);
        }
        if self.ready_emitted {
            return Err(ProtocolError::DuplicateFrame);
        }
        self.ready_emitted = true;
        self.state = ProtocolState::Ready;
        let frame = StatusFrame::Ready;
        self.events.push(SupervisorEvent {
            frame: Some(frame),
            audit_executed: false,
        });
        Ok(frame)
    }

    pub fn release_initial_stop(&mut self, success: bool) -> Result<(), ProtocolError> {
        if !self.ready_emitted {
            return Err(ProtocolError::OutOfOrder);
        }
        if !success {
            return Err(ProtocolError::ContinueFailed);
        }
        self.continued = true;
        Ok(())
    }

    pub fn handle_before_exec_signal<B: ContainmentBackend>(
        &mut self,
        backend: &mut B,
    ) -> Result<(), ProtocolError> {
        if self.audit_executed || self.state == ProtocolState::Terminal {
            return Err(ProtocolError::OutOfOrder);
        }
        if self.termination_requested {
            return Err(ProtocolError::PreExecTermination);
        }
        self.termination_requested = true;
        let group = self.group.ok_or(ProtocolError::GroupMismatch)?;
        let child = self.child.ok_or(ProtocolError::MissingInitialStop)?;
        backend.terminate_confirmed_group(group)?;
        backend.reap_direct_child(child)?;
        self.events.push(SupervisorEvent {
            frame: None,
            audit_executed: false,
        });
        Err(ProtocolError::PreExecTermination)
    }

    pub fn handle_exec_stop(
        &mut self,
        stop: ExecStop,
        detach_success: bool,
    ) -> Result<StatusFrame, ProtocolError> {
        if !self.ready_emitted || !self.continued || self.state != ProtocolState::Ready {
            return Err(ProtocolError::ExecEventMissing);
        }
        if Some(stop.child) != self.child
            || stop.stop_signal != 5
            || stop.event != PTRACE_EVENT_EXEC
        {
            return Err(ProtocolError::ExecEventMissing);
        }
        if !detach_success {
            return Err(ProtocolError::DetachFailed);
        }
        self.state = ProtocolState::Executed;
        self.audit_executed = true;
        let frame = StatusFrame::Executed;
        self.events.push(SupervisorEvent {
            frame: Some(frame),
            audit_executed: true,
        });
        Ok(frame)
    }

    pub fn handle_before_exec_death(&mut self) -> Result<(), ProtocolError> {
        if self.audit_executed {
            return Err(ProtocolError::OutOfOrder);
        }
        Err(ProtocolError::PreExecDeath)
    }

    pub fn handle_terminal(
        &mut self,
        terminal: TerminalStatus,
    ) -> Result<StatusFrame, ProtocolError> {
        if !self.audit_executed || self.state != ProtocolState::Executed {
            return Err(ProtocolError::StatusNotExecuted);
        }
        self.state = ProtocolState::Terminal;
        self.terminal = Some(terminal);
        let frame = match terminal {
            TerminalStatus::Exited(code) => StatusFrame::Exited(code),
            TerminalStatus::Signaled(signal) => StatusFrame::Signaled(signal),
        };
        self.events.push(SupervisorEvent {
            frame: Some(frame),
            audit_executed: true,
        });
        Ok(frame)
    }

    pub fn terminal(&self) -> Option<TerminalStatus> {
        self.terminal
    }
}

impl Default for SupervisorProtocol {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildStage {
    Group,
    Signal,
    Stdio,
    Cloexec,
    Close,
    Ptrace,
    Stop,
    Execveat,
}

impl ChildStage {
    const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Group),
            2 => Some(Self::Signal),
            3 => Some(Self::Stdio),
            4 => Some(Self::Cloexec),
            5 => Some(Self::Close),
            6 => Some(Self::Ptrace),
            7 => Some(Self::Stop),
            8 => Some(Self::Execveat),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ExecErrorRecord {
    pub code: u16,
}

impl ExecErrorRecord {
    pub const fn stage(self) -> Option<ChildStage> {
        ChildStage::from_wire(self.code as u8)
    }
}

impl fmt::Debug for ExecErrorRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExecErrorRecord(..)")
    }
}

pub const EXEC_ERROR_RECORD_SIZE: usize = 8;

pub fn decode_exec_error(
    bytes: &[u8],
    eof: bool,
) -> Result<Option<ExecErrorRecord>, ProtocolError> {
    if bytes.is_empty() {
        return if eof {
            Err(ProtocolError::EmptyExecErrorEof)
        } else {
            Err(ProtocolError::ExecErrorHeldOpen)
        };
    }
    if bytes.len() < EXEC_ERROR_RECORD_SIZE {
        return if eof {
            Err(ProtocolError::ExecErrorPartial)
        } else {
            Err(ProtocolError::ExecErrorHeldOpen)
        };
    }
    if bytes.len() > EXEC_ERROR_RECORD_SIZE {
        return Err(ProtocolError::ExecErrorOverlong);
    }
    if bytes[..4] != *b"D2BE" || bytes[4] != 1 || bytes[5] != 1 || bytes[6] != 0 {
        return Err(ProtocolError::ExecErrorUnknown);
    }
    if !eof {
        return Err(ProtocolError::ExecErrorHeldOpen);
    }
    let code = u16::from_be_bytes([bytes[6], bytes[7]]);
    ChildStage::from_wire(bytes[7]).ok_or(ProtocolError::UnknownChildCode)?;
    Ok(Some(ExecErrorRecord { code }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusWriteError {
    ClosedReader,
    Other,
}

pub fn classify_status_write(error: StatusWriteError) -> Result<(), ProtocolError> {
    match error {
        StatusWriteError::ClosedReader => Err(ProtocolError::StatusEpipe),
        StatusWriteError::Other => Err(ProtocolError::StatusMismatch),
    }
}

pub fn helper_exit_before_executed() -> Result<(), ProtocolError> {
    Err(ProtocolError::HelperBeforeExecuted)
}

#[cfg(feature = "test-support")]
pub mod test_support {
    use super::{
        ExecutionRequest, HandoffError, InternalLaunchPlan, LaunchCoordinator, SupervisorIdentity,
        TerminalStatus,
    };
    use crate::VerifiedExecutable;
    use crate::provider::test_support::verified_executable;
    use std::fs::File;

    pub use super::{BackendError, LaunchPlan, MaskSnapshot};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct SpawnReceipt {
        helper_started: bool,
    }

    impl SpawnReceipt {
        pub const fn started() -> Self {
            Self {
                helper_started: true,
            }
        }

        pub const fn not_started() -> Self {
            Self {
                helper_started: false,
            }
        }

        pub const fn helper_started(self) -> bool {
            self.helper_started
        }
    }

    pub trait ExecutionBackend {
        fn capture_mask(&self) -> Result<MaskSnapshot, BackendError>;
        fn block_managed(&self) -> Result<(), BackendError>;
        fn restore_mask(&self, snapshot: MaskSnapshot) -> Result<(), BackendError>;
        fn spawn(&self, plan: LaunchPlan) -> Result<SpawnReceipt, BackendError>;
    }

    struct Adapter<'a, B>(&'a B);

    impl<B: ExecutionBackend> super::LaunchBackend for Adapter<'_, B> {
        fn capture_mask(&self) -> Result<MaskSnapshot, BackendError> {
            self.0.capture_mask()
        }

        fn block_managed(&self) -> Result<(), BackendError> {
            self.0.block_managed()
        }

        fn restore_mask(&self, snapshot: MaskSnapshot) -> Result<(), BackendError> {
            self.0.restore_mask(snapshot)
        }

        fn spawn(
            &self,
            plan: super::InternalLaunchPlan,
        ) -> Result<super::InternalSpawnReceipt, BackendError> {
            let public_plan = LaunchPlan {
                private_fd: plan.private_fd,
                request: plan.request,
                supervisor: plan.supervisor,
            };
            let receipt = self.0.spawn(public_plan)?;
            Ok(super::InternalSpawnReceipt::Test {
                helper_started: receipt.helper_started,
            })
        }
    }

    pub fn execute_verified_with_backend<B: ExecutionBackend>(
        executable: VerifiedExecutable,
        request: ExecutionRequest,
        backend: &B,
    ) -> Result<super::ExecutionResult, HandoffError> {
        #[cfg(unix)]
        let private_fd = executable
            .duplicate_for_mapping()
            .map_err(|_| HandoffError::Backend(BackendError::Mapping))?;
        #[cfg(not(unix))]
        let private_fd = ();
        let plan = InternalLaunchPlan {
            private_fd,
            request,
            supervisor: SupervisorIdentity::immutable(),
        };
        let receipt =
            super::launch_with_signal_handoff(&LaunchCoordinator::new(), &Adapter(backend), plan)?;
        receipt.finish()
    }

    pub fn run_signal_handoff<B, F, T>(
        coordinator: &LaunchCoordinator,
        backend: &B,
        spawn: F,
    ) -> Result<T, HandoffError>
    where
        B: ExecutionBackend,
        F: FnOnce() -> Result<T, BackendError>,
    {
        let _guard = coordinator
            .gate
            .lock()
            .map_err(|_| HandoffError::GuardPoisoned)?;
        let snapshot = backend.capture_mask().map_err(HandoffError::Backend)?;
        if let Err(error) = backend.block_managed() {
            let restore = backend.restore_mask(snapshot);
            if let Err(restore_error) = restore {
                return Err(HandoffError::Backend(restore_error));
            }
            return Err(HandoffError::Backend(error));
        }
        let result = spawn();
        let restored = backend.restore_mask(snapshot);
        match (result, restored) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(_)) => Err(HandoffError::RestoreAfterSpawn),
            (Err(error), Ok(())) => Err(HandoffError::Backend(error)),
            (Err(_), Err(_)) => Err(HandoffError::RestoreAfterSpawnFailure),
        }
    }

    pub fn verified_file(file: File) -> VerifiedExecutable {
        verified_executable(file)
    }

    pub fn target_succeeded(result: &super::ExecutionResult) -> bool {
        result.terminal == TerminalStatus::Exited(0)
    }
}
