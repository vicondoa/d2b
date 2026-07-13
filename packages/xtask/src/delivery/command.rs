use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::Read,
    os::{
        fd::{AsFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    flag,
};

use super::{
    DeliveryError, Result,
    model::{
        CheckPublisher, CheckPublisherKind, GitObjectFormat, PullRequestState, StackBranch,
        StackGraph, StackNodePolicy, StackPr, validate_git_ref, validate_hash,
        validate_repository_id,
    },
};

pub const DEFAULT_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_GIT_BLOB_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMMAND_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const TERMINATION_GRACE: Duration = Duration::from_millis(250);
const OUTPUT_DRAIN_GRACE: Duration = Duration::from_millis(100);
const OUTPUT_POLL_MILLISECONDS: i32 = 20;
const EXIT_COMMAND_TIMEOUT: i32 = -1;
const EXIT_STDOUT_OVERFLOW: i32 = -2;
const EXIT_STDERR_OVERFLOW: i32 = -3;
const EXIT_OUTPUT_OVERFLOW: i32 = -4;
const EXIT_INTERRUPTED: i32 = -5;
const EXIT_TERMINATED: i32 = -6;
pub const GIT_TOWN_LOCKED_VERSION: &str = "23.0.1";
pub const GIT_TOWN_SUPPORTED_MAJOR: u64 = 23;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandLimits {
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub timeout: Duration,
}

impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            stdout_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandFailure {
    Exit(i32),
    Signal,
    Timeout,
    StdoutOverflow,
    StderrOverflow,
    OutputOverflow,
    Interrupted,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrivateCommandDiagnostics<'a> {
    pub stdout: &'a [u8],
    pub stderr: &'a [u8],
}

impl CommandOutput {
    pub fn failure(&self) -> Option<CommandFailure> {
        if self.success {
            return None;
        }
        match self.exit_code {
            Some(EXIT_COMMAND_TIMEOUT) => Some(CommandFailure::Timeout),
            Some(EXIT_STDOUT_OVERFLOW) => Some(CommandFailure::StdoutOverflow),
            Some(EXIT_STDERR_OVERFLOW) => Some(CommandFailure::StderrOverflow),
            Some(EXIT_OUTPUT_OVERFLOW) => Some(CommandFailure::OutputOverflow),
            Some(EXIT_INTERRUPTED) => Some(CommandFailure::Interrupted),
            Some(EXIT_TERMINATED) => Some(CommandFailure::Terminated),
            Some(code) => Some(CommandFailure::Exit(code)),
            None => Some(CommandFailure::Signal),
        }
    }

    pub fn private_diagnostics(&self) -> PrivateCommandDiagnostics<'_> {
        PrivateCommandDiagnostics {
            stdout: &self.stdout,
            stderr: &self.stderr,
        }
    }

    pub fn safe_failure_summary(&self) -> String {
        let reason = match self.failure() {
            None => "command succeeded".to_owned(),
            Some(CommandFailure::Exit(code)) => format!("command exited with status {code}"),
            Some(CommandFailure::Signal) => "command exited after a signal".to_owned(),
            Some(CommandFailure::Timeout) => "command exceeded its deadline".to_owned(),
            Some(CommandFailure::StdoutOverflow) => {
                "command exceeded its stdout capture limit".to_owned()
            }
            Some(CommandFailure::StderrOverflow) => {
                "command exceeded its stderr capture limit".to_owned()
            }
            Some(CommandFailure::OutputOverflow) => {
                "command exceeded both output capture limits".to_owned()
            }
            Some(CommandFailure::Interrupted) => "command was interrupted by SIGINT".to_owned(),
            Some(CommandFailure::Terminated) => "command was terminated by SIGTERM".to_owned(),
        };
        format!(
            "{reason}; retained {} stdout bytes and {} stderr bytes for private diagnostics",
            self.stdout.len(),
            self.stderr.len()
        )
    }
}

pub trait CommandOutputAdapter {
    fn output_with_limits(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        limits: CommandLimits,
    ) -> Result<CommandOutput>;

    fn output_with_environment(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        environment: &BTreeMap<OsString, OsString>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        let _ = environment;
        self.output_with_limits(program, args, cwd, limits)
    }

    fn output(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<CommandOutput> {
        self.output_with_limits(program, args, cwd, CommandLimits::default())
    }
}

impl<A: CommandOutputAdapter + ?Sized> CommandOutputAdapter for &A {
    fn output_with_limits(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        (**self).output_with_limits(program, args, cwd, limits)
    }

