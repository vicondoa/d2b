use std::{
    ffi::OsString,
    fmt,
    fs::File,
    io::{self, Read},
    os::fd::OwnedFd,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::fd::AsFd;

use command_fds::{CommandFdExt, FdMapping};
use d2b_bazel_support::startup::{
    KernelVersion, NativeSystem, RuntimeStartupProbe, StartupCode, StartupProbe,
    StartupRequirements, validate_startup,
};
use nix::{
    poll::{PollFd, PollFlags, PollTimeout, poll},
    sys::signal::{SigSet, Signal},
};

use crate::provider::VerifiedExecutable;

#[cfg(unix)]
use rustix::pipe::{PipeFlags, pipe_with};

const PRIVATE_STATUS_FD: i32 = 8;
const PRIVATE_EXECUTABLE_FD: i32 = 9;
const PRIVATE_HELPER_ERROR_FD: i32 = 10;
pub const SUPERVISOR_ENVIRONMENT: &str = "D2B_BAZEL_EXEC_SUPERVISOR";
const IMMUTABLE_SUPERVISOR_PATH: Option<&str> = option_env!("D2B_BAZEL_EXEC_SUPERVISOR");
const STATUS_PHASE_TIMEOUT: Duration = Duration::from_secs(10);

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

pub const HELPER_ERROR_CODES: &[&str] = &[
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

    #[cfg(test)]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[cfg(test)]
    pub const fn is_immutable(self) -> bool {
        self.immutable
    }
}

/// A launch plan is crate-internal test state. Production callers cannot
/// implement a backend that receives an executable descriptor or inspect it.
#[cfg(test)]
pub struct LaunchPlan {
    #[cfg(unix)]
    private_fd: OwnedFd,
    #[cfg(not(unix))]
    private_fd: (),
    request: ExecutionRequest,
    supervisor: SupervisorIdentity,
}

#[cfg(test)]
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
    HelperErrorPipe,
    Startup(StartupCode),
    TargetArguments,
}

impl BackendError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Capture | Self::Block | Self::Restore => "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF",
            Self::Spawn => "D2B-BZLEXEC-PARENT-SPAWN",
            Self::Mapping | Self::StatusPipe | Self::HelperErrorPipe => {
                "D2B-BZLEXEC-PARENT-PREPARE"
            }
            Self::HelperIdentity => "D2B-BZLEXEC-PARENT-HELPER-IDENTITY",
            Self::Startup(code) => code.as_str(),
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

/// An opaque captured mask. Synthetic values exist only in crate-internal
/// tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaskSnapshot {
    Native(SigSet),
    #[cfg(test)]
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
    #[cfg(test)]
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
#[cfg(unix)]
static PENDING_CLEANUP_CHILDREN: OnceLock<Mutex<Vec<Child>>> = OnceLock::new();
#[cfg(unix)]
static CLEANUP_REAPER: OnceLock<()> = OnceLock::new();

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

fn runtime_native_system() -> NativeSystem {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        NativeSystem::X86_64Linux
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        NativeSystem::Aarch64Linux
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    )))]
    {
        NativeSystem::Unsupported
    }
}

fn runtime_kernel_version() -> Result<KernelVersion, StartupCode> {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_err(|_| StartupCode::ProbeFailed)?;
    let mut components = release.trim().split('.');
    let major = components
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(StartupCode::ProbeFailed)?;
    let minor = components
        .next()
        .and_then(|value| value.split('-').next())
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(StartupCode::ProbeFailed)?;
    Ok(KernelVersion::new(major, minor))
}

fn runtime_yama_scope() -> Result<Option<u8>, StartupCode> {
    match std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope") {
        Ok(value) => value
            .trim()
            .parse::<u8>()
            .map(Some)
            .map_err(|_| StartupCode::ProbeFailed),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(StartupCode::ProbeFailed),
    }
}

fn runtime_startup_requirements() -> Result<StartupRequirements, StartupCode> {
    Ok(StartupRequirements {
        system: runtime_native_system(),
        kernel: runtime_kernel_version()?,
        yama_scope: runtime_yama_scope()?,
        sandbox_policy_ok: immutable_supervisor_path().is_some(),
    })
}