    fn output_with_environment(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        environment: &BTreeMap<OsString, OsString>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        (**self).output_with_environment(program, args, cwd, environment, limits)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessCommandOutput;

impl CommandOutputAdapter for ProcessCommandOutput {
    fn output_with_limits(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        let environment = controlled_environment(
            program,
            &BTreeMap::new(),
            CommandEnvironment::Authority,
            std::env::vars_os(),
        );
        let mut terminal_signals = TerminalSignals::new()?;
        run_process(
            program,
            args,
            cwd,
            &environment,
            limits,
            &mut terminal_signals,
        )
    }

    fn output_with_environment(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        environment: &BTreeMap<OsString, OsString>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        let environment = controlled_environment(
            program,
            environment,
            CommandEnvironment::Validation,
            std::env::vars_os(),
        );
        let mut terminal_signals = TerminalSignals::new()?;
        run_process(
            program,
            args,
            cwd,
            &environment,
            limits,
            &mut terminal_signals,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandEnvironment {
    Authority,
    Validation,
}

fn controlled_environment(
    program: &str,
    explicit: &BTreeMap<OsString, OsString>,
    kind: CommandEnvironment,
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
) -> BTreeMap<OsString, OsString> {
    let inherited = inherited.into_iter().collect::<BTreeMap<_, _>>();
    let mut environment = BTreeMap::from([
        (OsString::from("LANG"), OsString::from("C")),
        (OsString::from("LC_ALL"), OsString::from("C")),
        (OsString::from("TZ"), OsString::from("UTC")),
    ]);
    let path = inherited
        .get(std::ffi::OsStr::new("PATH"))
        .and_then(|value| sanitize_path_list(value))
        .unwrap_or_else(|| OsString::from("/run/current-system/sw/bin:/usr/bin:/bin"));
    environment.insert(OsString::from("PATH"), path);
    for variable in ["RUSTUP_HOME"] {
        if let Some(value) = inherited
            .get(std::ffi::OsStr::new(variable))
            .filter(|value| safe_absolute_path(value))
        {
            environment.insert(OsString::from(variable), value.clone());
        }
    }
    if kind == CommandEnvironment::Authority && program == "gh" {
        for variable in ["GH_TOKEN", "GITHUB_TOKEN"] {
            if let Some(value) = inherited
                .get(std::ffi::OsStr::new(variable))
                .filter(|value| !value.is_empty())
            {
                environment.insert(OsString::from(variable), value.clone());
            }
        }
    }
    if kind == CommandEnvironment::Authority && matches!(program, "gh" | "git-town") {
        for variable in ["HOME", "XDG_CONFIG_HOME"] {
            if let Some(value) = inherited
                .get(std::ffi::OsStr::new(variable))
                .filter(|value| safe_absolute_path(value))
            {
                environment.insert(OsString::from(variable), value.clone());
            }
        }
    }
    if kind == CommandEnvironment::Authority
        && program == "gh"
        && let Some(value) = inherited
            .get(std::ffi::OsStr::new("GH_CONFIG_DIR"))
            .filter(|value| safe_absolute_path(value))
    {
        environment.insert(OsString::from("GH_CONFIG_DIR"), value.clone());
    }
    environment.extend(
        explicit
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    environment
}

fn sanitize_path_list(value: &std::ffi::OsStr) -> Option<OsString> {
    let paths = std::env::split_paths(value)
        .filter(|path| safe_path(path))
        .collect::<Vec<_>>();
    (!paths.is_empty())
        .then(|| std::env::join_paths(paths).ok())
        .flatten()
}

fn safe_absolute_path(value: &std::ffi::OsStr) -> bool {
    safe_path(Path::new(value))
}

fn safe_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::ParentDir))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalSignal {
    Interrupt,
    Terminate,
}

trait TerminalSignalSource {
    fn pending(&mut self) -> Option<TerminalSignal>;
}

struct GlobalTerminalSignals {
    interrupt: Arc<AtomicBool>,
    terminate: Arc<AtomicBool>,
    inactive: Arc<AtomicBool>,
}

static TERMINAL_SIGNAL_FLAGS: OnceLock<std::result::Result<GlobalTerminalSignals, String>> =
    OnceLock::new();
static TERMINAL_SIGNAL_OWNER: Mutex<()> = Mutex::new(());
#[cfg(test)]
static TERMINAL_SIGNAL_INITIALIZATIONS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

struct TerminalSignals {
    flags: &'static GlobalTerminalSignals,
    _owner: MutexGuard<'static, ()>,
}

impl TerminalSignals {
    fn new() -> Result<Self> {
        let flags = TERMINAL_SIGNAL_FLAGS.get_or_init(|| {
            #[cfg(test)]
            TERMINAL_SIGNAL_INITIALIZATIONS.fetch_add(1, Ordering::Relaxed);
            let interrupt = Arc::new(AtomicBool::new(false));
            let terminate = Arc::new(AtomicBool::new(false));
            let inactive = Arc::new(AtomicBool::new(true));
            let mut registrations = Vec::new();
            let installed = (|| {
                registrations.push(flag::register_conditional_default(
                    SIGINT,
                    Arc::clone(&inactive),
                )?);
                registrations.push(flag::register(SIGINT, Arc::clone(&interrupt))?);
                registrations.push(flag::register_conditional_default(
                    SIGTERM,
                    Arc::clone(&inactive),
                )?);
                registrations.push(flag::register(SIGTERM, Arc::clone(&terminate))?);
                std::io::Result::Ok(())
            })();
            if let Err(error) = installed {
                for registration in registrations {
                    signal_hook::low_level::unregister(registration);
                }
                return Err(error.to_string());
            }
            Ok(GlobalTerminalSignals {
                interrupt,
                terminate,
                inactive,
            })
        });
        let flags = flags.as_ref().map_err(|error| {
            DeliveryError::new(format!(
                "cannot install terminal signal forwarding: {error}"
            ))
        })?;
        let owner = TERMINAL_SIGNAL_OWNER
            .lock()
            .map_err(|_| DeliveryError::new("terminal signal forwarding owner was poisoned"))?;
        flags.interrupt.store(false, Ordering::Release);
        flags.terminate.store(false, Ordering::Release);
        flags.inactive.store(false, Ordering::Release);
        Ok(Self {
            flags,
            _owner: owner,
        })
    }
}

impl TerminalSignalSource for TerminalSignals {
    fn pending(&mut self) -> Option<TerminalSignal> {
        if self.flags.interrupt.swap(false, Ordering::AcqRel) {
            Some(TerminalSignal::Interrupt)
        } else if self.flags.terminate.swap(false, Ordering::AcqRel) {
            Some(TerminalSignal::Terminate)
        } else {
            None
        }
    }
}

impl Drop for TerminalSignals {
    fn drop(&mut self) {
        self.flags.interrupt.store(false, Ordering::Release);
        self.flags.terminate.store(false, Ordering::Release);
        self.flags.inactive.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StopReason {
    Timeout,
    StdoutOverflow,
    StderrOverflow,
    OutputOverflow,
    Terminal(TerminalSignal),
}

fn run_process<S: TerminalSignalSource>(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    environment: &BTreeMap<OsString, OsString>,
    limits: CommandLimits,
    terminal_signals: &mut S,
) -> Result<CommandOutput> {
    if limits.stdout_bytes == 0
        || limits.stderr_bytes == 0
        || limits.stdout_bytes > MAX_GIT_BLOB_BYTES
        || limits.stderr_bytes > MAX_GIT_BLOB_BYTES
        || limits.timeout.is_zero()
        || limits.timeout > MAX_COMMAND_TIMEOUT
    {
        return Err(DeliveryError::new(
            "command limits are zero or exceed hard process bounds",
        ));
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    command.env_clear().envs(environment);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| DeliveryError::new(format!("could not execute {program}: {error}")))?;
    let process_group = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(|| {
            let _ = child.kill();
            let _ = child.wait();
            DeliveryError::new("child PID cannot identify its process group")
        })?;
    let pidfd =
        match rustix::process::pidfd_open(process_group, rustix::process::PidfdFlags::empty()) {
            Ok(pidfd) => pidfd,
            Err(error) => {
                kill_group_and_reap(&mut child, process_group);
                return Err(DeliveryError::new(format!(
                    "cannot obtain race-free child process authority: {error}"
                )));
            }
        };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            kill_group_and_reap(&mut child, process_group);
            return Err(DeliveryError::new("child stdout pipe was not available"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            kill_group_and_reap(&mut child, process_group);
            return Err(DeliveryError::new("child stderr pipe was not available"));
        }
    };
    let stdout_overflow = Arc::new(AtomicBool::new(false));
    let stderr_overflow = Arc::new(AtomicBool::new(false));
    let cancel_readers = Arc::new(AtomicBool::new(false));
    let (stdout_reader, stdout_rx) = spawn_capped_reader(
        stdout,
        limits.stdout_bytes,
        &stdout_overflow,
        &cancel_readers,
    );
    let (stderr_reader, stderr_rx) = spawn_capped_reader(
        stderr,
        limits.stderr_bytes,
        &stderr_overflow,
        &cancel_readers,
    );

    let mut stop_reason = None;
    let mut force_kill_at = None;
    let mut cancel_readers_at = None;
    let mut leader_exited = false;
    let mut stdout_result = None;
    let mut stderr_result = None;
    let supervision = loop {
        if let Err(error) = receive_reader(&stdout_rx, &mut stdout_result) {
            break Err(error);
        }
        if let Err(error) = receive_reader(&stderr_rx, &mut stderr_result) {
            break Err(error);
        }
        if !leader_exited {
            match observe_child_exit(&pidfd) {
                Ok(exited) => leader_exited = exited,
                Err(error) => {
                    break Err(DeliveryError::new(format!(
                        "could not observe {program}: {error}"
                    )));
                }
            }
        }

        let now = Instant::now();
        if stop_reason.is_none() {
            let stdout_exceeded = stdout_overflow.load(Ordering::Acquire);
            let stderr_exceeded = stderr_overflow.load(Ordering::Acquire);
            let reason = terminal_signals
                .pending()
                .map(StopReason::Terminal)
                .or_else(|| (started.elapsed() >= limits.timeout).then_some(StopReason::Timeout))
                .or(match (stdout_exceeded, stderr_exceeded) {
                    (true, true) => Some(StopReason::OutputOverflow),
                    (true, false) => Some(StopReason::StdoutOverflow),
                    (false, true) => Some(StopReason::StderrOverflow),
                    (false, false) => None,
                });
            if let Some(reason) = reason {
                stop_reason = Some(reason);
                match reason {
                    StopReason::Terminal(signal) => {
                        let _ = rustix::process::kill_process_group(
                            process_group,
                            rustix_signal(signal),
                        );
                        force_kill_at = Some(now + TERMINATION_GRACE);
                    }
                    _ => {
                        let _ = rustix::process::kill_process_group(
                            process_group,
                            rustix::process::Signal::Kill,
                        );
                        cancel_readers_at = Some(now + OUTPUT_DRAIN_GRACE);
                    }
                }
            }
        }
        if force_kill_at.is_some_and(|deadline| now >= deadline) {
            let _ =
                rustix::process::kill_process_group(process_group, rustix::process::Signal::Kill);
            force_kill_at = None;
            cancel_readers_at = Some(now + OUTPUT_DRAIN_GRACE);
        }
        if cancel_readers_at.is_some_and(|deadline| now >= deadline) {
            cancel_readers.store(true, Ordering::Release);
            cancel_readers_at = None;
        }

        if leader_exited && stdout_result.is_some() && stderr_result.is_some() {
            // A completed leader may have left descendants which closed their
            // output descriptors. Clean the still-pinned group before reaping.
            let _ =
                rustix::process::kill_process_group(process_group, rustix::process::Signal::Kill);
            break Ok(());
        }
        thread::sleep(Duration::from_millis(5));
    };
    if let Err(error) = supervision {
        cancel_readers.store(true, Ordering::Release);
        kill_group_and_reap(&mut child, process_group);
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        return Err(error);
    }

    // The leader remains an unreaped zombie until here. Its PID, and therefore
    // the process-group ID, cannot be reused while any group signal is possible.
    let status = child
        .wait()
        .map_err(|error| DeliveryError::new(format!("could not reap {program}: {error}")))?;

    let stdout = stdout_result.expect("reader completion was checked")?;
    let stderr = stderr_result.expect("reader completion was checked")?;
    stdout_reader
        .join()
        .map_err(|_| DeliveryError::new("stdout reader panicked"))?;
    stderr_reader
        .join()
        .map_err(|_| DeliveryError::new("stderr reader panicked"))?;
    let exit_code = match stop_reason {
        Some(StopReason::Timeout) => Some(EXIT_COMMAND_TIMEOUT),
        Some(StopReason::StdoutOverflow) => Some(EXIT_STDOUT_OVERFLOW),
        Some(StopReason::StderrOverflow) => Some(EXIT_STDERR_OVERFLOW),
        Some(StopReason::OutputOverflow) => Some(EXIT_OUTPUT_OVERFLOW),
        Some(StopReason::Terminal(TerminalSignal::Interrupt)) => Some(EXIT_INTERRUPTED),
        Some(StopReason::Terminal(TerminalSignal::Terminate)) => Some(EXIT_TERMINATED),
        None => status.code(),
    };
    Ok(CommandOutput {
        success: stop_reason.is_none() && status.success(),
        exit_code,
        stdout,
        stderr,
    })
}

fn observe_child_exit(pidfd: &OwnedFd) -> rustix::io::Result<bool> {
    rustix::process::waitid(
        rustix::process::WaitId::PidFd(pidfd.as_fd()),
        rustix::process::WaitidOptions::EXITED
            | rustix::process::WaitidOptions::NOHANG
            | rustix::process::WaitidOptions::NOWAIT,
    )
    .map(|status| status.is_some())
}

fn kill_group_and_reap(child: &mut Child, process_group: rustix::process::Pid) {
    let _ = rustix::process::kill_process_group(process_group, rustix::process::Signal::Kill);
    let _ = child.wait();
}

fn rustix_signal(signal: TerminalSignal) -> rustix::process::Signal {
    match signal {
        TerminalSignal::Interrupt => rustix::process::Signal::Int,
        TerminalSignal::Terminate => rustix::process::Signal::Term,
    }
}

fn spawn_capped_reader<R: Read + AsFd + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflow: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
) -> (thread::JoinHandle<()>, Receiver<Result<Vec<u8>>>) {
    let overflow = Arc::clone(overflow);
    let cancel = Arc::clone(cancel);
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let result = (|| {
            let flags = rustix::fs::fcntl_getfl(&reader).map_err(|error| {
                DeliveryError::new(format!("cannot inspect child output pipe: {error}"))
            })?;
            rustix::fs::fcntl_setfl(&reader, flags | rustix::fs::OFlags::NONBLOCK).map_err(
                |error| {
                    DeliveryError::new(format!(
                        "cannot make child output pipe nonblocking: {error}"
                    ))
                },
            )?;
            let mut output = Vec::with_capacity(limit.min(64 * 1024));
            let mut buffer = [0_u8; 8192];
            loop {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                let poll_fd = rustix::event::PollFd::new(&reader, rustix::event::PollFlags::IN);
                match rustix::event::poll(&mut [poll_fd], OUTPUT_POLL_MILLISECONDS) {
                    Ok(0) => continue,
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => {
                        return Err(DeliveryError::new(format!(
                            "cannot poll child output: {error}"
                        )));
                    }
                }
                let read = match reader.read(&mut buffer) {
                    Ok(read) => read,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(error) => {
                        return Err(DeliveryError::new(format!(
                            "cannot read child output: {error}"
                        )));
                    }
                };
                if read == 0 {
                    break;
                }
                let remaining = limit.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..read.min(remaining)]);
                if read > remaining {
                    overflow.store(true, Ordering::Release);
                    break;
                }
            }
            Ok(output)
        })();
        let _ = tx.send(result);
    });
    (handle, rx)
}

fn receive_reader(
    receiver: &Receiver<Result<Vec<u8>>>,
    slot: &mut Option<Result<Vec<u8>>>,
) -> Result<()> {
    if slot.is_some() {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(result) => {
            *slot = Some(result);
            Ok(())
        }
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(DeliveryError::new(
            "child output reader disconnected before completion",
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StackCapability {
    pub tool: String,
    pub version: String,
    pub supported_major: u64,
    pub non_interactive_propose: bool,
    pub github_authenticated: bool,
    pub repository_readable: bool,
    pub ordinary_pull_request_api: bool,
}

#[derive(Debug, Deserialize)]
struct CapabilityGraphQlEnvelope {
    data: CapabilityGraphQlData,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CapabilityGraphQlData {
    repository: Option<CapabilityRepository>,
}

#[derive(Debug, Deserialize)]
struct CapabilityRepository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    #[serde(rename = "pullRequests")]
    pull_requests: CapabilityPullRequests,
}

#[derive(Debug, Deserialize)]
struct CapabilityPullRequests {
    nodes: Vec<CapabilityPullRequest>,
}

#[derive(Debug, Deserialize)]
struct CapabilityPullRequest {
    number: u64,
    state: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
}

const CAPABILITY_QUERY: &str = r#"query($owner:String!,$name:String!){
  repository(owner:$owner,name:$name){
    nameWithOwner
    pullRequests(first:1,orderBy:{field:UPDATED_AT,direction:DESC}){
      nodes{number state baseRefName headRefName headRefOid}
    }
  }
}"#;

pub fn check_git_town_capability<A: CommandOutputAdapter>(
    command: &A,
    repository: &str,
) -> Result<StackCapability> {
    validate_repository_slug(repository)?;
    let version = command
        .output("git-town", &["--version".to_owned()], None)
        .map_err(|_| DeliveryError::new("Git Town is unavailable"))?;
    if !version.success {
        return Err(DeliveryError::new("Git Town is unavailable"));
    }
    let version = String::from_utf8(version.stdout)
        .map_err(|_| DeliveryError::new("Git Town version output is not UTF-8"))?;
    let version = version
        .trim()
        .strip_prefix("Git Town ")
        .ok_or_else(|| DeliveryError::new("Git Town version output is not recognized"))?;
    let parts = version.split('.').collect::<Vec<_>>();
    let major = parts
        .first()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| DeliveryError::new("Git Town version output is not recognized"))?;
    if major != GIT_TOWN_SUPPORTED_MAJOR
        || parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(DeliveryError::new(format!(
            "unsupported Git Town version; required major is {GIT_TOWN_SUPPORTED_MAJOR}"
        )));
    }

    let propose_help = command
        .output(
            "git-town",
            &["propose".to_owned(), "--help".to_owned()],
            None,
        )
        .map_err(|_| DeliveryError::new("Git Town propose capability is unavailable"))?;
    if !propose_help.success {
        return Err(DeliveryError::new(
            "Git Town propose capability is unavailable",
        ));
    }
    let propose_help = String::from_utf8(propose_help.stdout)
        .map_err(|_| DeliveryError::new("Git Town propose help is not UTF-8"))?;
    for flag in ["--stack", "--non-interactive", "--no-browser"] {
        if !propose_help.contains(flag) {
            return Err(DeliveryError::new(format!(
                "Git Town propose does not expose required {flag} behavior"
            )));
        }
    }

    let auth = command
        .output(
            "gh",
            &[
                "auth".to_owned(),
                "status".to_owned(),
                "--hostname".to_owned(),
                "github.com".to_owned(),
            ],
            None,
        )
        .map_err(|_| DeliveryError::new("GitHub authentication is unavailable"))?;
    if !auth.success {
        return Err(DeliveryError::new("GitHub authentication is unavailable"));
    }

    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| DeliveryError::new("invalid GitHub repository identity"))?;
    let response = command.output(
        "gh",
        &[
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            format!("query={CAPABILITY_QUERY}"),
            "-f".to_owned(),
            format!("owner={owner}"),
            "-f".to_owned(),
            format!("name={name}"),
        ],
        None,
    )?;
    if !response.success {
        return Err(command_failed(
            "GitHub ordinary pull-request API capability query failed",
            &response,
        ));
    }
    let response: CapabilityGraphQlEnvelope = serde_json::from_slice(&response.stdout)
        .map_err(|_| DeliveryError::new("GitHub capability response is invalid"))?;
    if !response.errors.is_empty() {
        return Err(DeliveryError::new(
            "GitHub capability response contains partial GraphQL errors",
        ));
    }
    let observed = response
        .data
        .repository
        .ok_or_else(|| DeliveryError::new("GitHub repository is unavailable"))?;
    if !observed.name_with_owner.eq_ignore_ascii_case(repository) {
        return Err(DeliveryError::new(
            "GitHub repository capability identity does not match",
        ));
    }
    for pr in &observed.pull_requests.nodes {
        if pr.number == 0 || !matches!(pr.state.as_str(), "OPEN" | "CLOSED" | "MERGED") {
            return Err(DeliveryError::new(
                "GitHub ordinary pull-request capability data is invalid",
            ));
        }
        validate_git_ref(&pr.base_ref_name, "GitHub capability base ref")?;
        validate_git_ref(&pr.head_ref_name, "GitHub capability head ref")?;
        validate_hash(&pr.head_ref_oid, "GitHub capability head OID")?;
    }
    Ok(StackCapability {
        tool: "git-town".to_owned(),
        version: version.to_owned(),
        supported_major: GIT_TOWN_SUPPORTED_MAJOR,
        non_interactive_propose: true,
        github_authenticated: true,
        repository_readable: true,
        ordinary_pull_request_api: true,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedBlob {
    pub oid: String,
    pub mode: String,
    pub bytes: Vec<u8>,
}

pub trait RepositoryProbe {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf>;
    fn repository_identity(&self, root: &Path) -> Result<String>;
    fn git_common_dir(&self, root: &Path) -> Result<PathBuf>;
    fn object_format(&self, root: &Path) -> Result<GitObjectFormat>;
    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String>;
    fn tree_for_commit(&self, root: &Path, commit_oid: &str) -> Result<String>;
    fn is_dirty(&self, root: &Path) -> Result<bool>;
    fn is_ancestor(&self, root: &Path, ancestor: &str, descendant: &str) -> Result<bool>;
    fn tracked_blob(&self, root: &Path, commit_oid: &str, path: &Path) -> Result<TrackedBlob>;
    fn canonical_diff(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<u8>>;
    fn prospective_merge_tree(&self, root: &Path, base_oid: &str, head_oid: &str)
    -> Result<String>;
}

#[derive(Clone, Debug)]
pub struct GitProbe<A> {
    command: A,
}

impl<A> GitProbe<A> {
    pub fn new(command: A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> GitProbe<A> {
    fn git_output(
        &self,
        root: &Path,
        arguments: &[String],
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        let root_string = path_string(root)?;
        let mut args = vec!["-C".to_owned(), root_string];
        args.extend_from_slice(arguments);
        self.command.output_with_limits("git", &args, None, limits)
    }

    fn git_stdout(&self, root: &Path, arguments: &[String]) -> Result<String> {
        let output = self.git_output(root, arguments, CommandLimits::default())?;
        if !output.success {
            return Err(command_failed(
                format!("git {} failed", arguments.join(" ")),
                &output,
            ));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| DeliveryError::new("git output was not UTF-8"))?;
        Ok(stdout.trim().to_owned())
    }
}

impl<A: CommandOutputAdapter> RepositoryProbe for GitProbe<A> {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf> {
        reject_symlink_components(root)?;
        let canonical = fs::canonicalize(root).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize repository root {}: {error}",
                root.display()
            ))
        })?;
        let reported = self.git_stdout(
            &canonical,
            &["rev-parse".to_owned(), "--show-toplevel".to_owned()],
        )?;
        reject_symlink_components(Path::new(&reported))?;
        let reported = fs::canonicalize(reported).map_err(|error| {
            DeliveryError::new(format!("cannot canonicalize Git root: {error}"))
        })?;
        if reported != canonical {
            return Err(DeliveryError::new(format!(
                "{} is not the repository root",
                root.display()
            )));
        }
        Ok(canonical)
    }

    fn repository_identity(&self, root: &Path) -> Result<String> {
        let remote = self.git_stdout(
            root,
            &[
                "config".to_owned(),
                "--get".to_owned(),
                "remote.origin.url".to_owned(),
            ],
        )?;
        parse_repository_identity(&remote)
    }

    fn git_common_dir(&self, root: &Path) -> Result<PathBuf> {
        let value = self.git_stdout(
            root,
            &["rev-parse".to_owned(), "--git-common-dir".to_owned()],
        )?;
        let path = PathBuf::from(value);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        reject_symlink_components(&path)?;
        fs::canonicalize(&path).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize Git common directory {}: {error}",
                path.display()
            ))
        })
    }

    fn object_format(&self, root: &Path) -> Result<GitObjectFormat> {
        match self
            .git_stdout(
                root,
                &[
                    "rev-parse".to_owned(),
                    "--show-object-format=storage".to_owned(),
                ],
            )?
            .as_str()
        {
            "sha1" => Ok(GitObjectFormat::Sha1),
            "sha256" => Ok(GitObjectFormat::Sha256),
            value => Err(DeliveryError::new(format!(
                "unsupported Git object format {value}"
            ))),
        }
    }

    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String> {
        validate_git_ref(revision, "Git revision")?;
        let oid = self.git_stdout(
            root,
            &[
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--end-of-options".to_owned(),
                format!("{revision}^{{commit}}"),
            ],
        )?;
        validate_hash(&oid, "resolved commit")?;
        Ok(oid)
    }

    fn tree_for_commit(&self, root: &Path, commit_oid: &str) -> Result<String> {
        validate_hash(commit_oid, "commit OID")?;
        let tree = self.git_stdout(
            root,
            &[
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--end-of-options".to_owned(),
                format!("{commit_oid}^{{tree}}"),
            ],
        )?;
        validate_hash(&tree, "resolved tree")?;
        Ok(tree)
    }

    fn is_dirty(&self, root: &Path) -> Result<bool> {
        Ok(!self
            .git_stdout(
                root,
                &[
                    "status".to_owned(),
                    "--porcelain=v1".to_owned(),
                    "--untracked-files=normal".to_owned(),
                ],
            )?
            .is_empty())
    }

    fn is_ancestor(&self, root: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
        validate_hash(ancestor, "ancestor commit")?;
        validate_hash(descendant, "descendant commit")?;
        let output = self.git_output(
            root,
            &[
                "merge-base".to_owned(),
                "--is-ancestor".to_owned(),
                ancestor.to_owned(),
                descendant.to_owned(),
            ],
            CommandLimits::default(),
        )?;
        match output.exit_code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(command_failed(
                "git merge-base --is-ancestor failed",
                &output,
            )),
        }
    }

    fn tracked_blob(&self, root: &Path, commit_oid: &str, path: &Path) -> Result<TrackedBlob> {
        validate_hash(commit_oid, "blob commit")?;
        super::model::validate_repo_relative_path(path)?;
        let path_string = path_string(path)?;
        let listing = self.git_output(
            root,
            &[
                "ls-tree".to_owned(),
                "-z".to_owned(),
                commit_oid.to_owned(),
                "--".to_owned(),
                format!(":(literal){path_string}"),
            ],
            CommandLimits::default(),
        )?;
        if !listing.success {
            return Err(command_failed(
                format!("cannot inspect tracked blob {path_string}"),
                &listing,
            ));
        }
        let entries = listing
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>();
        if entries.len() != 1 {
            return Err(DeliveryError::new(format!(
                "tracked fingerprint path must resolve to exactly one Git blob: {path_string}"
            )));
        }
        let entry = std::str::from_utf8(entries[0])
            .map_err(|_| DeliveryError::new("git ls-tree output was not UTF-8"))?;
        let (metadata, listed_path) = entry
            .split_once('\t')
            .ok_or_else(|| DeliveryError::new("malformed git ls-tree output"))?;
        if listed_path != path_string {
            return Err(DeliveryError::new(
                "git ls-tree returned a different fingerprint path",
            ));
        }
        let fields = metadata.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") || fields[1] != "blob" {
            return Err(DeliveryError::new(format!(
                "fingerprint path is not a tracked regular Git blob: {path_string}"
            )));
        }
        validate_hash(fields[2], "tracked blob OID")?;
        let payload = self.git_output(
            root,
            &[
                "cat-file".to_owned(),
                "blob".to_owned(),
                fields[2].to_owned(),
            ],
            CommandLimits {
                stdout_bytes: MAX_GIT_BLOB_BYTES,
                stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
                timeout: DEFAULT_COMMAND_TIMEOUT,
            },
        )?;
        if !payload.success {
            return Err(command_failed(
                format!("cannot read tracked Git blob for {path_string}"),
                &payload,
            ));
        }
        Ok(TrackedBlob {
            oid: fields[2].to_owned(),
            mode: fields[0].to_owned(),
            bytes: payload.stdout,
        })
    }

    fn canonical_diff(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<u8>> {
        validate_hash(base_oid, "diff base OID")?;
        validate_hash(head_oid, "diff head OID")?;
        let mut arguments = vec![
            "diff".to_owned(),
            "--binary".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-renames".to_owned(),
            "--full-index".to_owned(),
            "--src-prefix=a/".to_owned(),
            "--dst-prefix=b/".to_owned(),
            base_oid.to_owned(),
            head_oid.to_owned(),
        ];
        if !paths.is_empty() {
            arguments.push("--".to_owned());
            let mut normalized = paths.to_vec();
            normalized.sort();
            normalized.dedup();
            for path in normalized {
                super::model::validate_repo_relative_path(&path)?;
                arguments.push(format!(":(literal){}", path_string(&path)?));
            }
        }
        let output = self.git_output(
            root,
            &arguments,
            CommandLimits {
                stdout_bytes: MAX_GIT_BLOB_BYTES,
                stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
                timeout: DEFAULT_COMMAND_TIMEOUT,
            },
        )?;
        if !output.success {
            return Err(command_failed(
                "cannot compute canonical base-to-head Git diff",
                &output,
            ));
        }
        Ok(output.stdout)
    }

    fn prospective_merge_tree(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
    ) -> Result<String> {
        validate_hash(base_oid, "merge base OID")?;
        validate_hash(head_oid, "merge head OID")?;
        let output = self.git_output(
            root,
            &[
                "merge-tree".to_owned(),
                "--write-tree".to_owned(),
                base_oid.to_owned(),
                head_oid.to_owned(),
            ],
            CommandLimits::default(),
        )?;
        if !output.success {
            return Err(command_failed(
                "prospective merge tree has conflicts or could not be computed",
                &output,
            ));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| DeliveryError::new("git merge-tree output was not UTF-8"))?;
        let tree = stdout
            .lines()
            .next()
            .ok_or_else(|| DeliveryError::new("git merge-tree returned no tree OID"))?
            .trim()
            .to_owned();
        validate_hash(&tree, "prospective merge tree")?;
        Ok(tree)
    }
}

pub trait StackGraphSource {
    fn graph(
        &self,
        repository: &str,
        checkout_root: &Path,
        expected_nodes: &[StackNodePolicy],
    ) -> Result<StackGraph>;
}

#[derive(Debug)]
pub struct GitTownStackSource<'a, A> {
    command: &'a A,
}

impl<'a, A> GitTownStackSource<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> StackGraphSource for GitTownStackSource<'_, A> {
    fn graph(
        &self,
        repository: &str,
        checkout_root: &Path,
        expected_nodes: &[StackNodePolicy],
    ) -> Result<StackGraph> {
        validate_repository_id(repository)?;
        let slug = repository
            .strip_prefix("github.com/")
            .ok_or_else(|| DeliveryError::new("stack repository is not hosted by GitHub"))?;
        validate_repository_slug(slug)?;
        if expected_nodes.is_empty() {
            return Err(DeliveryError::new(
                "authoritative stack policy has no nodes for repository",
            ));
        }
        let mut expected_branches = BTreeSet::new();
        let mut expected_prs = BTreeSet::new();
        for node in expected_nodes {
            if node.repository != repository {
                return Err(DeliveryError::new(
                    "authoritative stack policy contains a node for another repository",
                ));
            }
            validate_git_ref(&node.branch, "authoritative stack branch")?;
            if node.pr_number == 0
                || !expected_branches.insert(node.branch.as_str())
                || !expected_prs.insert(node.pr_number)
            {
                return Err(DeliveryError::new(
                    "authoritative stack policy repeats or invalidates a branch or PR",
                ));
            }
        }
        self.reject_ambiguous_worktree(checkout_root)?;

        let current_branch = self.command_text(
            "git",
            &[
                "symbolic-ref".to_owned(),
                "--quiet".to_owned(),
                "--short".to_owned(),
                "HEAD".to_owned(),
            ],
            checkout_root,
            "cannot resolve the current stack branch",
        )?;
        validate_git_ref(&current_branch, "Git Town current branch")?;
        let trunk = self.command_text(
            "git",
            &[
                "config".to_owned(),
                "--get".to_owned(),
                "git-town.main-branch".to_owned(),
            ],
            checkout_root,
            "Git Town main branch is not configured",
        )?;
        validate_git_ref(&trunk, "Git Town main branch")?;
        if current_branch == trunk {
            return Err(DeliveryError::new(
                "Git Town current branch is the main branch, not a stack top",
            ));
        }

        let mut statuses = Vec::with_capacity(expected_nodes.len());
        let mut saw_active = false;
        for node in expected_nodes {
            let status = GhStatusSource::new(self.command).status(repository, node.pr_number)?;
            if status.number != node.pr_number
                || status.repository != repository
                || status.head_repository != repository
                || status.head_ref != node.branch
            {
                return Err(DeliveryError::new(format!(
                    "ordinary GitHub PR identity does not match configured branch {}",
                    node.branch
                )));
            }
            if status.is_in_merge_queue {
                return Err(DeliveryError::new(format!(
                    "Git Town branch {} is queued",
                    node.branch
                )));
            }
            if status.state == PullRequestState::Open
                && matches!(status.merge_state.as_str(), "BEHIND" | "DIRTY" | "UNKNOWN")
            {
                return Err(DeliveryError::new(format!(
                    "Git Town branch {} has ambiguous or stale merge state",
                    node.branch
                )));
            }
            if !matches!(
                status.merge_state.as_str(),
                "BLOCKED" | "CLEAN" | "DRAFT" | "HAS_HOOKS" | "UNSTABLE"
            ) && status.state == PullRequestState::Open
            {
                return Err(DeliveryError::new(
                    "GitHub returned an unsupported merge state",
                ));
            }
            match status.state {
                PullRequestState::Open => saw_active = true,
                PullRequestState::Merged if saw_active => {
                    return Err(DeliveryError::new(
                        "Git Town stack has a merged node after an active node",
                    ));
                }
                PullRequestState::Merged => {}
                PullRequestState::Closed => {
                    return Err(DeliveryError::new(format!(
                        "Git Town branch {} has a closed unmerged PR",
                        node.branch
                    )));
                }
            }
            statuses.push(status);
        }
        let first_active = statuses
            .iter()
            .position(|status| status.state == PullRequestState::Open)
            .ok_or_else(|| DeliveryError::new("Git Town stack has no active top branch"))?;
        let active_nodes = &expected_nodes[first_active..];
        if active_nodes
            .last()
            .is_none_or(|node| node.branch != current_branch)
        {
            return Err(DeliveryError::new(
                "Git Town current branch is not the configured active stack top",
            ));
        }

        let mut lineage_branch = current_branch.clone();
        let mut seen = BTreeSet::new();
        for (offset, node) in active_nodes.iter().enumerate().rev() {
            if lineage_branch != node.branch || !seen.insert(lineage_branch.clone()) {
                return Err(DeliveryError::new(
                    "Git Town active lineage is reordered or cyclic",
                ));
            }
            let parent = self.command_text(
                "git-town",
                &[
                    "config".to_owned(),
                    "get-parent".to_owned(),
                    lineage_branch.clone(),
                ],
                checkout_root,
                "Git Town parent configuration is missing or unreadable",
            )?;
            validate_git_ref(&parent, "Git Town parent branch")?;
            if parent == lineage_branch {
                return Err(DeliveryError::new(
                    "Git Town parent configuration contains a self-cycle",
                ));
            }
            let expected_parent = if offset == 0 {
                statuses[first_active].base_ref.as_str()
            } else {
                active_nodes[offset - 1].branch.as_str()
            };
            if parent != expected_parent {
                return Err(DeliveryError::new(format!(
                    "Git Town active parent for {} is {}, expected {}",
                    node.branch, parent, expected_parent
                )));
            }
            lineage_branch = parent;
        }
        if lineage_branch != statuses[first_active].base_ref {
            return Err(DeliveryError::new(
                "Git Town active lineage contains an extra branch",
            ));
        }

        for index in 0..first_active {
            let status = &statuses[index];
            let node = &expected_nodes[index];
            let historical_base = status.merge_base_oid.as_deref().ok_or_else(|| {
                DeliveryError::new(format!(
                    "merged stack node {} has no historical merge base",
                    node.branch
                ))
            })?;
            if index == 0 {
                if status.base_ref != trunk || status.base_oid != historical_base {
                    return Err(DeliveryError::new(
                        "bottom merged stack node does not record its exact trunk base",
                    ));
                }
                continue;
            }
            let previous_node = &expected_nodes[index - 1];
            let previous_status = &statuses[index - 1];
            let previous_merge = previous_status.merge_commit_oid.as_deref().ok_or_else(|| {
                DeliveryError::new("merged stack prefix is missing prior merge commit authority")
            })?;
            let historical_branch_base = status.base_ref == previous_node.branch
                && status.base_oid == previous_status.head_oid
                && historical_base == previous_status.head_oid;
            let exact_post_merge_retarget = status.base_ref == trunk
                && status.base_oid == previous_merge
                && historical_base == previous_merge;
            if !historical_branch_base && !exact_post_merge_retarget {
                return Err(DeliveryError::new(format!(
                    "merged stack node {} has no exact historical continuity with {}",
                    node.branch, previous_node.branch
                )));
            }
        }
        if first_active > 0 {
            let previous_merge = statuses[first_active - 1]
                .merge_commit_oid
                .as_deref()
                .ok_or_else(|| {
                    DeliveryError::new(
                        "merged stack prefix is missing final merge commit authority",
                    )
                })?;
            if statuses[first_active].base_ref != trunk
                || statuses[first_active].base_oid != previous_merge
            {
                return Err(DeliveryError::new(
                    "first active stack node was not retargeted to exact trunk after the merged prefix",
                ));
            }
        }

        let mut expected_live_base = trunk.as_str();
        for (index, status) in statuses.iter().enumerate().skip(first_active) {
            if status.base_ref != expected_live_base {
                return Err(DeliveryError::new(format!(
                    "ordinary GitHub PR base for {} does not match the active Git Town topology",
                    expected_nodes[index].branch
                )));
            }
            expected_live_base = &expected_nodes[index].branch;
        }

        let mut branches = Vec::with_capacity(expected_nodes.len());
        let mut topology_parent = trunk.as_str();
        let active_trunk_oid = statuses[first_active].base_oid.clone();
        for (index, (node, status)) in expected_nodes.iter().zip(statuses).enumerate() {
            let is_merged = status.state == PullRequestState::Merged;
            let base = if is_merged {
                status.merge_base_oid.clone().ok_or_else(|| {
                    DeliveryError::new(format!(
                        "merged stack node {} has no historical merge base",
                        node.branch
                    ))
                })?
            } else {
                status.base_oid.clone()
            };
            if is_merged {
                self.verify_merge_authority(
                    checkout_root,
                    &base,
                    &status.head_oid,
                    status.merge_commit_oid.as_deref(),
                    status.merge_commit_tree_oid.as_deref(),
                    &node.branch,
                )?;
                self.verify_ancestor_oids(
                    checkout_root,
                    status
                        .merge_commit_oid
                        .as_deref()
                        .expect("merged authority was required"),
                    &active_trunk_oid,
                    &format!(
                        "active trunk before {}",
                        expected_nodes[first_active].branch
                    ),
                )?;
            } else {
                let local_head = self.resolve_local_oid(checkout_root, &node.branch)?;
                let local_base = self.resolve_local_oid(checkout_root, &status.base_ref)?;
                if status.head_oid != local_head || status.base_oid != local_base {
                    return Err(DeliveryError::new(format!(
                        "ordinary GitHub PR OIDs for {} do not match exact local refs",
                        node.branch
                    )));
                }
            }
            self.verify_ancestor_oids(checkout_root, &base, &status.head_oid, &node.branch)?;
            branches.push(StackBranch {
                name: node.branch.clone(),
                parent: topology_parent.to_owned(),
                base_ref: status.base_ref,
                observed_base: status.base_oid,
                head: status.head_oid,
                base,
                is_current: index + 1 == expected_nodes.len(),
                is_merged,
                is_queued: false,
                needs_rebase: false,
                pr: Some(StackPr {
                    number: node.pr_number,
                    url: format!("https://github.com/{slug}/pull/{}", node.pr_number),
                    state: if is_merged { "MERGED" } else { "OPEN" }.to_owned(),
                }),
                merge_commit_oid: status.merge_commit_oid,
                merge_commit_tree_oid: status.merge_commit_tree_oid,
            });
            topology_parent = &node.branch;
        }
        let graph = StackGraph {
            trunk,
            current_branch,
            branches,
        };
        graph.validate()?;
        Ok(graph)
    }
}