fn execute_after_startup<B: LaunchBackend, P: StartupProbe>(
    executable: VerifiedExecutable,
    request: ExecutionRequest,
    backend: &B,
    requirements: StartupRequirements,
    probe: &P,
) -> Result<ExecutionResult, HandoffError> {
    if request.target_argv.is_empty()
        || request
            .target_argv
            .first()
            .is_some_and(|value| value.as_os_str().is_empty())
    {
        return Err(HandoffError::Backend(BackendError::TargetArguments));
    }
    validate_startup(requirements, probe)
        .map_err(|error| HandoffError::Backend(BackendError::Startup(error.code())))?;
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
        let receipt = launch_with_signal_handoff(process_launch_coordinator(), backend, plan)?;
        receipt.finish()
    }
    #[cfg(not(unix))]
    {
        let _ = (executable, request, backend);
        Err(HandoffError::Backend(BackendError::HelperIdentity))
    }
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
        (Ok(value), Err(_)) => {
            value.cleanup();
            Err(HandoffError::RestoreAfterSpawn)
        }
        (Err(error), Ok(())) => Err(HandoffError::Backend(error)),
        (Err(_), Err(_)) => Err(HandoffError::RestoreAfterSpawnFailure),
    }
}

/// The only production API that consumes `VerifiedExecutable`.
pub fn execute_verified(
    executable: VerifiedExecutable,
    request: ExecutionRequest,
) -> Result<ExecutionResult, HandoffError> {
    let requirements = runtime_startup_requirements()
        .map_err(|code| HandoffError::Backend(BackendError::Startup(code)))?;
    execute_after_startup(
        executable,
        request,
        &ProductionBackend,
        requirements,
        &RuntimeStartupProbe,
    )
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
            #[cfg(test)]
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
            let (helper_error_reader, helper_error_writer) =
                pipe_with(PipeFlags::CLOEXEC).map_err(|_| BackendError::HelperErrorPipe)?;

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
                    FdMapping {
                        parent_fd: helper_error_writer,
                        child_fd: PRIVATE_HELPER_ERROR_FD,
                    },
                ])
                .map_err(|_| BackendError::Mapping)?;
            let child = command.spawn().map_err(|_| BackendError::Spawn)?;
            Ok(InternalSpawnReceipt::Child {
                child,
                status_reader: File::from(status_reader),
                helper_error_reader: File::from(helper_error_reader),
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
    Child {
        child: Child,
        status_reader: File,
        helper_error_reader: File,
    },
    #[cfg(test)]
    Test {
        helper_started: bool,
        cleanup_observer: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    },
}

impl InternalSpawnReceipt {
    fn cleanup(self) {
        match self {
            #[cfg(unix)]
            Self::Child {
                child,
                status_reader,
                helper_error_reader,
            } => {
                drop(status_reader);
                drop(helper_error_reader);
                retain_cleanup_child(child);
            }
            #[cfg(test)]
            Self::Test {
                cleanup_observer, ..
            } => {
                cleanup_observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    fn finish(self) -> Result<ExecutionResult, HandoffError> {
        match self {
            #[cfg(unix)]
            Self::Child {
                mut child,
                status_reader,
                helper_error_reader,
            } => {
                let terminal = match read_status(status_reader, helper_error_reader) {
                    Ok(terminal) => terminal,
                    Err(error) => {
                        retain_cleanup_child(child);
                        return Err(HandoffError::Protocol(error));
                    }
                };
                let status = match wait_child_bounded(&mut child, STATUS_PHASE_TIMEOUT) {
                    Ok(Some(status)) => status,
                    Ok(None) | Err(_) => {
                        retain_cleanup_child(child);
                        return Err(HandoffError::Wait);
                    }
                };
                ensure_helper_status(status, terminal)?;
                if terminal != TerminalStatus::Exited(0) {
                    return Err(HandoffError::Target(terminal));
                }
                Ok(ExecutionResult {
                    helper_started: true,
                    terminal,
                })
            }
            #[cfg(test)]
            Self::Test { helper_started, .. } => Ok(ExecutionResult {
                helper_started,
                terminal: TerminalStatus::Exited(0),
            }),
        }
    }
}

#[cfg(unix)]
fn wait_child_bounded(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(Some(status)),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            None => return Ok(None),
        }
    }
}

#[cfg(unix)]
fn pending_cleanup_children() -> &'static Mutex<Vec<Child>> {
    PENDING_CLEANUP_CHILDREN.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(unix)]
fn reap_cleanup_children() {
    let mut children = pending_cleanup_children()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    children.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
}

#[cfg(unix)]
fn retain_cleanup_child(mut child: Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    {
        let mut children = pending_cleanup_children()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        children.push(child);
    }
    CLEANUP_REAPER.get_or_init(|| {
        let _ = thread::Builder::new()
            .name("d2b-bazel-exec-reaper".to_owned())
            .spawn(|| {
                loop {
                    reap_cleanup_children();
                    thread::sleep(Duration::from_millis(10));
                }
            });
    });
}

#[cfg(unix)]
fn phase_timeout(state: ProtocolState) -> ProtocolError {
    match state {
        ProtocolState::Start => ProtocolError::ReadyTimeout,
        ProtocolState::Ready => ProtocolError::ExecutedTimeout,
        ProtocolState::Executed | ProtocolState::Terminal => ProtocolError::TerminalTimeout,
    }
}

#[cfg(unix)]
fn deadline_for_phase(state: ProtocolState, timeout: Duration) -> Option<Instant> {
    match state {
        ProtocolState::Start | ProtocolState::Ready | ProtocolState::Terminal => {
            Some(Instant::now() + timeout)
        }
        ProtocolState::Executed => None,
    }
}

#[cfg(unix)]
fn nearest_deadline(
    phase: Option<Instant>,
    status_fragment: Option<Instant>,
    helper_fragment: Option<Instant>,
) -> Option<Instant> {
    [phase, status_fragment, helper_fragment]
        .into_iter()
        .flatten()
        .min()
}

#[cfg(unix)]
fn expired_protocol_error(
    state: ProtocolState,
    phase: Option<Instant>,
    status_fragment: Option<Instant>,
    helper_fragment: Option<Instant>,
) -> ProtocolError {
    let now = Instant::now();
    if helper_fragment.is_some_and(|deadline| deadline <= now) {
        ProtocolError::ExecErrorHeldOpen
    } else if status_fragment.is_some_and(|deadline| deadline <= now)
        || phase.is_some_and(|deadline| deadline <= now)
    {
        phase_timeout(state)
    } else {
        ProtocolError::StatusRead
    }
}

#[cfg(unix)]
fn remaining_poll_timeout(deadline: Option<Instant>) -> PollTimeout {
    let Some(deadline) = deadline else {
        return PollTimeout::NONE;
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return PollTimeout::ZERO;
    }
    PollTimeout::from(
        u16::try_from(remaining.as_millis().min(u16::MAX as u128)).unwrap_or(u16::MAX),
    )
}

#[cfg(unix)]
fn read_helper_error<R: Read>(
    reader: &mut R,
    bytes: &mut Vec<u8>,
) -> Result<(Option<HelperStage>, bool, bool), ProtocolError> {
    let mut buffer = [0_u8; EXEC_ERROR_RECORD_SIZE + 1];
    match reader.read(&mut buffer) {
        Ok(0) if bytes.is_empty() => Ok((None, true, false)),
        Ok(0) => match decode_helper_error(bytes, true) {
            Ok(Some(stage)) => Ok((Some(stage), true, false)),
            Ok(None) => Err(ProtocolError::EmptyExecErrorEof),
            Err(error) => Err(error),
        },
        Ok(length) => {
            bytes.extend_from_slice(&buffer[..length]);
            if bytes.len() > EXEC_ERROR_RECORD_SIZE {
                return Err(ProtocolError::ExecErrorOverlong);
            }
            if bytes.len() == EXEC_ERROR_RECORD_SIZE {
                let stage =
                    decode_helper_error(bytes, true)?.ok_or(ProtocolError::EmptyExecErrorEof)?;
                return Ok((Some(stage), false, false));
            }
            Ok((None, false, true))
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok((None, false, false)),
        Err(_) => Err(ProtocolError::StatusRead),
    }
}

#[cfg(unix)]
fn read_status(
    status_reader: File,
    helper_error_reader: File,
) -> Result<TerminalStatus, ProtocolError> {
    read_status_with_timeout(status_reader, helper_error_reader, STATUS_PHASE_TIMEOUT)
}

#[cfg(unix)]
fn read_status_with_timeout(
    mut status_reader: File,
    mut helper_error_reader: File,
    timeout: Duration,
) -> Result<TerminalStatus, ProtocolError> {
    let mut protocol = ProtocolReader::new();
    let mut buffer = [0_u8; STATUS_BUFFER_CAPACITY];
    let mut helper_error = Vec::with_capacity(EXEC_ERROR_RECORD_SIZE);
    let mut helper_closed = false;
    let mut phase = protocol.state();
    let mut phase_deadline = deadline_for_phase(phase, timeout);
    let mut status_fragment_deadline = None;
    let mut helper_fragment_deadline = None;
    loop {
        let mut poll_fds = vec![PollFd::new(
            status_reader.as_fd(),
            PollFlags::POLLIN | PollFlags::POLLHUP,
        )];
        if !helper_closed {
            poll_fds.push(PollFd::new(
                helper_error_reader.as_fd(),
                PollFlags::POLLIN | PollFlags::POLLHUP,
            ));
        }
        let deadline = nearest_deadline(
            phase_deadline,
            status_fragment_deadline,
            helper_fragment_deadline,
        );
        if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
            return Err(expired_protocol_error(
                protocol.state(),
                phase_deadline,
                status_fragment_deadline,
                helper_fragment_deadline,
            ));
        }
        let poll_result = match poll(&mut poll_fds, remaining_poll_timeout(deadline)) {
            Ok(value) => value,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Err(ProtocolError::StatusRead),
        };
        if poll_result == 0 {
            if deadline.is_some_and(|deadline| deadline > Instant::now()) {
                continue;
            }
            return Err(expired_protocol_error(
                protocol.state(),
                phase_deadline,
                status_fragment_deadline,
                helper_fragment_deadline,
            ));
        }
        let status_events = poll_fds[0].revents().unwrap_or(PollFlags::empty());
        let helper_events = poll_fds
            .get(1)
            .and_then(|descriptor| descriptor.revents())
            .unwrap_or(PollFlags::empty());
        drop(poll_fds);

        if status_events.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Err(ProtocolError::StatusRead);
        }
        let mut status_eof = false;
        if status_events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
            match status_reader.read(&mut buffer) {
                Ok(0) => status_eof = true,
                Ok(length) => {
                    protocol.feed(&buffer[..length])?;
                    if protocol.decoder.buffered_len() == 0 {
                        status_fragment_deadline = None;
                    } else if status_fragment_deadline.is_none() {
                        status_fragment_deadline = Some(Instant::now() + timeout);
                    }
                    if protocol.state() != phase {
                        phase = protocol.state();
                        phase_deadline = deadline_for_phase(phase, timeout);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(ProtocolError::StatusRead),
            }
        }

        if !helper_closed && helper_events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
            let (stage, closed, partial) =
                read_helper_error(&mut helper_error_reader, &mut helper_error)?;
            helper_closed |= closed;
            if partial && helper_fragment_deadline.is_none() {
                helper_fragment_deadline = Some(Instant::now() + timeout);
            }
            if closed {
                helper_fragment_deadline = None;
                if protocol.state() == ProtocolState::Start
                    || protocol.state() == ProtocolState::Ready
                {
                    return Err(ProtocolError::HelperBeforeExecuted);
                }
            }
            if let Some(stage) = stage {
                return Err(ProtocolError::HelperStage(stage));
            }
        }
        if status_eof {
            if !helper_error.is_empty() {
                return Err(ProtocolError::ExecErrorHeldOpen);
            }
            return protocol.eof();
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
    HelperStage(HelperStage),
    ReadyTimeout,
    ExecutedTimeout,
    TerminalTimeout,
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
            Self::HelperStage(stage) => stage.code(),
            Self::ReadyTimeout => "D2B-BZLEXEC-PARENT-READY",
            Self::ExecutedTimeout => "D2B-BZLEXEC-PARENT-EXECUTED",
            Self::TerminalTimeout => "D2B-BZLEXEC-PARENT-TERMINAL",
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

    pub const fn state(&self) -> ProtocolState {
        self.decoder.state()
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

/// A typed, closed helper failure received over the private parent channel.
///
/// The wire value is deliberately opaque to callers; only the fixed
/// repository diagnostic code is exposed.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct HelperStage(u8);

impl HelperStage {
    const fn from_wire(value: u8) -> Option<Self> {
        if value == 0 || value as usize > HELPER_ERROR_CODES.len() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub const fn code(self) -> &'static str {
        HELPER_ERROR_CODES[self.0 as usize - 1]
    }
}

impl fmt::Debug for HelperStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HelperStage(..)")
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

/// Decode one helper failure from the private helper-to-parent channel.
///
/// Unlike the child exec pipe, this record is never inherited by the target.
/// A complete record is sufficient to publish the typed failure; EOF is still
/// required for partial-record diagnostics.
pub fn decode_helper_error(bytes: &[u8], eof: bool) -> Result<Option<HelperStage>, ProtocolError> {
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
    if bytes[..4] != *b"D2BE" || bytes[4] != STATUS_VERSION || bytes[5] != 2 {
        return Err(ProtocolError::ExecErrorUnknown);
    }
    if !eof {
        return Err(ProtocolError::ExecErrorHeldOpen);
    }
    if bytes[6] != 0 {
        return Err(ProtocolError::ExecErrorUnknown);
    }
    HelperStage::from_wire(bytes[7])
        .map(Some)
        .ok_or(ProtocolError::UnknownChildCode)
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

#[cfg(test)]
mod tests {
    use super::{
        BackendError, ExecutionRequest, HandoffError, InternalLaunchPlan, LaunchCoordinator,
        LaunchPlan, MaskSnapshot, SupervisorIdentity,
    };
    use crate::VerifiedExecutable;
    use crate::provider::verified_executable_for_test;
    use d2b_bazel_support::startup::{
        KernelVersion, NativeSystem, ProbeResult, StartupCode, StartupRequirements,
    };
    use std::{
        collections::VecDeque,
        fs::File,
        io::{self, Read, Write},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct SpawnReceipt {
        helper_started: bool,
    }

    impl SpawnReceipt {
        const fn started() -> Self {
            Self {
                helper_started: true,
            }
        }

        const fn helper_started(self) -> bool {
            self.helper_started
        }
    }

    #[derive(Clone)]
    struct FakeBackend {
        events: Arc<Mutex<Vec<&'static str>>>,
        capture: Result<MaskSnapshot, BackendError>,
        block: Result<(), BackendError>,
        restore: Result<(), BackendError>,
        spawn: Result<SpawnReceipt, BackendError>,
        spawn_count: Arc<Mutex<usize>>,
        plan_seen: Arc<Mutex<bool>>,
        cleanup_observer: Arc<AtomicUsize>,
    }

    impl FakeBackend {
        fn passing() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
                capture: Ok(MaskSnapshot::Test(7)),
                block: Ok(()),
                restore: Ok(()),
                spawn: Ok(SpawnReceipt::started()),
                spawn_count: Arc::new(Mutex::new(0)),
                plan_seen: Arc::new(Mutex::new(false)),
                cleanup_observer: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn event_names(&self) -> Vec<&'static str> {
            self.events.lock().expect("events").clone()
        }

        fn spawn_count(&self) -> usize {
            *self.spawn_count.lock().expect("spawn count")
        }

        fn cleanup_count(&self) -> usize {
            self.cleanup_observer.load(Ordering::SeqCst)
        }
    }

    trait ExecutionBackend {
        fn capture_mask(&self) -> Result<MaskSnapshot, BackendError>;
        fn block_managed(&self) -> Result<(), BackendError>;
        fn restore_mask(&self, snapshot: MaskSnapshot) -> Result<(), BackendError>;
        fn spawn(&self, plan: LaunchPlan) -> Result<SpawnReceipt, BackendError>;

        fn cleanup_observer(&self) -> Arc<AtomicUsize>;
    }

    impl ExecutionBackend for FakeBackend {
        fn capture_mask(&self) -> Result<MaskSnapshot, BackendError> {
            self.events.lock().expect("events").push("capture");
            self.capture
        }

        fn block_managed(&self) -> Result<(), BackendError> {
            self.events.lock().expect("events").push("block");
            self.block
        }

        fn restore_mask(&self, _snapshot: MaskSnapshot) -> Result<(), BackendError> {
            self.events.lock().expect("events").push("restore");
            self.restore
        }

        fn spawn(&self, plan: LaunchPlan) -> Result<SpawnReceipt, BackendError> {
            self.events.lock().expect("events").push("spawn");
            *self.spawn_count.lock().expect("spawn count") += 1;
            *self.plan_seen.lock().expect("plan") = plan.preserves_standard_streams();
            self.spawn
        }

        fn cleanup_observer(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.cleanup_observer)
        }
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
                cleanup_observer: self.0.cleanup_observer(),
            })
        }
    }

    fn execute_verified_with_backend<B: ExecutionBackend>(
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

    fn run_signal_handoff<B, F, T>(
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

    fn verified_file(file: File) -> VerifiedExecutable {
        verified_executable_for_test(file)
    }

    fn passing_requirements() -> StartupRequirements {
        StartupRequirements {
            system: NativeSystem::X86_64Linux,
            kernel: KernelVersion::new(6, 1),
            yama_scope: Some(1),
            sandbox_policy_ok: true,
        }
    }

    #[cfg(unix)]
    fn status_pipes() -> (File, File, File, File) {
        let (status_reader, status_writer) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("status pipe");
        let (helper_reader, helper_writer) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("helper pipe");
        (
            File::from(status_reader),
            File::from(status_writer),
            File::from(helper_reader),
            File::from(helper_writer),
        )
    }

    #[cfg(unix)]
    enum ScriptedRead {
        Bytes(Vec<u8>),
        Interrupted,
        Eof,
    }

    #[cfg(unix)]
    struct ScriptedReader {
        reads: VecDeque<ScriptedRead>,
    }

    #[cfg(unix)]
    impl Read for ScriptedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            match self.reads.pop_front().expect("scripted read") {
                ScriptedRead::Bytes(bytes) => {
                    buffer[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                ScriptedRead::Interrupted => Err(io::ErrorKind::Interrupted.into()),
                ScriptedRead::Eof => Ok(0),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn helper_error_reads_preserve_interruption_partial_and_closed_boundaries() {
        let record = b"D2BE\x01\x02\x00\x0c".to_vec();
        let mut reader = ScriptedReader {
            reads: VecDeque::from([
                ScriptedRead::Interrupted,
                ScriptedRead::Bytes(record.clone()),
            ]),
        };
        let mut bytes = Vec::new();
        assert_eq!(
            super::read_helper_error(&mut reader, &mut bytes),
            Ok((None, false, false))
        );
        let (stage, closed, partial) =
            super::read_helper_error(&mut reader, &mut bytes).expect("complete helper record");
        assert_eq!(
            stage.expect("typed helper stage").code(),
            "D2B-BZLEXEC-HELPER-PTRACE-OPTIONS"
        );
        assert!(!closed);
        assert!(!partial);

        let mut reader = ScriptedReader {
            reads: VecDeque::from([ScriptedRead::Bytes(record[..3].to_vec()), ScriptedRead::Eof]),
        };
        let mut bytes = Vec::new();
        assert_eq!(
            super::read_helper_error(&mut reader, &mut bytes),
            Ok((None, false, true))
        );
        assert_eq!(
            super::read_helper_error(&mut reader, &mut bytes),
            Err(super::ProtocolError::ExecErrorPartial)
        );

        let mut reader = ScriptedReader {
            reads: VecDeque::from([ScriptedRead::Eof]),
        };
        assert_eq!(
            super::read_helper_error(&mut reader, &mut Vec::new()),
            Ok((None, true, false))
        );
    }

    #[cfg(unix)]
    #[test]
    fn ready_and_executed_handoffs_timeout_but_executed_runtime_does_not() {
        let timeout = Duration::from_millis(30);
        let (status_reader, status_writer, helper_reader, helper_writer) = status_pipes();
        assert_eq!(
            super::read_status_with_timeout(status_reader, helper_reader, timeout),
            Err(super::ProtocolError::ReadyTimeout)
        );
        drop(status_writer);
        drop(helper_writer);

        let (status_reader, mut status_writer, helper_reader, helper_writer) = status_pipes();
        status_writer
            .write_all(&super::encode_status(super::StatusFrame::Ready))
            .expect("READY frame");
        assert_eq!(
            super::read_status_with_timeout(status_reader, helper_reader, timeout),
            Err(super::ProtocolError::ExecutedTimeout)
        );
        drop(status_writer);
        drop(helper_writer);

        let (status_reader, mut status_writer, helper_reader, helper_writer) = status_pipes();
        let writer = std::thread::spawn(move || {
            status_writer
                .write_all(
                    &[
                        super::encode_status(super::StatusFrame::Ready),
                        super::encode_status(super::StatusFrame::Executed),
                    ]
                    .concat(),
                )
                .expect("handoff frames");
            std::thread::sleep(timeout * 3);
            status_writer
                .write_all(&super::encode_status(super::StatusFrame::Exited(0)))
                .expect("terminal frame");
            drop(status_writer);
            drop(helper_writer);
        });
        assert_eq!(
            super::read_status_with_timeout(status_reader, helper_reader, timeout),
            Ok(super::TerminalStatus::Exited(0))
        );
        writer.join().expect("status writer");
    }

    #[cfg(unix)]
    #[test]
    fn interleaved_helper_failure_and_empty_close_are_typed() {
        let timeout = Duration::from_millis(100);
        let (status_reader, mut status_writer, helper_reader, mut helper_writer) = status_pipes();
        let writer = std::thread::spawn(move || {
            status_writer
                .write_all(&super::encode_status(super::StatusFrame::Ready))
                .expect("READY frame");
            helper_writer
                .write_all(b"D2B")
                .expect("partial helper record");
            status_writer
                .write_all(&super::encode_status(super::StatusFrame::Executed))
                .expect("EXECUTED frame");
            drop(helper_writer);
        });
        assert_eq!(
            super::read_status_with_timeout(status_reader, helper_reader, timeout),
            Err(super::ProtocolError::ExecErrorPartial)
        );
        writer.join().expect("interleaved writer");

        let (status_reader, _status_writer, helper_reader, helper_writer) = status_pipes();
        drop(helper_writer);
        assert_eq!(
            super::read_status_with_timeout(status_reader, helper_reader, timeout),
            Err(super::ProtocolError::HelperBeforeExecuted)
        );
    }

    #[test]
    fn signal_handoff_restores_before_returning_to_the_caller() {
        let coordinator = LaunchCoordinator::new();
        let backend = FakeBackend::passing();
        let result = run_signal_handoff(&coordinator, &backend, || {
            backend.events.lock().expect("events").push("closure");
            Ok(SpawnReceipt::started())
        })
        .expect("handoff");
        assert!(result.helper_started());
        assert_eq!(
            backend.event_names(),
            ["capture", "block", "closure", "restore"]
        );
    }

    #[test]
    fn capture_and_block_failures_never_enter_the_spawn_closure() {
        let coordinator = LaunchCoordinator::new();
        let mut backend = FakeBackend::passing();
        backend.capture = Err(BackendError::Capture);
        let error = run_signal_handoff(
            &coordinator,
            &backend,
            || -> Result<SpawnReceipt, BackendError> {
                panic!("capture failure must not spawn");
            },
        )
        .expect_err("capture failure");
        assert_eq!(error, HandoffError::Backend(BackendError::Capture));
        assert_eq!(backend.event_names(), ["capture"]);

        let coordinator = LaunchCoordinator::new();
        let mut backend = FakeBackend::passing();
        backend.block = Err(BackendError::Block);
        let error = run_signal_handoff(
            &coordinator,
            &backend,
            || -> Result<SpawnReceipt, BackendError> {
                panic!("block failure must not spawn");
            },
        )
        .expect_err("block failure");
        assert_eq!(error, HandoffError::Backend(BackendError::Block));
        assert_eq!(backend.event_names(), ["capture", "block", "restore"]);
    }

    #[test]
    fn spawn_and_restore_failures_keep_their_first_typed_error() {
        let mut backend = FakeBackend::passing();
        backend.spawn = Err(BackendError::Spawn);
        let error = execute_verified_with_backend(
            verified_file(File::open("/dev/null").expect("test descriptor")),
            ExecutionRequest::default(),
            &backend,
        )
        .expect_err("spawn failure");
        assert_eq!(error, HandoffError::Backend(BackendError::Spawn));
        assert_eq!(
            backend.event_names(),
            ["capture", "block", "spawn", "restore"]
        );
        assert_eq!(backend.cleanup_count(), 0);

        let mut backend = FakeBackend::passing();
        backend.restore = Err(BackendError::Restore);
        let error = execute_verified_with_backend(
            verified_file(File::open("/dev/null").expect("test descriptor")),
            ExecutionRequest::default(),
            &backend,
        )
        .expect_err("restore failure");
        assert_eq!(error, HandoffError::RestoreAfterSpawn);
        assert_eq!(
            backend.event_names(),
            ["capture", "block", "spawn", "restore"]
        );
        assert_eq!(backend.cleanup_count(), 1);
    }

    #[test]
    fn poisoned_launch_coordinator_refuses_before_capture() {
        let coordinator = LaunchCoordinator::new();
        let _ = std::panic::catch_unwind(|| coordinator.poison_for_test());
        let backend = FakeBackend::passing();
        let error = run_signal_handoff(
            &coordinator,
            &backend,
            || -> Result<SpawnReceipt, BackendError> {
                panic!("poisoned coordinator must not spawn");
            },
        )
        .expect_err("poisoned coordinator");
        assert_eq!(error, HandoffError::GuardPoisoned);
        assert!(backend.event_names().is_empty());
    }

    #[test]
    fn startup_refusal_is_before_any_backend_spawn() {
        let cases = [
            (
                StartupRequirements {
                    system: NativeSystem::Unsupported,
                    ..passing_requirements()
                },
                ProbeResult::Pass,
                StartupCode::UnsupportedSystem,
            ),
            (
                StartupRequirements {
                    kernel: KernelVersion::new(3, 18),
                    ..passing_requirements()
                },
                ProbeResult::Pass,
                StartupCode::KernelTooOld,
            ),
            (
                StartupRequirements {
                    yama_scope: Some(2),
                    ..passing_requirements()
                },
                ProbeResult::Pass,
                StartupCode::YamaRefused,
            ),
            (
                passing_requirements(),
                ProbeResult::Fail,
                StartupCode::ProbeFailed,
            ),
            (
                StartupRequirements {
                    sandbox_policy_ok: false,
                    ..passing_requirements()
                },
                ProbeResult::Pass,
                StartupCode::SandboxPolicyDrift,
            ),
        ];

        for (requirements, probe, expected) in cases {
            let backend = FakeBackend::passing();
            let error = super::execute_after_startup(
                verified_file(File::open("/dev/null").expect("test descriptor")),
                ExecutionRequest::default(),
                &Adapter(&backend),
                requirements,
                &probe,
            )
            .expect_err("startup refusal");
            assert_eq!(
                error,
                HandoffError::Backend(BackendError::Startup(expected))
            );
            assert_eq!(backend.spawn_count(), 0);
        }
    }

    #[test]
    fn internal_test_backend_receives_only_the_consumed_capability_plan() {
        let backend = FakeBackend::passing();
        let result = execute_verified_with_backend(
            verified_file(File::open("/dev/null").expect("test descriptor")),
            ExecutionRequest::default(),
            &backend,
        )
        .expect("internal backend");
        assert!(result.helper_started);
        assert!(*backend.plan_seen.lock().expect("plan"));
        assert_eq!(backend.spawn_count(), 1);
    }
}