impl<A: CommandOutputAdapter> GitTownStackSource<'_, A> {
    fn command_text(
        &self,
        program: &str,
        arguments: &[String],
        checkout_root: &Path,
        failure: &str,
    ) -> Result<String> {
        let output = self
            .command
            .output(program, arguments, Some(checkout_root))?;
        if !output.success {
            return Err(command_failed(failure, &output));
        }
        let value = String::from_utf8(output.stdout)
            .map_err(|_| DeliveryError::new(format!("{failure}: output is not UTF-8")))?
            .trim()
            .to_owned();
        if value.is_empty() || value.contains('\n') || value.contains('\0') {
            return Err(DeliveryError::new(format!(
                "{failure}: output is missing or ambiguous"
            )));
        }
        Ok(value)
    }

    fn resolve_local_oid(&self, checkout_root: &Path, branch: &str) -> Result<String> {
        validate_git_ref(branch, "Git Town local branch")?;
        let oid = self.command_text(
            "git",
            &[
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--end-of-options".to_owned(),
                format!("{branch}^{{commit}}"),
            ],
            checkout_root,
            "Git Town branch is missing locally",
        )?;
        validate_hash(&oid, "Git Town local branch OID")?;
        Ok(oid)
    }

    fn verify_ancestor_oids(
        &self,
        checkout_root: &Path,
        base_oid: &str,
        head_oid: &str,
        branch: &str,
    ) -> Result<()> {
        let output = self.command.output(
            "git",
            &[
                "merge-base".to_owned(),
                "--is-ancestor".to_owned(),
                base_oid.to_owned(),
                head_oid.to_owned(),
            ],
            Some(checkout_root),
        )?;
        if !output.success {
            return Err(DeliveryError::new(format!(
                "Git Town content base is not an ancestor of {branch}"
            )));
        }
        Ok(())
    }

    fn verify_merge_authority(
        &self,
        checkout_root: &Path,
        base_oid: &str,
        head_oid: &str,
        merge_commit_oid: Option<&str>,
        merge_tree_oid: Option<&str>,
        branch: &str,
    ) -> Result<()> {
        let commit = merge_commit_oid.ok_or_else(|| {
            DeliveryError::new(format!(
                "merged Git Town branch {branch} has no merge commit authority"
            ))
        })?;
        let tree = merge_tree_oid.ok_or_else(|| {
            DeliveryError::new(format!(
                "merged Git Town branch {branch} has no merge tree authority"
            ))
        })?;
        validate_hash(commit, "GitHub merge commit OID")?;
        validate_hash(tree, "GitHub merge commit tree OID")?;
        let observed_tree = self.command_text(
            "git",
            &[
                "show".to_owned(),
                "-s".to_owned(),
                "--format=%T".to_owned(),
                commit.to_owned(),
            ],
            checkout_root,
            "cannot resolve the GitHub merge commit tree",
        )?;
        validate_hash(&observed_tree, "local merge commit tree OID")?;
        if observed_tree != tree {
            return Err(DeliveryError::new(format!(
                "merged Git Town branch {branch} has forged merge commit/tree authority"
            )));
        }
        let observed_parents = self.command_text(
            "git",
            &[
                "show".to_owned(),
                "-s".to_owned(),
                "--format=%P".to_owned(),
                commit.to_owned(),
            ],
            checkout_root,
            "cannot resolve the GitHub merge commit parents",
        )?;
        let observed_parents = observed_parents
            .split_ascii_whitespace()
            .collect::<Vec<_>>();
        if observed_parents.is_empty()
            || observed_parents.len() > 2
            || observed_parents[0] != base_oid
            || (observed_parents.len() == 2 && observed_parents[1] != head_oid)
        {
            return Err(DeliveryError::new(format!(
                "merged Git Town branch {branch} has forged merge parent authority"
            )));
        }
        let prospective_tree = self.command_text(
            "git",
            &[
                "merge-tree".to_owned(),
                "--write-tree".to_owned(),
                base_oid.to_owned(),
                head_oid.to_owned(),
            ],
            checkout_root,
            "cannot reproduce the historical GitHub merge tree",
        )?;
        validate_hash(&prospective_tree, "historical prospective merge tree OID")?;
        if prospective_tree != tree {
            return Err(DeliveryError::new(format!(
                "merged Git Town branch {branch} merge tree differs from its exact historical base/head merge"
            )));
        }
        Ok(())
    }

    fn reject_ambiguous_worktree(&self, checkout_root: &Path) -> Result<()> {
        let status = self.command.output(
            "git",
            &[
                "status".to_owned(),
                "--porcelain=v1".to_owned(),
                "-z".to_owned(),
                "--untracked-files=normal".to_owned(),
            ],
            Some(checkout_root),
        )?;
        if !status.success {
            return Err(command_failed("cannot inspect stack worktree", &status));
        }
        if !status.stdout.is_empty() {
            return Err(DeliveryError::new(
                "Git Town stack worktree is dirty or ambiguous",
            ));
        }
        for state in [
            "rebase-merge",
            "rebase-apply",
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
        ] {
            let path = self.command_text(
                "git",
                &[
                    "rev-parse".to_owned(),
                    "--git-path".to_owned(),
                    state.to_owned(),
                ],
                checkout_root,
                "cannot inspect Git operation state",
            )?;
            let path = Path::new(&path);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                checkout_root.join(path)
            };
            if path.exists() {
                return Err(DeliveryError::new(
                    "Git Town stack worktree has an in-progress Git operation",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedCheckState {
    Successful,
    Failed,
    Pending,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedCheck {
    pub name: String,
    pub publisher: CheckPublisher,
    pub check_run_id: Option<u64>,
    pub workflow_run_id: Option<u64>,
    pub status: String,
    pub conclusion: String,
    pub state: ObservedCheckState,
    pub commit_oid: String,
    pub started_at_unix_seconds: u64,
    pub completed_at_unix_seconds: Option<u64>,
    pub workflow_created_at_unix_seconds: Option<u64>,
    pub workflow_updated_at_unix_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestStatus {
    pub repository: String,
    pub number: u64,
    pub state: PullRequestState,
    pub merge_state: String,
    pub base_ref: String,
    pub base_oid: String,
    pub head_repository: String,
    pub head_ref: String,
    pub head_oid: String,
    pub merge_commit_oid: Option<String>,
    pub merge_commit_tree_oid: Option<String>,
    pub merge_base_oid: Option<String>,
    pub is_in_merge_queue: bool,
    pub is_merge_queue_enabled: bool,
    pub merge_queue_entry: Option<ObservedMergeQueueEntry>,
    pub checks: Vec<ObservedCheck>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedMergeQueueEntry {
    pub id: String,
    pub state: String,
    pub base_oid: String,
    pub head_oid: String,
}

pub trait PullRequestStatusSource {
    fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus>;
}

#[derive(Debug)]
pub struct GhStatusSource<'a, A> {
    command: &'a A,
}

impl<'a, A> GhStatusSource<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

const PR_STATUS_QUERY: &str = r#"query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    nameWithOwner
    pullRequest(number:$number){
      number state mergeStateStatus baseRefName baseRefOid headRefName headRefOid
      isInMergeQueue isMergeQueueEnabled
      mergeQueueEntry{id state baseCommit{oid} headCommit{oid}}
      headRepository{nameWithOwner}
      mergeCommit{oid tree{oid} parents(first:2){nodes{oid} pageInfo{hasNextPage}}}
      commits(last:1){
        nodes{commit{oid statusCheckRollup{contexts(first:100){
          nodes{
            __typename
            ... on CheckRun{
              databaseId name status conclusion startedAt completedAt
              checkSuite{
                app{slug databaseId}
                commit{oid}
                workflowRun{
                  databaseId createdAt updatedAt
                  workflow{name databaseId}
                }
              }
            }
            ... on StatusContext{context state createdAt creator{login} commit{oid}}
          }
          pageInfo{hasNextPage}
        }}}}
      }
    }
  }
}"#;

impl<A: CommandOutputAdapter> PullRequestStatusSource for GhStatusSource<'_, A> {
    fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus> {
        let (owner, name) = github_owner_name(repository)?;
        let args = vec![
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            format!("query={PR_STATUS_QUERY}"),
            "-f".to_owned(),
            format!("owner={owner}"),
            "-f".to_owned(),
            format!("name={name}"),
            "-F".to_owned(),
            format!("number={pr}"),
        ];
        let output = self.command.output("gh", &args, None)?;
        if !output.success {
            return Err(command_failed(
                format!("GitHub status query failed for {repository}#{pr}"),
                &output,
            ));
        }
        parse_gh_status(repository, pr, &output.stdout)
    }
}

#[derive(Debug, Deserialize)]
struct GraphQlEnvelope {
    data: GraphQlData,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GraphQlData {
    repository: Option<GraphQlRepository>,
}

#[derive(Debug, Deserialize)]
struct GraphQlRepository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    #[serde(rename = "pullRequest")]
    pull_request: Option<GraphQlPullRequest>,
}

#[derive(Debug, Deserialize)]
struct GraphQlPullRequest {
    number: u64,
    state: String,
    #[serde(rename = "mergeStateStatus")]
    merge_state_status: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "baseRefOid")]
    base_ref_oid: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(rename = "headRepository")]
    head_repository: GraphQlName,
    #[serde(rename = "mergeCommit")]
    merge_commit: Option<GraphQlMergeCommit>,
    #[serde(rename = "isInMergeQueue")]
    is_in_merge_queue: bool,
    #[serde(rename = "isMergeQueueEnabled")]
    is_merge_queue_enabled: bool,
    #[serde(rename = "mergeQueueEntry")]
    merge_queue_entry: Option<GraphQlMergeQueueEntry>,
    commits: GraphQlCommits,
}

#[derive(Debug, Deserialize)]
struct GraphQlMergeCommit {
    oid: String,
    tree: GraphQlOid,
    parents: GraphQlParents,
}

#[derive(Debug, Deserialize)]
struct GraphQlParents {
    nodes: Vec<GraphQlOid>,
    #[serde(rename = "pageInfo")]
    page_info: GraphQlPageInfo,
}

#[derive(Debug, Deserialize)]
struct GraphQlMergeQueueEntry {
    id: String,
    state: String,
    #[serde(rename = "baseCommit")]
    base_commit: Option<GraphQlOid>,
    #[serde(rename = "headCommit")]
    head_commit: Option<GraphQlOid>,
}

#[derive(Debug, Deserialize)]
struct GraphQlOid {
    oid: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlName {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommits {
    nodes: Vec<GraphQlCommitNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommitNode {
    commit: GraphQlCommit,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommit {
    oid: String,
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<GraphQlRollup>,
}

#[derive(Debug, Deserialize)]
struct GraphQlRollup {
    contexts: GraphQlContexts,
}

#[derive(Debug, Deserialize)]
struct GraphQlContexts {
    nodes: Vec<serde_json::Value>,
    #[serde(rename = "pageInfo")]
    page_info: GraphQlPageInfo,
}

#[derive(Debug, Deserialize)]
struct GraphQlPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
}

pub(crate) fn parse_gh_status(
    repository: &str,
    expected_pr: u64,
    bytes: &[u8],
) -> Result<PullRequestStatus> {
    validate_repository_id(repository)?;
    let parsed: GraphQlEnvelope = serde_json::from_slice(bytes)
        .map_err(|error| DeliveryError::new(format!("invalid GitHub status JSON: {error}")))?;
    if !parsed.errors.is_empty() {
        return Err(DeliveryError::new(
            "GitHub status response contains partial GraphQL errors",
        ));
    }
    let repo = parsed
        .data
        .repository
        .ok_or_else(|| DeliveryError::new("GitHub repository was not found"))?;
    if !repository_matches(repository, &repo.name_with_owner) {
        return Err(DeliveryError::new(
            "GitHub response repository identity does not match",
        ));
    }
    let pr = repo
        .pull_request
        .ok_or_else(|| DeliveryError::new("GitHub pull request was not found"))?;
    if pr.number != expected_pr {
        return Err(DeliveryError::new(
            "GitHub response PR number does not match",
        ));
    }
    let state = match pr.state.as_str() {
        "OPEN" => PullRequestState::Open,
        "MERGED" => PullRequestState::Merged,
        "CLOSED" => PullRequestState::Closed,
        other => {
            return Err(DeliveryError::new(format!(
                "unknown GitHub PR state {other}"
            )));
        }
    };
    let (merge_commit_oid, merge_commit_tree_oid, merge_base_oid) = match (state, &pr.merge_commit)
    {
        (PullRequestState::Merged, Some(commit)) => {
            validate_hash(&commit.oid, "GitHub merge commit OID")?;
            validate_hash(&commit.tree.oid, "GitHub merge commit tree OID")?;
            if commit.parents.page_info.has_next_page || commit.parents.nodes.is_empty() {
                return Err(DeliveryError::new(
                    "merged GitHub PR has no unambiguous historical merge base",
                ));
            }
            let merge_base = commit.parents.nodes[0].oid.clone();
            validate_hash(&merge_base, "GitHub historical merge base OID")?;
            (
                Some(commit.oid.clone()),
                Some(commit.tree.oid.clone()),
                Some(merge_base),
            )
        }
        (PullRequestState::Merged, None) => {
            return Err(DeliveryError::new(
                "merged GitHub PR has no exact merge commit authority",
            ));
        }
        (_, None) => (None, None, None),
        (_, Some(_)) => {
            return Err(DeliveryError::new(
                "unmerged GitHub PR unexpectedly has merge commit authority",
            ));
        }
    };
    if pr.commits.nodes.len() != 1 {
        return Err(DeliveryError::new(
            "GitHub status response must contain exactly one latest commit",
        ));
    }
    let commit = &pr.commits.nodes[0].commit;
    if commit.oid != pr.head_ref_oid {
        return Err(DeliveryError::new(
            "GitHub check rollup is not associated with the PR head commit",
        ));
    }
    let mut checks = Vec::new();
    if let Some(rollup) = &commit.status_check_rollup {
        if rollup.contexts.page_info.has_next_page {
            return Err(DeliveryError::new(
                "GitHub check rollup exceeds the supported 100 contexts",
            ));
        }
        for value in &rollup.contexts.nodes {
            checks.push(parse_check(value, &commit.oid)?);
        }
    }
    checks.sort();
    let mut names = BTreeSet::new();
    for check in &checks {
        if !names.insert(check.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate same-name GitHub check publisher for {}",
                check.name
            )));
        }
    }
    validate_hash(&pr.base_ref_oid, "GitHub base OID")?;
    validate_hash(&pr.head_ref_oid, "GitHub head OID")?;
    validate_git_ref(&pr.base_ref_name, "GitHub base ref")?;
    validate_git_ref(&pr.head_ref_name, "GitHub head ref")?;
    let merge_queue_entry = match pr.merge_queue_entry {
        Some(entry) => {
            if !pr.is_in_merge_queue {
                return Err(DeliveryError::new(
                    "GitHub returned a merge-queue entry for a PR outside the queue",
                ));
            }
            let base_oid = entry
                .base_commit
                .ok_or_else(|| DeliveryError::new("merge-queue entry has no base commit"))?
                .oid;
            let head_oid = entry
                .head_commit
                .ok_or_else(|| DeliveryError::new("merge-queue entry has no head commit"))?
                .oid;
            validate_bounded_github_value(&entry.id, "merge-queue entry ID")?;
            validate_bounded_github_value(&entry.state, "merge-queue entry state")?;
            validate_hash(&base_oid, "merge-queue base OID")?;
            validate_hash(&head_oid, "merge-queue head OID")?;
            Some(ObservedMergeQueueEntry {
                id: entry.id,
                state: entry.state,
                base_oid,
                head_oid,
            })
        }
        None if pr.is_in_merge_queue => {
            return Err(DeliveryError::new(
                "GitHub reports a queued PR without exact merge-queue authority",
            ));
        }
        None => None,
    };
    Ok(PullRequestStatus {
        repository: repository.to_owned(),
        number: pr.number,
        state,
        merge_state: pr.merge_state_status,
        base_ref: pr.base_ref_name,
        base_oid: pr.base_ref_oid,
        head_repository: logical_github_id(&pr.head_repository.name_with_owner),
        head_ref: pr.head_ref_name,
        head_oid: pr.head_ref_oid,
        merge_commit_oid,
        merge_commit_tree_oid,
        merge_base_oid,
        is_in_merge_queue: pr.is_in_merge_queue,
        is_merge_queue_enabled: pr.is_merge_queue_enabled,
        merge_queue_entry,
        checks,
    })
}

fn validate_bounded_github_value(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > super::model::MAX_STRING_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(DeliveryError::new(format!("invalid GitHub {label}")));
    }
    Ok(())
}

fn parse_check(value: &serde_json::Value, outer_commit: &str) -> Result<ObservedCheck> {
    let object = value
        .as_object()
        .ok_or_else(|| DeliveryError::new("GitHub check entry is not an object"))?;
    let kind = required_string(object, "__typename")?;
    let check = match kind {
        "CheckRun" => {
            let suite = object
                .get("checkSuite")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| DeliveryError::new("GitHub check run has no checkSuite"))?;
            let commit_oid = nested_string(suite, "commit", "oid")?;
            if commit_oid != outer_commit {
                return Err(DeliveryError::new(
                    "GitHub check suite is associated with a different commit",
                ));
            }
            let name = required_string(object, "name")?.to_owned();
            let check_run_id = required_u64(object, "databaseId")?;
            let status = required_string(object, "status")?.to_owned();
            let conclusion = object
                .get("conclusion")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("NONE")
                .to_owned();
            let started_at_unix_seconds =
                parse_github_timestamp(required_string(object, "startedAt")?)?;
            let completed_at_unix_seconds = optional_timestamp(object, "completedAt")?;
            let (workflow, workflow_id, workflow_run_id, workflow_created, workflow_updated) =
                optional_workflow_run(suite)?;
            let app_slug = nested_string(suite, "app", "slug")?.to_owned();
            let app_id = nested_u64(suite, "app", "databaseId")?;
            let state = check_run_state(&status, &conclusion)?;
            if state == ObservedCheckState::Successful && completed_at_unix_seconds.is_none() {
                return Err(DeliveryError::new(
                    "successful GitHub check run has no completion timestamp",
                ));
            }
            ObservedCheck {
                name,
                publisher: CheckPublisher {
                    kind: CheckPublisherKind::CheckRun,
                    app_slug,
                    app_id,
                    workflow,
                    workflow_id,
                },
                check_run_id: Some(check_run_id),
                workflow_run_id,
                status,
                conclusion,
                state,
                commit_oid: commit_oid.to_owned(),
                started_at_unix_seconds,
                completed_at_unix_seconds,
                workflow_created_at_unix_seconds: workflow_created,
                workflow_updated_at_unix_seconds: workflow_updated,
            }
        }
        "StatusContext" => {
            let commit_oid = nested_string(object, "commit", "oid")?;
            if commit_oid != outer_commit {
                return Err(DeliveryError::new(
                    "GitHub status context is associated with a different commit",
                ));
            }
            let name = required_string(object, "context")?.to_owned();
            let status = required_string(object, "state")?.to_owned();
            let app_slug = nested_string(object, "creator", "login")?.to_owned();
            let created_at = parse_github_timestamp(required_string(object, "createdAt")?)?;
            let state = match status.as_str() {
                "SUCCESS" => ObservedCheckState::Successful,
                "PENDING" | "EXPECTED" => ObservedCheckState::Pending,
                "ERROR" | "FAILURE" => ObservedCheckState::Failed,
                other => {
                    return Err(DeliveryError::new(format!(
                        "unknown GitHub status context state {other}"
                    )));
                }
            };
            ObservedCheck {
                name,
                publisher: CheckPublisher {
                    kind: CheckPublisherKind::StatusContext,
                    app_slug,
                    app_id: 0,
                    workflow: "status-context".to_owned(),
                    workflow_id: 0,
                },
                check_run_id: None,
                workflow_run_id: None,
                status: status.clone(),
                conclusion: status,
                state,
                commit_oid: commit_oid.to_owned(),
                started_at_unix_seconds: created_at,
                completed_at_unix_seconds: Some(created_at),
                workflow_created_at_unix_seconds: None,
                workflow_updated_at_unix_seconds: None,
            }
        }
        other => {
            return Err(DeliveryError::new(format!(
                "unknown GitHub check publisher type {other}"
            )));
        }
    };
    Ok(check)
}

fn check_run_state(status: &str, conclusion: &str) -> Result<ObservedCheckState> {
    match status {
        "QUEUED" | "IN_PROGRESS" | "WAITING" | "PENDING" | "REQUESTED" => {
            Ok(ObservedCheckState::Pending)
        }
        "COMPLETED" => match conclusion {
            "SUCCESS" => Ok(ObservedCheckState::Successful),
            "ACTION_REQUIRED" | "CANCELLED" | "FAILURE" | "NEUTRAL" | "SKIPPED" | "STALE"
            | "TIMED_OUT" | "STARTUP_FAILURE" => Ok(ObservedCheckState::Failed),
            "NONE" | "" => Ok(ObservedCheckState::Pending),
            other => Err(DeliveryError::new(format!(
                "unknown GitHub check conclusion {other}"
            ))),
        },
        other => Err(DeliveryError::new(format!(
            "unknown GitHub check status {other}"
        ))),
    }
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<&'a str> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| DeliveryError::new(format!("GitHub check has no {field}")))
}

fn nested_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    parent: &str,
    field: &str,
) -> Result<&'a str> {
    object
        .get(parent)
        .and_then(serde_json::Value::as_object)
        .and_then(|nested| nested.get(field))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            DeliveryError::new(format!(
                "GitHub check has no {parent}.{field} publisher identity"
            ))
        })
}

fn nested_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    parent: &str,
    field: &str,
) -> Result<u64> {
    object
        .get(parent)
        .and_then(serde_json::Value::as_object)
        .and_then(|nested| nested.get(field))
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            DeliveryError::new(format!(
                "GitHub check has no {parent}.{field} publisher identity"
            ))
        })
}

fn required_u64(object: &serde_json::Map<String, serde_json::Value>, field: &str) -> Result<u64> {
    object
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value != 0)
        .ok_or_else(|| DeliveryError::new(format!("GitHub check has no {field}")))
}

type WorkflowRunIdentity = (String, u64, Option<u64>, Option<u64>, Option<u64>);

fn optional_workflow_run(
    check_suite: &serde_json::Map<String, serde_json::Value>,
) -> Result<WorkflowRunIdentity> {
    match check_suite.get("workflowRun") {
        None | Some(serde_json::Value::Null) => Ok(("none".to_owned(), 0, None, None, None)),
        Some(serde_json::Value::Object(run)) => {
            let run_id = required_u64(run, "databaseId")?;
            let created = parse_github_timestamp(required_string(run, "createdAt")?)?;
            let updated = parse_github_timestamp(required_string(run, "updatedAt")?)?;
            if updated < created {
                return Err(DeliveryError::new(
                    "GitHub workflow run timestamps are out of order",
                ));
            }
            let workflow = run
                .get("workflow")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| DeliveryError::new("GitHub workflow run has no workflow"))?;
            let name = workflow
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| DeliveryError::new("GitHub check workflow has no name"))?;
            let id = workflow
                .get("databaseId")
                .and_then(serde_json::Value::as_u64)
                .filter(|id| *id != 0)
                .ok_or_else(|| DeliveryError::new("GitHub check workflow has no databaseId"))?;
            Ok((
                name.to_owned(),
                id,
                Some(run_id),
                Some(created),
                Some(updated),
            ))
        }
        Some(_) => Err(DeliveryError::new(
            "GitHub check workflowRun is not an object or null",
        )),
    }
}

fn optional_timestamp(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<u64>> {
    match object.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => parse_github_timestamp(value).map(Some),
        Some(_) => Err(DeliveryError::new(format!(
            "GitHub check {field} is not a timestamp or null"
        ))),
    }
}

pub(crate) fn parse_github_timestamp(value: &str) -> Result<u64> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return Err(DeliveryError::new(
            "GitHub timestamp is not canonical UTC RFC3339",
        ));
    }
    let year = decimal(bytes, 0, 4)? as i64;
    let month = decimal(bytes, 5, 2)? as i64;
    let day = decimal(bytes, 8, 2)? as i64;
    let hour = decimal(bytes, 11, 2)? as i64;
    let minute = decimal(bytes, 14, 2)? as i64;
    let second = decimal(bytes, 17, 2)? as i64;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(DeliveryError::new("GitHub timestamp is out of range"));
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return Err(DeliveryError::new(
            "GitHub timestamp predates the Unix epoch",
        ));
    }
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second)
        .map_err(|_| DeliveryError::new("GitHub timestamp exceeds supported range"))
}

fn decimal(bytes: &[u8], offset: usize, length: usize) -> Result<u32> {
    bytes[offset..offset + length]
        .iter()
        .try_fold(0_u32, |value, byte| {
            byte.is_ascii_digit()
                .then(|| value * 10 + u32::from(byte - b'0'))
                .ok_or_else(|| DeliveryError::new("GitHub timestamp contains a non-digit"))
        })
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub trait PullRequestMerger {
    fn merge_with_expected_base_and_head(
        &self,
        repository: &str,
        pr: u64,
        expected_base: &str,
        expected_head: &str,
    ) -> Result<()>;
}

#[derive(Debug)]
pub struct GhMergeSource<'a, A> {
    command: &'a A,
}

impl<'a, A> GhMergeSource<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> PullRequestMerger for GhMergeSource<'_, A> {
    fn merge_with_expected_base_and_head(
        &self,
        repository: &str,
        pr: u64,
        expected_base: &str,
        expected_head: &str,
    ) -> Result<()> {
        validate_repository_id(repository)?;
        if pr == 0 {
            return Err(DeliveryError::new("merge PR number must be non-zero"));
        }
        validate_hash(expected_base, "expected merge base")?;
        validate_hash(expected_head, "expected merge head")?;
        let _ = &self.command;
        Err(DeliveryError::new(format!(
            "refusing GitHub merge for {repository}#{pr}: gh exposes expected-head protection but no exact base+head compare-and-swap or verified merge-group authority"
        )))
    }
}

fn github_owner_name(repository: &str) -> Result<(&str, &str)> {
    validate_repository_id(repository)?;
    let parts = repository.split('/').collect::<Vec<_>>();
    if parts.len() == 3 && parts[0] == "github.com" {
        return Ok((parts[1], parts[2]));
    }
    Err(DeliveryError::new(format!(
        "GitHub repository identity must be github.com/owner/name: {repository}"
    )))
}

fn logical_github_id(name_with_owner: &str) -> String {
    format!("github.com/{name_with_owner}")
}

fn repository_matches(logical: &str, name_with_owner: &str) -> bool {
    logical == logical_github_id(name_with_owner)
}

fn parse_repository_identity(remote: &str) -> Result<String> {
    let path = if let Some(rest) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
    {
        let (authority, path) = rest.split_once('/').ok_or_else(remote_identity_error)?;
        validate_https_github_authority(authority)?;
        path
    } else if let Some(path) = remote.strip_prefix("git@github.com:") {
        path
    } else if let Some(rest) = remote.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/').ok_or_else(remote_identity_error)?;
        validate_ssh_github_authority(authority)?;
        path
    } else {
        return Err(remote_identity_error());
    };
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|part| matches!(*part, "." | ".."))
        || remote.bytes().any(|byte| byte.is_ascii_control())
        || path.contains(['\\', '?', '#', '%'])
    {
        return Err(DeliveryError::new(
            "origin remote does not contain exact owner/repository identity",
        ));
    }
    let identity = format!("github.com/{}/{}", parts[0], parts[1]);
    validate_repository_id(&identity)?;
    Ok(identity)
}

fn validate_https_github_authority(authority: &str) -> Result<()> {
    let pieces = authority.split('@').collect::<Vec<_>>();
    match pieces.as_slice() {
        ["github.com"] => Ok(()),
        [userinfo, "github.com"] if valid_http_userinfo(userinfo) => Ok(()),
        _ => Err(remote_identity_error()),
    }
}

fn valid_http_userinfo(userinfo: &str) -> bool {
    if userinfo.is_empty()
        || userinfo.len() > 256
        || userinfo.starts_with(':')
        || userinfo.ends_with(':')
    {
        return false;
    }
    let bytes = userinfo.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let byte = bytes[offset];
        if byte == b'%' {
            if bytes
                .get(offset + 1..offset + 3)
                .is_none_or(|digits| !digits.iter().all(u8::is_ascii_hexdigit))
            {
                return false;
            }
            offset += 3;
            continue;
        }
        if !(byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
            ))
        {
            return false;
        }
        offset += 1;
    }
    true
}

fn validate_ssh_github_authority(authority: &str) -> Result<()> {
    let authority = authority
        .strip_prefix("git@github.com")
        .ok_or_else(remote_identity_error)?;
    if authority.is_empty() {
        return Ok(());
    }
    let port = authority
        .strip_prefix(':')
        .ok_or_else(remote_identity_error)?;
    if port.is_empty()
        || port.len() > 5
        || !port.bytes().all(|byte| byte.is_ascii_digit())
        || port.starts_with('0')
        || port
            .parse::<u16>()
            .ok()
            .filter(|value| *value != 0)
            .is_none()
    {
        return Err(remote_identity_error());
    }
    Ok(())
}

fn remote_identity_error() -> DeliveryError {
    DeliveryError::new("origin remote is not an unambiguous github.com repository identity")
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("path is not valid UTF-8"))
}

fn command_failed(context: impl AsRef<str>, output: &CommandOutput) -> DeliveryError {
    DeliveryError::new(format!(
        "{}: {}",
        context.as_ref(),
        output.safe_failure_summary()
    ))
}

pub fn reject_symlink_components(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(DeliveryError::new(format!(
                    "path contains parent traversal: {}",
                    path.display()
                )));
            }
            Component::Normal(name) => {
                current.push(name);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(DeliveryError::new(format!(
                            "path contains a symlink component: {}",
                            current.display()
                        )));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error()) {
                            break;
                        }
                        return Err(DeliveryError::new(format!(
                            "cannot inspect path component {}: {error}",
                            current.display()
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_repository_slug(repository: &str) -> Result<()> {
    let parts = repository.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 100
                || part.starts_with('.')
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(DeliveryError::new("invalid GitHub repository identity"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::VecDeque,
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    };

    use super::*;

    static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(1);

    type CommandCall = (String, Vec<String>, Option<PathBuf>);

    struct FakeCommand {
        calls: RefCell<Vec<CommandCall>>,
        outputs: RefCell<VecDeque<CommandOutput>>,
    }

    impl FakeCommand {
        fn new(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                outputs: RefCell::new(outputs.into_iter().collect()),
            }
        }
    }

    impl CommandOutputAdapter for FakeCommand {
        fn output_with_limits(
            &self,
            program: &str,
            args: &[String],
            cwd: Option<&Path>,
            _limits: CommandLimits,
        ) -> Result<CommandOutput> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.to_vec(),
                cwd.map(Path::to_path_buf),
            ));
            self.outputs
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| DeliveryError::new("missing fake command output"))
        }
    }

    fn successful_output(stdout: Vec<u8>) -> CommandOutput {
        CommandOutput {
            success: true,
            exit_code: Some(0),
            stdout,
            stderr: vec![],
        }
    }

    fn graphql(checks: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "data": {
                "repository": {
                    "nameWithOwner": "example/d2b",
                    "pullRequest": {
                        "number": 42,
                        "state": "OPEN",
                        "mergeStateStatus": "CLEAN",
                        "baseRefName": "main",
                        "baseRefOid": "a".repeat(40),
                        "headRefName": "feature",
                        "headRefOid": "b".repeat(40),
                        "headRepository": {"nameWithOwner": "example/d2b"},
                        "mergeCommit": null,
                        "isInMergeQueue": false,
                        "isMergeQueueEnabled": false,
                        "mergeQueueEntry": null,
                        "commits": {"nodes": [{
                            "commit": {
                                "oid": "b".repeat(40),
                                "statusCheckRollup": {
                                    "contexts": {
                                        "nodes": checks,
                                        "pageInfo": {"hasNextPage": false}
                                    }
                                }
                            }
                        }]}
                    }
                }
            }
        }))
        .expect("JSON")
    }

    fn check(name: &str, app: &str, status: &str, conclusion: &str) -> serde_json::Value {
        serde_json::json!({
            "__typename": "CheckRun",
            "databaseId": 9876,
            "name": name,
            "status": status,
            "conclusion": conclusion,
            "startedAt": "2026-07-13T10:00:00Z",
            "completedAt": "2026-07-13T10:01:00Z",
            "checkSuite": {
                "app": {"slug": app, "databaseId": 15368},
                "commit": {"oid": "b".repeat(40)},
                "workflowRun": {
                    "databaseId": 6543,
                    "createdAt": "2026-07-13T09:59:00Z",
                    "updatedAt": "2026-07-13T10:01:00Z",
                    "workflow": {"name": "Layer 1", "databaseId": 321}
                }
            }
        })
    }

    #[test]
    fn parses_exact_check_publisher_and_commit_association() {
        let status = parse_gh_status(
            "github.com/example/d2b",
            42,
            &graphql(serde_json::json!([check(
                "check",
                "github-actions",
                "COMPLETED",
                "SUCCESS"
            )])),
        )
        .expect("status");
        assert_eq!(status.head_oid, "b".repeat(40));
        assert_eq!(status.checks[0].publisher.app_slug, "github-actions");
        assert_eq!(status.checks[0].state, ObservedCheckState::Successful);
        assert_eq!(status.checks[0].check_run_id, Some(9876));
        assert_eq!(status.checks[0].workflow_run_id, Some(6543));
        assert_eq!(
            status.checks[0].completed_at_unix_seconds,
            Some(1_783_936_860)
        );
    }

    #[test]
    fn duplicate_same_name_publishers_fail_closed() {
        let error = parse_gh_status(
            "github.com/example/d2b",
            42,
            &graphql(serde_json::json!([
                check("check", "github-actions", "COMPLETED", "SUCCESS"),
                check("check", "other-app", "COMPLETED", "SUCCESS")
            ])),
        )
        .expect_err("duplicates");
        assert!(error.to_string().contains("duplicate same-name"));
    }

    #[test]
    fn external_graphql_extensions_are_ignored_but_authority_remains_required() {
        let mut extended: serde_json::Value =
            serde_json::from_slice(&graphql(serde_json::json!([]))).expect("fixture JSON");
        extended["extensions"] = serde_json::json!({"requestId": "opaque"});
        extended["data"]["extension"] = serde_json::json!(true);
        extended["data"]["repository"]["extension"] = serde_json::json!({"new": "field"});
        extended["data"]["repository"]["pullRequest"]["extension"] = serde_json::json!(["future"]);
        extended["data"]["repository"]["pullRequest"]["commits"]["nodes"][0]["commit"]["extension"] =
            serde_json::json!(42);
        parse_gh_status(
            "github.com/example/d2b",
            42,
            &serde_json::to_vec(&extended).expect("extended JSON"),
        )
        .expect("non-breaking extensions");

        extended["errors"] = serde_json::json!([{"message": "partial"}]);
        let error = parse_gh_status(
            "github.com/example/d2b",
            42,
            &serde_json::to_vec(&extended).expect("partial JSON"),
        )
        .expect_err("partial GraphQL response");
        assert!(error.to_string().contains("partial GraphQL errors"));

        let mut missing: serde_json::Value =
            serde_json::from_slice(&graphql(serde_json::json!([]))).expect("fixture JSON");
        missing["data"]["repository"]["pullRequest"]
            .as_object_mut()
            .expect("pull request")
            .remove("baseRefOid");
        assert!(
            parse_gh_status(
                "github.com/example/d2b",
                42,
                &serde_json::to_vec(&missing).expect("missing JSON"),
            )
            .is_err()
        );
        missing["data"]["repository"]["pullRequest"]["baseRefOid"] = serde_json::json!(7);
        assert!(
            parse_gh_status(
                "github.com/example/d2b",
                42,
                &serde_json::to_vec(&missing).expect("wrong-type JSON"),
            )
            .is_err()
        );
    }

    #[test]
    fn validation_children_receive_only_controlled_environment() {
        let inherited = BTreeMap::from([
            (
                OsString::from("PATH"),
                OsString::from("/bin:relative:/usr/bin"),
            ),
            (
                OsString::from("GITHUB_TOKEN"),
                OsString::from("secret-canary"),
            ),
            (
                OsString::from("SSH_AUTH_SOCK"),
                OsString::from("/run/secret-agent"),
            ),
            (
                OsString::from("AWS_ACCESS_KEY_ID"),
                OsString::from("secret-canary"),
            ),
            (
                OsString::from("CARGO_HOME"),
                OsString::from("/var/lib/toolchain/cargo"),
            ),
            (OsString::from("HOME"), OsString::from("/home/alice")),
            (
                OsString::from("XDG_CONFIG_HOME"),
                OsString::from("/home/alice/.config"),
            ),
        ]);
        let explicit = BTreeMap::from([
            (OsString::from("D2B_REQUIRED"), OsString::from("present")),
            (
                OsString::from("CARGO_HOME"),
                OsString::from("/private/validation/cargo-home"),
            ),
        ]);
        let controlled = controlled_environment(
            "sh",
            &explicit,
            CommandEnvironment::Validation,
            inherited.clone(),
        );
        assert_eq!(
            controlled.get(std::ffi::OsStr::new("D2B_REQUIRED")),
            Some(&OsString::from("present"))
        );
        assert_eq!(
            controlled.get(std::ffi::OsStr::new("LC_ALL")),
            Some(&OsString::from("C"))
        );
        assert_eq!(
            controlled.get(std::ffi::OsStr::new("CARGO_HOME")),
            Some(&OsString::from("/private/validation/cargo-home"))
        );
        for secret in ["GITHUB_TOKEN", "SSH_AUTH_SOCK", "AWS_ACCESS_KEY_ID"] {
            assert!(!controlled.contains_key(std::ffi::OsStr::new(secret)));
        }
        assert!(
            std::env::split_paths(
                controlled
                    .get(std::ffi::OsStr::new("PATH"))
                    .expect("controlled PATH")
            )
            .all(|path| path.is_absolute())
        );
        let authority = controlled_environment(
            "gh",
            &BTreeMap::new(),
            CommandEnvironment::Authority,
            inherited.clone(),
        );
        assert_eq!(
            authority.get(std::ffi::OsStr::new("GITHUB_TOKEN")),
            Some(&OsString::from("secret-canary"))
        );
        let git_town = controlled_environment(
            "git-town",
            &BTreeMap::new(),
            CommandEnvironment::Authority,
            inherited,
        );
        assert_eq!(
            git_town.get(std::ffi::OsStr::new("HOME")),
            Some(&OsString::from("/home/alice"))
        );
        assert_eq!(
            git_town.get(std::ffi::OsStr::new("XDG_CONFIG_HOME")),
            Some(&OsString::from("/home/alice/.config"))
        );
        assert!(!git_town.contains_key(std::ffi::OsStr::new("GITHUB_TOKEN")));

        let output = ProcessCommandOutput
            .output_with_environment(
                "sh",
                &[
                    "-c".to_owned(),
                    "test \"$D2B_REQUIRED\" = present \
                     && test \"$LC_ALL\" = C \
                     && test \"$TZ\" = UTC \
                     && test -n \"$PATH\" \
                     && test -z \"${GITHUB_TOKEN+x}\" \
                     && test -z \"${SSH_AUTH_SOCK+x}\""
                        .to_owned(),
                ],
                None,
                &explicit,
                CommandLimits {
                    stdout_bytes: 64,
                    stderr_bytes: 64,
                    timeout: Duration::from_secs(1),
                },
            )
            .expect("controlled child");
        assert!(output.success, "{}", output.safe_failure_summary());
    }

    #[test]
    fn process_runner_caps_stdout_stderr_and_timeout() {
        let runner = ProcessCommandOutput;
        let stdout = runner
            .output_with_limits(
                "sh",
                &["-c".to_owned(), "printf 12345".to_owned()],
                None,
                CommandLimits {
                    stdout_bytes: 4,
                    stderr_bytes: 4,
                    timeout: Duration::from_secs(1),
                },
            )
            .expect("bounded stdout result");
        assert_eq!(stdout.failure(), Some(CommandFailure::StdoutOverflow));
        assert_eq!(stdout.private_diagnostics().stdout, b"1234");
        assert!(!stdout.safe_failure_summary().contains("1234"));

        let stderr = runner
            .output_with_limits(
                "sh",
                &["-c".to_owned(), "printf 12345 >&2".to_owned()],
                None,
                CommandLimits {
                    stdout_bytes: 4,
                    stderr_bytes: 4,
                    timeout: Duration::from_secs(1),
                },
            )
            .expect("bounded stderr result");
        assert_eq!(stderr.failure(), Some(CommandFailure::StderrOverflow));
        assert_eq!(stderr.private_diagnostics().stderr, b"1234");
        assert!(!stderr.safe_failure_summary().contains("1234"));

        let started = Instant::now();
        let timeout = runner
            .output_with_limits(
                "sh",
                &[
                    "-c".to_owned(),
                    "printf private-timeout-diagnostic; sleep 5".to_owned(),
                ],
                None,
                CommandLimits {
                    stdout_bytes: 64,
                    stderr_bytes: 64,
                    timeout: Duration::from_millis(20),
                },
            )
            .expect("bounded timeout result");
        assert_eq!(timeout.failure(), Some(CommandFailure::Timeout));
        assert_eq!(
            timeout.private_diagnostics().stdout,
            b"private-timeout-diagnostic"
        );
        assert!(
            !timeout
                .safe_failure_summary()
                .contains("private-timeout-diagnostic")
        );
        assert!(started.elapsed() < Duration::from_secs(1));

        let started = Instant::now();
        let escaped_pipe = runner
            .output_with_limits(
                "sh",
                &[
                    "-c".to_owned(),
                    "setsid sh -c 'exec sleep 0.25' & printf parent-complete".to_owned(),
                ],
                None,
                CommandLimits {
                    stdout_bytes: 64,
                    stderr_bytes: 64,
                    timeout: Duration::from_millis(20),
                },
            )
            .expect("detached descendant timeout result");
        assert_eq!(escaped_pipe.failure(), Some(CommandFailure::Timeout));
        assert_eq!(
            escaped_pipe.private_diagnostics().stdout,
            b"parent-complete"
        );
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn process_runner_poll_handles_silent_child_and_output_burst() {
        let silent = ProcessCommandOutput
            .output_with_limits(
                "sh",
                &["-c".to_owned(), "sleep 0.1; printf complete".to_owned()],
                None,
                CommandLimits {
                    stdout_bytes: 64,
                    stderr_bytes: 64,
                    timeout: Duration::from_secs(1),
                },
            )
            .expect("silent child");
        assert!(silent.success);
        assert_eq!(silent.stdout, b"complete");

        let burst = ProcessCommandOutput
            .output_with_limits(
                "sh",
                &[
                    "-c".to_owned(),
                    "i=0; while test \"$i\" -lt 4096; do printf x; i=$((i+1)); done".to_owned(),
                ],
                None,
                CommandLimits {
                    stdout_bytes: 8192,
                    stderr_bytes: 64,
                    timeout: Duration::from_secs(1),
                },
            )
            .expect("output burst");
        assert!(burst.success);
        assert_eq!(burst.stdout.len(), 4096);
    }

    #[test]
    fn terminal_signal_registration_is_global_across_repeated_commands() {
        for _ in 0..2 {
            let output = ProcessCommandOutput
                .output_with_limits(
                    "sh",
                    &["-c".to_owned(), "printf ok".to_owned()],
                    None,
                    CommandLimits {
                        stdout_bytes: 16,
                        stderr_bytes: 16,
                        timeout: Duration::from_secs(1),
                    },
                )
                .expect("repeated command");
            assert!(output.success);
        }
        assert_eq!(TERMINAL_SIGNAL_INITIALIZATIONS.load(Ordering::Acquire), 1);
        assert!(
            TERMINAL_SIGNAL_FLAGS
                .get()
                .and_then(|signals| signals.as_ref().ok())
                .expect("global signal dispatcher")
                .inactive
                .load(Ordering::Acquire),
            "completed commands must deregister active signal ownership"
        );
    }

    #[test]
    fn exited_leader_stays_unreaped_until_descendant_group_cleanup() {
        let started = Instant::now();
        let output = ProcessCommandOutput
            .output_with_limits(
                "sh",
                &[
                    "-c".to_owned(),
                    "sleep 30 & printf parent-complete; exit 0".to_owned(),
                ],
                None,
                CommandLimits {
                    stdout_bytes: 64,
                    stderr_bytes: 64,
                    timeout: Duration::from_millis(50),
                },
            )
            .expect("descendant timeout result");
        assert_eq!(output.failure(), Some(CommandFailure::Timeout));
        assert_eq!(output.private_diagnostics().stdout, b"parent-complete");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    struct DelayedTerminalSignal {
        polls: usize,
        signal: Option<TerminalSignal>,
    }

    impl TerminalSignalSource for DelayedTerminalSignal {
        fn pending(&mut self) -> Option<TerminalSignal> {
            if self.polls == 0 {
                return self.signal.take();
            }
            self.polls -= 1;
            None
        }
    }

    #[test]
    fn terminal_signal_forwards_to_and_cleans_exact_process_group() {
        for (signal, shell_signal, expected) in [
            (
                TerminalSignal::Interrupt,
                "INT",
                CommandFailure::Interrupted,
            ),
            (
                TerminalSignal::Terminate,
                "TERM",
                CommandFailure::Terminated,
            ),
        ] {
            let marker = unique_test_path("signal-orphan-marker");
            let marker_string = marker.to_string_lossy().into_owned();
            let script = format!(
                "trap 'printf leader-signaled >&2; exit 0' {shell_signal}; \
                 (trap '' {shell_signal}; sleep 0.4; printf orphan > \"$1\") & wait"
            );
            let mut signals = DelayedTerminalSignal {
                polls: 20,
                signal: Some(signal),
            };
            let environment = controlled_environment(
                "sh",
                &BTreeMap::new(),
                CommandEnvironment::Validation,
                std::env::vars_os(),
            );
            let started = Instant::now();
            let output = run_process(
                "sh",
                &["-c".to_owned(), script, "sh".to_owned(), marker_string],
                None,
                &environment,
                CommandLimits {
                    stdout_bytes: 128,
                    stderr_bytes: 128,
                    timeout: Duration::from_secs(2),
                },
                &mut signals,
            )
            .expect("terminal signal result");
            assert_eq!(output.failure(), Some(expected));
            assert!(
                String::from_utf8_lossy(output.private_diagnostics().stderr)
                    .contains("leader-signaled")
            );
            assert!(started.elapsed() < Duration::from_secs(1));
            thread::sleep(Duration::from_millis(500));
            let survived = marker.exists();
            if survived {
                fs::remove_file(&marker).expect("remove orphan marker");
            }
            assert!(!survived, "descendant survived process-group cleanup");
        }
    }

    #[test]
    fn github_status_uses_string_fields_for_numeric_looking_names() {
        let command = FakeCommand::new(vec![successful_output(Vec::new())]);
        GhStatusSource::new(&command)
            .status("github.com/123/456", 42)
            .expect_err("fixture response is intentionally empty");
        let args = &command.calls.borrow()[0].1;
        assert!(args.windows(2).any(|pair| pair == ["-f", "owner=123"]));
        assert!(args.windows(2).any(|pair| pair == ["-f", "name=456"]));
        assert!(args.windows(2).any(|pair| pair == ["-F", "number=42"]));
        assert!(!args.windows(2).any(|pair| pair == ["-F", "owner=123"]));
        assert!(!args.windows(2).any(|pair| pair == ["-F", "name=456"]));
    }

    #[test]
    fn canonical_github_remotes_accept_userinfo_and_ssh_ports() {
        for remote in [
            "https://github.com/example/d2b.git",
            "https://alice@github.com/example/d2b.git",
            "http://alice:token@github.com/example/d2b/",
            "https://alice%2Bbot@github.com/example/d2b.git",
            "git@github.com:example/d2b.git",
            "ssh://git@github.com/example/d2b.git",
            "ssh://git@github.com:2222/example/d2b.git",
        ] {
            assert_eq!(
                parse_repository_identity(remote).expect("canonical GitHub remote"),
                "github.com/example/d2b"
            );
        }
    }

    #[test]
    fn github_remote_parser_rejects_ambiguous_authorities_and_traversal() {
        for remote in [
            "https://github.com.evil/example/d2b.git",
            "https://github.com:443/example/d2b.git",
            "https://alice@@github.com/example/d2b.git",
            "https://:token@github.com/example/d2b.git",
            "https://alice:@github.com/example/d2b.git",
            "https://alice%4@github.com/example/d2b.git",
            "https://github.com/example/../d2b.git",
            "https://github.com/example/d2b.git?token=secret",
            "ssh://root@github.com/example/d2b.git",
            "ssh://git@github.com:0/example/d2b.git",
            "ssh://git@github.com:65536/example/d2b.git",
            "ssh://git@github.com:22@example.invalid/example/d2b.git",
            "git@github.com:example/d2b/extra.git",
        ] {
            let error = parse_repository_identity(remote).expect_err("ambiguous remote");
            assert!(
                !error.to_string().contains("secret")
                    && !error.to_string().contains("alice")
                    && !error.to_string().contains("token")
            );
        }
    }

    #[test]
    fn dangling_symlink_components_are_rejected() {
        let root = unique_test_path("dangling-symlink");
        fs::create_dir(&root).expect("test root");
        let dangling = root.join("dangling");
        symlink(root.join("missing-target"), &dangling).expect("dangling symlink");
        for path in [&dangling, &dangling.join("child")] {
            let error = reject_symlink_components(path).expect_err("dangling symlink");
            assert!(error.to_string().contains("symlink component"));
        }
        fs::remove_file(&dangling).expect("remove symlink");
        fs::remove_dir(&root).expect("remove test root");
    }

    fn unique_test_path(label: &str) -> PathBuf {
        std::env::current_dir()
            .expect("current directory")
            .join(format!(
                ".d2b-{label}-{}-{}",
                std::process::id(),
                NEXT_TEST_PATH.fetch_add(1, AtomicOrdering::Relaxed)
            ))
    }

    #[test]
    fn object_format_validation_is_exact() {
        assert!(
            super::super::model::validate_hash_for_format(
                &"a".repeat(40),
                GitObjectFormat::Sha1,
                "oid"
            )
            .is_ok()
        );
        assert!(
            super::super::model::validate_hash_for_format(
                &"a".repeat(40),
                GitObjectFormat::Sha256,
                "oid"
            )
            .is_err()
        );
    }

    fn stack_source_prefix(current: &str) -> Vec<CommandOutput> {
        let mut outputs = vec![successful_output(Vec::new())];
        for path in [
            ".git/rebase-merge",
            ".git/rebase-apply",
            ".git/MERGE_HEAD",
            ".git/CHERRY_PICK_HEAD",
            ".git/REVERT_HEAD",
        ] {
            outputs.push(successful_output(format!("{path}\n").into_bytes()));
        }
        outputs.push(successful_output(format!("{current}\n").into_bytes()));
        outputs.push(successful_output(b"main\n".to_vec()));
        outputs
    }

    fn stack_policy(branch: &str, pr_number: u64) -> StackNodePolicy {
        StackNodePolicy {
            id: branch.to_owned(),
            repository: "github.com/example/d2b".to_owned(),
            branch: branch.to_owned(),
            pr_number,
            external_dependencies: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stack_status(
        number: u64,
        branch: &str,
        base_ref: &str,
        observed_base: &str,
        head: &str,
        state: &str,
        merge_base: Option<&str>,
        merge_commit: Option<&str>,
        merge_tree: Option<&str>,
    ) -> CommandOutput {
        let mut value: serde_json::Value =
            serde_json::from_slice(&graphql(serde_json::json!([]))).expect("status fixture");
        let pr = &mut value["data"]["repository"]["pullRequest"];
        pr["number"] = serde_json::json!(number);
        pr["state"] = serde_json::json!(state);
        pr["baseRefName"] = serde_json::json!(base_ref);
        pr["baseRefOid"] = serde_json::json!(observed_base);
        pr["headRefName"] = serde_json::json!(branch);
        pr["headRefOid"] = serde_json::json!(head);
        pr["commits"]["nodes"][0]["commit"]["oid"] = serde_json::json!(head);
        pr["mergeCommit"] = match (merge_base, merge_commit, merge_tree) {
            (Some(base), Some(commit), Some(tree)) => serde_json::json!({
                "oid": commit,
                "tree": {"oid": tree},
                "parents": {
                    "nodes": [{"oid": base}],
                    "pageInfo": {"hasNextPage": false}
                }
            }),
            (None, None, None) => serde_json::Value::Null,
            _ => panic!("merge fixture must be complete"),
        };
        successful_output(serde_json::to_vec(&value).expect("status JSON"))
    }

    fn active_verification(head: &str, base: &str) -> Vec<CommandOutput> {
        vec![
            successful_output(format!("{head}\n").into_bytes()),
            successful_output(format!("{base}\n").into_bytes()),
            successful_output(Vec::new()),
        ]
    }

    fn merged_verification(base: &str, head: &str, tree: &str) -> Vec<CommandOutput> {
        vec![
            successful_output(format!("{tree}\n").into_bytes()),
            successful_output(format!("{base} {head}\n").into_bytes()),
            successful_output(format!("{tree}\n").into_bytes()),
            successful_output(Vec::new()),
            successful_output(Vec::new()),
        ]
    }

    #[test]
    fn git_town_adapter_uses_parent_config_local_oids_and_ordinary_prs() {
        let feature = "b".repeat(40);
        let main = "a".repeat(40);
        let mut outputs = stack_source_prefix("feature");
        outputs.push(stack_status(
            42, "feature", "main", &main, &feature, "OPEN", None, None, None,
        ));
        outputs.push(successful_output(b"main\n".to_vec()));
        outputs.extend(active_verification(&feature, &main));
        let command = FakeCommand::new(outputs);
        let graph = GitTownStackSource::new(&command)
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect("graph");
        assert_eq!(graph.current_branch, "feature");
        assert_eq!(graph.branches[0].base, main);
        let calls = command.calls.borrow();
        assert!(
            calls.iter().any(|call| {
                call.0 == "git-town" && call.1 == ["config", "get-parent", "feature"]
            })
        );
        assert!(calls.iter().any(|call| {
            call.0 == "gh"
                && call
                    .1
                    .starts_with(&["api".to_owned(), "graphql".to_owned()])
        }));
        assert!(
            !calls
                .iter()
                .any(|call| call.1.iter().any(|arg| arg == "preview"))
        );
    }

    #[test]
    fn git_town_adapter_reconstructs_one_merged_node_absent_from_lineage() {
        let main = "a".repeat(40);
        let first = "b".repeat(40);
        let second = "c".repeat(40);
        let merged = "d".repeat(40);
        let merge_tree = "e".repeat(40);
        let mut outputs = stack_source_prefix("second");
        outputs.push(stack_status(
            41,
            "first",
            "main",
            &main,
            &first,
            "MERGED",
            Some(&main),
            Some(&merged),
            Some(&merge_tree),
        ));
        outputs.push(stack_status(
            42, "second", "main", &merged, &second, "OPEN", None, None, None,
        ));
        outputs.push(successful_output(b"main\n".to_vec()));
        outputs.extend(merged_verification(&main, &first, &merge_tree));
        outputs.extend(active_verification(&second, &merged));
        let graph = GitTownStackSource::new(&FakeCommand::new(outputs))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("first", 41), stack_policy("second", 42)],
            )
            .expect("ordered graph");
        assert_eq!(
            graph
                .branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(graph.branches[0].is_merged);
        assert_eq!(graph.branches[0].base, main);
        assert_eq!(graph.branches[0].observed_base, main);
        assert_eq!(graph.branches[1].parent, "first");
        assert_eq!(graph.branches[1].base_ref, "main");
        assert!(graph.branches[1].is_current);
    }

    #[test]
    fn git_town_adapter_reconstructs_two_merged_nodes_absent_from_lineage() {
        let main = "a".repeat(40);
        let first = "b".repeat(40);
        let second = "c".repeat(40);
        let third = "d".repeat(40);
        let first_merge = "e".repeat(40);
        let second_merge = "f".repeat(40);
        let first_tree = "1".repeat(40);
        let second_tree = "2".repeat(40);
        let mut outputs = stack_source_prefix("third");
        outputs.push(stack_status(
            41,
            "first",
            "main",
            &main,
            &first,
            "MERGED",
            Some(&main),
            Some(&first_merge),
            Some(&first_tree),
        ));
        outputs.push(stack_status(
            42,
            "second",
            "main",
            &first_merge,
            &second,
            "MERGED",
            Some(&first_merge),
            Some(&second_merge),
            Some(&second_tree),
        ));
        outputs.push(stack_status(
            43,
            "third",
            "main",
            &second_merge,
            &third,
            "OPEN",
            None,
            None,
            None,
        ));
        outputs.push(successful_output(b"main\n".to_vec()));
        outputs.extend(merged_verification(&main, &first, &first_tree));
        outputs.extend(merged_verification(&first_merge, &second, &second_tree));
        outputs.extend(active_verification(&third, &second_merge));
        let graph = GitTownStackSource::new(&FakeCommand::new(outputs))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[
                    stack_policy("first", 41),
                    stack_policy("second", 42),
                    stack_policy("third", 43),
                ],
            )
            .expect("multiple merged prefix");
        assert_eq!(graph.branches[0].base, main);
        assert_eq!(graph.branches[1].base, first_merge);
        assert_eq!(graph.branches[2].base, second_merge);
        assert_eq!(graph.branches[2].base_ref, "main");
    }

    #[test]
    fn git_town_adapter_reconstructs_all_but_top_merged() {
        let main = "a".repeat(40);
        let heads = [
            "b".repeat(40),
            "c".repeat(40),
            "d".repeat(40),
            "e".repeat(40),
        ];
        let merges = ["f".repeat(40), "1".repeat(40), "2".repeat(40)];
        let trees = ["3".repeat(40), "4".repeat(40), "5".repeat(40)];
        let mut outputs = stack_source_prefix("fourth");
        for index in 0..3 {
            outputs.push(stack_status(
                41 + index as u64,
                ["first", "second", "third"][index],
                "main",
                if index == 0 {
                    &main
                } else {
                    &merges[index - 1]
                },
                &heads[index],
                "MERGED",
                Some(if index == 0 {
                    &main
                } else {
                    &merges[index - 1]
                }),
                Some(&merges[index]),
                Some(&trees[index]),
            ));
        }
        outputs.push(stack_status(
            44, "fourth", "main", &merges[2], &heads[3], "OPEN", None, None, None,
        ));
        outputs.push(successful_output(b"main\n".to_vec()));
        for index in 0..3 {
            outputs.extend(merged_verification(
                if index == 0 {
                    &main
                } else {
                    &merges[index - 1]
                },
                &heads[index],
                &trees[index],
            ));
        }
        outputs.extend(active_verification(&heads[3], &merges[2]));
        let graph = GitTownStackSource::new(&FakeCommand::new(outputs))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[
                    stack_policy("first", 41),
                    stack_policy("second", 42),
                    stack_policy("third", 43),
                    stack_policy("fourth", 44),
                ],
            )
            .expect("all but top merged");
        assert_eq!(
            graph.branches.iter().filter(|node| node.is_merged).count(),
            3
        );
        assert_eq!(graph.branches[3].parent, "third");
    }

    #[test]
    fn git_town_adapter_rejects_forged_merge_authority() {
        let main = "a".repeat(40);
        let first = "b".repeat(40);
        let merged = "c".repeat(40);
        let tree = "d".repeat(40);
        let top = "e".repeat(40);
        let mut outputs = stack_source_prefix("top");
        outputs.push(stack_status(
            41,
            "first",
            "main",
            &main,
            &first,
            "MERGED",
            Some(&main),
            Some(&merged),
            Some(&tree),
        ));
        outputs.push(stack_status(
            42, "top", "main", &merged, &top, "OPEN", None, None, None,
        ));
        outputs.push(successful_output(b"main\n".to_vec()));
        outputs.push(successful_output(
            format!("{}\n", "f".repeat(40)).into_bytes(),
        ));
        let error = GitTownStackSource::new(&FakeCommand::new(outputs))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("first", 41), stack_policy("top", 42)],
            )
            .expect_err("forged merge");
        assert!(error.to_string().contains("forged"));
    }

    #[test]
    fn git_town_adapter_rejects_unrelated_merged_prefix_and_wrong_historical_parent() {
        let main = "a".repeat(40);
        let first = "b".repeat(40);
        let first_merge = "c".repeat(40);
        let unrelated_merge = "d".repeat(40);
        let top = "e".repeat(40);
        let tree = "f".repeat(40);

        let mut unrelated = stack_source_prefix("top");
        unrelated.push(stack_status(
            41,
            "first",
            "main",
            &main,
            &first,
            "MERGED",
            Some(&main),
            Some(&first_merge),
            Some(&tree),
        ));
        unrelated.push(stack_status(
            42,
            "top",
            "main",
            &unrelated_merge,
            &top,
            "OPEN",
            None,
            None,
            None,
        ));
        unrelated.push(successful_output(b"main\n".to_vec()));
        let error = GitTownStackSource::new(&FakeCommand::new(unrelated))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("first", 41), stack_policy("top", 42)],
            )
            .expect_err("unrelated merged prefix");
        assert!(error.to_string().contains("retargeted to exact trunk"));

        let second = "1".repeat(40);
        let second_merge = "2".repeat(40);
        let wrong_base = "3".repeat(40);
        let mut wrong_parent = stack_source_prefix("top");
        wrong_parent.push(stack_status(
            41,
            "first",
            "main",
            &main,
            &first,
            "MERGED",
            Some(&main),
            Some(&first_merge),
            Some(&tree),
        ));
        wrong_parent.push(stack_status(
            42,
            "second",
            "first",
            &wrong_base,
            &second,
            "MERGED",
            Some(&wrong_base),
            Some(&second_merge),
            Some(&"4".repeat(40)),
        ));
        wrong_parent.push(stack_status(
            43,
            "top",
            "main",
            &second_merge,
            &top,
            "OPEN",
            None,
            None,
            None,
        ));
        wrong_parent.push(successful_output(b"main\n".to_vec()));
        let error = GitTownStackSource::new(&FakeCommand::new(wrong_parent))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[
                    stack_policy("first", 41),
                    stack_policy("second", 42),
                    stack_policy("top", 43),
                ],
            )
            .expect_err("wrong historical parent");
        assert!(error.to_string().contains("historical continuity"));
    }

    #[test]
    fn git_town_adapter_rejects_forged_local_merge_parents() {
        let main = "a".repeat(40);
        let first = "b".repeat(40);
        let merged = "c".repeat(40);
        let tree = "d".repeat(40);
        let top = "e".repeat(40);
        let mut outputs = stack_source_prefix("top");
        outputs.push(stack_status(
            41,
            "first",
            "main",
            &main,
            &first,
            "MERGED",
            Some(&main),
            Some(&merged),
            Some(&tree),
        ));
        outputs.push(stack_status(
            42, "top", "main", &merged, &top, "OPEN", None, None, None,
        ));
        outputs.push(successful_output(b"main\n".to_vec()));
        outputs.push(successful_output(format!("{tree}\n").into_bytes()));
        outputs.push(successful_output(
            format!("{main} {}\n", "f".repeat(40)).into_bytes(),
        ));
        let error = GitTownStackSource::new(&FakeCommand::new(outputs))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("first", 41), stack_policy("top", 42)],
            )
            .expect_err("forged parent");
        assert!(error.to_string().contains("forged merge parent"));
    }

    #[test]
    fn git_town_adapter_rejects_reordered_manifest_and_non_prefix_merge() {
        let main = "a".repeat(40);
        let first = "b".repeat(40);
        let second = "c".repeat(40);
        let merged = "d".repeat(40);
        let mut reordered = stack_source_prefix("second");
        reordered.push(stack_status(
            42, "second", "first", &first, &second, "OPEN", None, None, None,
        ));
        reordered.push(stack_status(
            41, "first", "main", &main, &first, "OPEN", None, None, None,
        ));
        let error = GitTownStackSource::new(&FakeCommand::new(reordered))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("second", 42), stack_policy("first", 41)],
            )
            .expect_err("reordered manifest");
        assert!(error.to_string().contains("active stack top"));

        let first_merge = "e".repeat(40);
        let second_merge = "f".repeat(40);
        let mut reordered_prefix = stack_source_prefix("top");
        reordered_prefix.push(stack_status(
            42,
            "second",
            "first",
            &first,
            &second,
            "MERGED",
            Some(&first),
            Some(&second_merge),
            Some(&"1".repeat(40)),
        ));
        reordered_prefix.push(stack_status(
            41,
            "first",
            "main",
            &main,
            &first,
            "MERGED",
            Some(&main),
            Some(&first_merge),
            Some(&"2".repeat(40)),
        ));
        reordered_prefix.push(stack_status(
            43,
            "top",
            "main",
            &second_merge,
            &"3".repeat(40),
            "OPEN",
            None,
            None,
            None,
        ));
        reordered_prefix.push(successful_output(b"main\n".to_vec()));
        let error = GitTownStackSource::new(&FakeCommand::new(reordered_prefix))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[
                    stack_policy("second", 42),
                    stack_policy("first", 41),
                    stack_policy("top", 43),
                ],
            )
            .expect_err("reordered merged prefix");
        assert!(error.to_string().contains("exact trunk base"));

        let mut non_prefix = stack_source_prefix("second");
        non_prefix.push(stack_status(
            41, "first", "main", &main, &first, "OPEN", None, None, None,
        ));
        non_prefix.push(stack_status(
            42,
            "second",
            "main",
            &merged,
            &second,
            "MERGED",
            Some(&main),
            Some(&merged),
            Some(&"e".repeat(40)),
        ));
        let error = GitTownStackSource::new(&FakeCommand::new(non_prefix))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("first", 41), stack_policy("second", 42)],
            )
            .expect_err("non-prefix merged state");
        assert!(error.to_string().contains("merged node after an active"));
    }

    #[test]
    fn git_town_adapter_rejects_stale_or_extra_active_parent() {
        let main = "a".repeat(40);
        let first = "b".repeat(40);
        let second = "c".repeat(40);
        let statuses = [
            stack_status(41, "first", "main", &main, &first, "OPEN", None, None, None),
            stack_status(
                42, "second", "first", &first, &second, "OPEN", None, None, None,
            ),
        ];
        let mut stale = stack_source_prefix("second");
        stale.extend(statuses.clone());
        stale.push(successful_output(b"main\n".to_vec()));
        let error = GitTownStackSource::new(&FakeCommand::new(stale))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("first", 41), stack_policy("second", 42)],
            )
            .expect_err("stale active parent");
        assert!(error.to_string().contains("active parent"));

        let mut extra = stack_source_prefix("first");
        extra.push(statuses[0].clone());
        extra.push(successful_output(b"extra\n".to_vec()));
        let error = GitTownStackSource::new(&FakeCommand::new(extra))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("first", 41)],
            )
            .expect_err("extra active branch");
        assert!(error.to_string().contains("active parent"));
    }

    #[test]
    fn git_town_adapter_rejects_closed_missing_and_wrong_pr_identity() {
        let main = "a".repeat(40);
        let feature = "b".repeat(40);
        let mut closed = stack_source_prefix("feature");
        closed.push(stack_status(
            42, "feature", "main", &main, &feature, "CLOSED", None, None, None,
        ));
        let error = GitTownStackSource::new(&FakeCommand::new(closed))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect_err("closed");
        assert!(error.to_string().contains("closed unmerged"));

        let mut missing = stack_source_prefix("feature");
        missing.push(CommandOutput {
            success: false,
            exit_code: Some(1),
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
        let error = GitTownStackSource::new(&FakeCommand::new(missing))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect_err("missing PR");
        assert!(error.to_string().contains("GitHub status query failed"));

        let mut mismatch = stack_source_prefix("feature");
        mismatch.push(stack_status(
            42, "other", "main", &main, &feature, "OPEN", None, None, None,
        ));
        let error = GitTownStackSource::new(&FakeCommand::new(mismatch))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect_err("PR mismatch");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn git_town_adapter_rejects_wrong_retarget_cycle_and_non_ancestor() {
        let main = "a".repeat(40);
        let feature = "b".repeat(40);
        let mut wrong_retarget = stack_source_prefix("feature");
        wrong_retarget.push(stack_status(
            42, "feature", "other", &main, &feature, "OPEN", None, None, None,
        ));
        wrong_retarget.push(successful_output(b"other\n".to_vec()));
        let error = GitTownStackSource::new(&FakeCommand::new(wrong_retarget))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect_err("wrong retarget");
        assert!(error.to_string().contains("active Git Town topology"));

        let mut cycle = stack_source_prefix("feature");
        cycle.push(stack_status(
            42, "feature", "main", &main, &feature, "OPEN", None, None, None,
        ));
        cycle.push(successful_output(b"feature\n".to_vec()));
        let error = GitTownStackSource::new(&FakeCommand::new(cycle))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect_err("self-cycle");
        assert!(error.to_string().contains("self-cycle"));

        let mut non_ancestor = stack_source_prefix("feature");
        non_ancestor.push(stack_status(
            42, "feature", "main", &main, &feature, "OPEN", None, None, None,
        ));
        non_ancestor.push(successful_output(b"main\n".to_vec()));
        non_ancestor.extend([
            successful_output(format!("{feature}\n").into_bytes()),
            successful_output(format!("{main}\n").into_bytes()),
            CommandOutput {
                success: false,
                exit_code: Some(1),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        ]);
        let error = GitTownStackSource::new(&FakeCommand::new(non_ancestor))
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect_err("non-ancestor");
        assert!(error.to_string().contains("not an ancestor"));
    }

    #[test]
    fn git_town_adapter_rejects_dirty_worktree_before_topology_queries() {
        let command = FakeCommand::new([successful_output(b" M tracked-file\0".to_vec())]);
        let error = GitTownStackSource::new(&command)
            .graph(
                "github.com/example/d2b",
                Path::new("/checkout"),
                &[stack_policy("feature", 42)],
            )
            .expect_err("dirty");
        assert!(error.to_string().contains("dirty or ambiguous"));
        assert_eq!(command.calls.borrow().len(), 1);
    }

    #[test]
    fn github_status_query_uses_real_check_suite_schema() {
        let command = FakeCommand::new(vec![successful_output(graphql(serde_json::json!([
            check("check", "github-actions", "COMPLETED", "SUCCESS")
        ])))]);
        GhStatusSource::new(&command)
            .status("github.com/example/d2b", 42)
            .expect("status");
        let calls = command.calls.borrow();
        let query = calls[0].1.join(" ");
        for field in [
            "baseRefOid",
            "headRefOid",
            "status",
            "conclusion",
            "workflow",
            "workflowRun",
            "checkSuite",
            "databaseId",
            "startedAt",
            "completedAt",
            "createdAt",
            "updatedAt",
            "isInMergeQueue",
            "isMergeQueueEnabled",
            "mergeQueueEntry",
            "mergeCommit",
            "baseCommit",
            "headCommit",
            "tree",
            "app",
            "commit",
        ] {
            assert!(query.contains(field), "query omitted {field}");
        }
        assert!(!query.contains("CheckRun{name status conclusion workflow"));
        assert!(!query.contains("conclusion workflow{name"));
        assert!(!query.contains("databaseId} app{"));
    }

    #[test]
    fn merge_queue_authority_is_complete_or_rejected() {
        let mut queued: serde_json::Value =
            serde_json::from_slice(&graphql(serde_json::json!([check(
                "check",
                "github-actions",
                "COMPLETED",
                "SUCCESS"
            )])))
            .expect("fixture JSON");
        queued
            .pointer_mut("/data/repository/pullRequest")
            .and_then(serde_json::Value::as_object_mut)
            .expect("pull request")
            .insert("isInMergeQueue".to_owned(), serde_json::json!(true));
        let missing = serde_json::to_vec(&queued).expect("queued JSON");
        let error = parse_gh_status("github.com/example/d2b", 42, &missing)
            .expect_err("missing queue authority");
        assert!(
            error
                .to_string()
                .contains("without exact merge-queue authority")
        );

        queued
            .pointer_mut("/data/repository/pullRequest")
            .and_then(serde_json::Value::as_object_mut)
            .expect("pull request")
            .insert(
                "mergeQueueEntry".to_owned(),
                serde_json::json!({
                    "id": "MQE_example",
                    "state": "AWAITING_CHECKS",
                    "baseCommit": {"oid": "a".repeat(40)},
                    "headCommit": {"oid": "c".repeat(40)}
                }),
            );
        let complete = parse_gh_status(
            "github.com/example/d2b",
            42,
            &serde_json::to_vec(&queued).expect("queued JSON"),
        )
        .expect("complete queue authority");
        assert!(complete.is_in_merge_queue);
        assert_eq!(
            complete.merge_queue_entry.expect("queue entry").base_oid,
            "a".repeat(40)
        );
    }

    #[test]
    fn merged_pr_fixture_carries_exact_merge_commit_and_tree() {
        let mut merged: serde_json::Value =
            serde_json::from_slice(&graphql(serde_json::json!([check(
                "check",
                "github-actions",
                "COMPLETED",
                "SUCCESS"
            )])))
            .expect("fixture JSON");
        let pull_request = merged
            .pointer_mut("/data/repository/pullRequest")
            .and_then(serde_json::Value::as_object_mut)
            .expect("pull request");
        pull_request.insert("state".to_owned(), serde_json::json!("MERGED"));
        pull_request.insert(
            "mergeCommit".to_owned(),
            serde_json::json!({
                "oid": "c".repeat(40),
                "tree": {"oid": "d".repeat(40)},
                "parents": {
                    "nodes": [{"oid": "a".repeat(40)}],
                    "pageInfo": {"hasNextPage": false}
                }
            }),
        );
        let status = parse_gh_status(
            "github.com/example/d2b",
            42,
            &serde_json::to_vec(&merged).expect("merged JSON"),
        )
        .expect("merged status");
        assert_eq!(status.state, PullRequestState::Merged);
        assert_eq!(status.merge_commit_oid, Some("c".repeat(40)));
        assert_eq!(status.merge_commit_tree_oid, Some("d".repeat(40)));
        assert_eq!(status.merge_base_oid, Some("a".repeat(40)));
    }

    #[test]
    fn gh_merge_adapter_refuses_head_only_authority() {
        let command = FakeCommand::new(vec![]);
        let error = GhMergeSource::new(&command)
            .merge_with_expected_base_and_head(
                "github.com/example/d2b",
                42,
                &"a".repeat(40),
                &"b".repeat(40),
            )
            .expect_err("no base compare-and-swap");
        assert!(error.to_string().contains("base+head compare-and-swap"));
        assert!(command.calls.borrow().is_empty());
    }

    fn capability_response(owner: &str, name: &str) -> CommandOutput {
        successful_output(
            serde_json::to_vec(&serde_json::json!({
                "data": {
                    "repository": {
                        "nameWithOwner": format!("{owner}/{name}"),
                        "pullRequests": {
                            "nodes": [{
                                "number": 42,
                                "state": "OPEN",
                                "baseRefName": "main",
                                "headRefName": "feature",
                                "headRefOid": "b".repeat(40)
                            }]
                        }
                    }
                }
            }))
            .expect("capability JSON"),
        )
    }

    #[test]
    fn git_town_capability_checks_supported_major_auth_and_ordinary_pr_api() {
        let mut response = capability_response("123", "456");
        let mut extended: serde_json::Value =
            serde_json::from_slice(&response.stdout).expect("capability JSON");
        extended["extensions"] = serde_json::json!({"requestId": "opaque"});
        extended["data"]["repository"]["extension"] = serde_json::json!(true);
        extended["data"]["repository"]["pullRequests"]["nodes"][0]["extension"] =
            serde_json::json!("future");
        response.stdout = serde_json::to_vec(&extended).expect("extended capability JSON");
        let command = FakeCommand::new([
            successful_output(b"Git Town 23.0.1\n".to_vec()),
            successful_output(b"--stack\n--non-interactive\n--no-browser\n".to_vec()),
            successful_output(Vec::new()),
            response,
        ]);
        let capability = check_git_town_capability(&command, "123/456").expect("available");
        assert_eq!(capability.version, GIT_TOWN_LOCKED_VERSION);
        assert_eq!(capability.supported_major, GIT_TOWN_SUPPORTED_MAJOR);
        assert!(capability.non_interactive_propose);
        let calls = command.calls.borrow();
        assert_eq!(calls[0].0, "git-town");
        assert_eq!(calls[1].1, ["propose", "--help"]);
        assert_eq!(calls[2].1, ["auth", "status", "--hostname", "github.com"]);
        let query = calls[3].1.join(" ");
        assert!(query.contains("pullRequests(first:1"));
        assert!(query.contains("owner=123"));
        assert!(query.contains("name=456"));
    }

    #[test]
    fn git_town_capability_rejects_partial_and_malformed_graphql_authority() {
        let mut partial = capability_response("example", "d2b");
        let mut value: serde_json::Value =
            serde_json::from_slice(&partial.stdout).expect("capability JSON");
        value["errors"] = serde_json::json!([{"message": "partial"}]);
        partial.stdout = serde_json::to_vec(&value).expect("partial JSON");
        let command = FakeCommand::new([
            successful_output(b"Git Town 23.0.1\n".to_vec()),
            successful_output(b"--stack --non-interactive --no-browser\n".to_vec()),
            successful_output(Vec::new()),
            partial,
        ]);
        let error = check_git_town_capability(&command, "example/d2b").expect_err("partial errors");
        assert!(error.to_string().contains("partial GraphQL errors"));

        let mut malformed = capability_response("example", "d2b");
        let mut value: serde_json::Value =
            serde_json::from_slice(&malformed.stdout).expect("capability JSON");
        value["data"]["repository"]["pullRequests"]["nodes"][0]["headRefOid"] =
            serde_json::json!(42);
        malformed.stdout = serde_json::to_vec(&value).expect("malformed JSON");
        let command = FakeCommand::new([
            successful_output(b"Git Town 23.0.1\n".to_vec()),
            successful_output(b"--stack --non-interactive --no-browser\n".to_vec()),
            successful_output(Vec::new()),
            malformed,
        ]);
        let error =
            check_git_town_capability(&command, "example/d2b").expect_err("wrong typed authority");
        assert!(error.to_string().contains("response is invalid"));
    }

    #[test]
    fn git_town_capability_fails_closed_on_version_auth_and_repository() {
        let wrong_version = FakeCommand::new([successful_output(b"Git Town 24.0.0\n".to_vec())]);
        let error = check_git_town_capability(&wrong_version, "example/d2b")
            .expect_err("unsupported major");
        assert!(error.to_string().contains("required major is 23"));

        let auth_failure = FakeCommand::new([
            successful_output(b"Git Town 23.9.0\n".to_vec()),
            successful_output(b"--stack --non-interactive --no-browser\n".to_vec()),
            CommandOutput {
                success: false,
                exit_code: Some(1),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        ]);
        let error =
            check_git_town_capability(&auth_failure, "example/d2b").expect_err("missing auth");
        assert!(error.to_string().contains("authentication"));

        let missing_repository = FakeCommand::new([
            successful_output(b"Git Town 23.0.1\n".to_vec()),
            successful_output(b"--stack --non-interactive --no-browser\n".to_vec()),
            successful_output(Vec::new()),
            successful_output(br#"{"data":{"repository":null}}"#.to_vec()),
        ]);
        let error = check_git_town_capability(&missing_repository, "example/d2b")
            .expect_err("missing repository");
        assert!(error.to_string().contains("repository is unavailable"));
    }
}
