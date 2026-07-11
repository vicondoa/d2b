use crate::output_ring::OutputRing;
use crate::shell_socket::OwnedShellListener;
use crate::supervisor_protocol::{
    SUPERVISOR_PROTOCOL_VERSION, SupervisorAction, SupervisorFailure, SupervisorRequest,
    SupervisorResponse, SupervisorResult, read_frame as read_supervisor_frame,
    validate_request as validate_supervisor_request, write_frame as write_supervisor_frame,
};
use d2b_contracts::terminal_wire::{
    TerminalCloseResult, TerminalControlResult, TerminalStatus, TerminalStream, TerminalWaitResult,
    TerminalWriteStdinResult,
};
use d2b_contracts::unsafe_local_wire::{
    HelperFailureCode, HelperSupervisorId, HelperTerminalAttachmentClosed,
    HelperTerminalControlResponse, HelperTerminalOperationResult, HelperTerminalReadOutputResult,
    HelperTerminalRejected, HelperTerminalRequest, HelperTerminalResponse,
    MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE, MAX_UNSAFE_LOCAL_TERMINAL_WAIT_TIMEOUT_MS,
    decode_unsafe_local_terminal_frame, encode_unsafe_local_terminal_frame,
};
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use rustix::fs::{Mode, OFlags};
use rustix::io::{Errno, ioctl_fionbio, read as fd_read, write as fd_write};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};
use uzers::os::unix::UserExt;
use uzers::{get_current_uid, get_user_by_uid};

pub(crate) const DEFAULT_SHELL_OUTPUT_RING_BYTES: usize = 512 * 1024;
pub(crate) const MAX_HELPER_SHELL_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const SHELL_SUPERVISOR_READY_TIMEOUT: Duration = Duration::from_secs(5);
const _: () = {
    assert!(DEFAULT_SHELL_OUTPUT_RING_BYTES > 0);
    assert!(DEFAULT_SHELL_OUTPUT_RING_BYTES < 8 * 1024 * 1024);
    assert!(MAX_HELPER_SHELL_OUTPUT_BYTES / DEFAULT_SHELL_OUTPUT_RING_BYTES == 64);
};
const MAX_SUPERVISOR_SPEC_BYTES: usize = 2 * 1024 * 1024;
const MAX_TERMINAL_WORKERS: usize = 16;
const MAX_CONTROL_CONNECTIONS: usize = 32;
const SUPERVISOR_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellSupervisorError {
    InvalidSpec,
    SpawnFailed,
    ReadyTimeout,
    RuntimeUnavailable,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ShellSupervisorSpec {
    pub supervisor_id: HelperSupervisorId,
    pub runtime_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub initial_rows: u16,
    pub initial_cols: u16,
    pub output_ring_bytes: usize,
}

impl fmt::Debug for ShellSupervisorSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellSupervisorSpec")
            .field("supervisor_id", &"<redacted>")
            .field("runtime_directory", &"<redacted>")
            .field("environment_count", &self.environment.len())
            .field("cwd", &"<redacted>")
            .field("initial_rows", &self.initial_rows)
            .field("initial_cols", &self.initial_cols)
            .field("output_ring_bytes", &self.output_ring_bytes)
            .finish()
    }
}

pub(crate) struct BlockedShellSupervisor {
    child: Option<Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
}

impl BlockedShellSupervisor {
    pub(crate) fn spawn(
        executable: &Path,
        spec: &ShellSupervisorSpec,
    ) -> Result<Self, ShellSupervisorError> {
        let encoded = serde_json::to_vec(spec).map_err(|_| ShellSupervisorError::InvalidSpec)?;
        if encoded.is_empty() || encoded.len() > MAX_SUPERVISOR_SPEC_BYTES {
            return Err(ShellSupervisorError::InvalidSpec);
        }
        let mut child = Command::new(executable)
            .arg("shell-supervisor")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ShellSupervisorError::SpawnFailed)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or(ShellSupervisorError::SpawnFailed)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(ShellSupervisorError::SpawnFailed)?;
        let length = u32::try_from(encoded.len()).map_err(|_| ShellSupervisorError::InvalidSpec)?;
        if stdin.write_all(&length.to_le_bytes()).is_err() || stdin.write_all(&encoded).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ShellSupervisorError::SpawnFailed);
        }
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: Some(stdout),
        })
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.as_ref().expect("shell supervisor child").id()
    }

    pub(crate) fn release_and_wait_ready(&mut self) -> Result<(), ShellSupervisorError> {
        let mut stdin = self.stdin.take().ok_or(ShellSupervisorError::SpawnFailed)?;
        stdin
            .write_all(&[1])
            .map_err(|_| ShellSupervisorError::SpawnFailed)?;
        drop(stdin);

        let mut stdout = self
            .stdout
            .take()
            .ok_or(ShellSupervisorError::SpawnFailed)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("d2b-shell-ready".to_owned())
            .spawn(move || {
                let mut ready = [0u8; 1];
                let result = stdout.read_exact(&mut ready).map(|()| ready[0]);
                let _ = sender.send(result);
            })
            .map_err(|_| ShellSupervisorError::RuntimeUnavailable)?;
        match receiver.recv_timeout(SHELL_SUPERVISOR_READY_TIMEOUT) {
            Ok(Ok(1)) => Ok(()),
            Ok(Ok(_)) | Ok(Err(_)) => Err(ShellSupervisorError::SpawnFailed),
            Err(_) => Err(ShellSupervisorError::ReadyTimeout),
        }
    }

    pub(crate) fn abort(&mut self) {
        self.stdin.take();
        self.stdout.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub(crate) fn reap_in_background(mut self) {
        self.stdin.take();
        self.stdout.take();
        if let Some(mut child) = self.child.take() {
            let _ = std::thread::Builder::new()
                .name("d2b-shell-reaper".to_owned())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}

impl Drop for BlockedShellSupervisor {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.abort();
        }
    }
}

#[derive(Debug, Default)]
struct ProcessState {
    terminal_status: Option<TerminalStatus>,
}

#[derive(Debug, Default)]
struct AttachmentProtocolState {
    input_offset: u64,
    control_sequence: u64,
}

struct AttachmentSlot {
    generation: u64,
    shutdown: UnixStream,
}

impl fmt::Debug for AttachmentSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AttachmentSlot")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

struct SupervisorState {
    ring: Arc<OutputRing>,
    master: Mutex<Option<OwnedFd>>,
    process: Mutex<ProcessState>,
    process_changed: Condvar,
    attachment: Mutex<Option<AttachmentSlot>>,
    next_attachment: AtomicU64,
    kill_requested: AtomicBool,
    stdin_closed: AtomicBool,
    terminal_workers: Arc<(Mutex<usize>, Condvar)>,
    control_connections: AtomicUsize,
}

impl fmt::Debug for SupervisorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupervisorState")
            .field(
                "kill_requested",
                &self.kill_requested.load(Ordering::Acquire),
            )
            .field("stdin_closed", &self.stdin_closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl SupervisorState {
    fn new(master: OwnedFd, ring: Arc<OutputRing>) -> Self {
        Self {
            ring,
            master: Mutex::new(Some(master)),
            process: Mutex::new(ProcessState::default()),
            process_changed: Condvar::new(),
            attachment: Mutex::new(None),
            next_attachment: AtomicU64::new(0),
            kill_requested: AtomicBool::new(false),
            stdin_closed: AtomicBool::new(false),
            terminal_workers: Arc::new((Mutex::new(0), Condvar::new())),
            control_connections: AtomicUsize::new(0),
        }
    }

    fn status(&self) -> (bool, bool, Option<TerminalStatus>) {
        let terminal_status = self
            .process
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .terminal_status
            .clone();
        let attached = self
            .attachment
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some();
        (terminal_status.is_none(), attached, terminal_status)
    }

    fn set_terminal_status(&self, status: TerminalStatus) {
        let mut process = self
            .process
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if process.terminal_status.is_none() {
            process.terminal_status = Some(status);
            self.process_changed.notify_all();
        }
    }

    fn register_attachment(
        &self,
        stream: &UnixStream,
        force: bool,
    ) -> Result<(u64, bool), SupervisorFailure> {
        let shutdown = stream
            .try_clone()
            .map_err(|_| SupervisorFailure::Internal)?;
        let mut slot = self
            .attachment
            .lock()
            .map_err(|_| SupervisorFailure::Internal)?;
        let force_evicted = if let Some(existing) = slot.as_ref() {
            if !force {
                return Err(SupervisorFailure::AlreadyAttached);
            }
            let _ = existing.shutdown.shutdown(std::net::Shutdown::Both);
            true
        } else {
            false
        };
        let mut generation = self.next_attachment.fetch_add(1, Ordering::AcqRel) + 1;
        if generation == 0 {
            self.next_attachment.store(1, Ordering::Release);
            generation = 1;
        }
        *slot = Some(AttachmentSlot {
            generation,
            shutdown,
        });
        Ok((generation, force_evicted))
    }

    fn attachment_is_current(&self, generation: u64) -> bool {
        self.attachment
            .lock()
            .map(|slot| {
                slot.as_ref()
                    .is_some_and(|active| active.generation == generation)
            })
            .unwrap_or(false)
    }

    fn clear_attachment(&self, generation: u64) -> bool {
        let Ok(mut slot) = self.attachment.lock() else {
            return false;
        };
        if slot
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            slot.take();
            true
        } else {
            false
        }
    }

    fn detach_current(&self) -> bool {
        let Ok(mut slot) = self.attachment.lock() else {
            return false;
        };
        let Some(active) = slot.take() else {
            return false;
        };
        let _ = active.shutdown.shutdown(std::net::Shutdown::Both);
        true
    }

    fn close_master(&self) {
        if let Ok(mut master) = self.master.lock() {
            master.take();
        }
        self.ring.close();
    }

    fn close_for_kill(&self) {
        self.detach_current();
        self.close_master();
    }

    fn finish_kill(&self) {
        self.kill_requested.store(true, Ordering::Release);
        self.process_changed.notify_all();
    }

    fn resize(&self, rows: u32, cols: u32) -> Result<(), HelperFailureCode> {
        let rows = u16::try_from(rows).map_err(|_| HelperFailureCode::InvalidTerminalSize)?;
        let cols = u16::try_from(cols).map_err(|_| HelperFailureCode::InvalidTerminalSize)?;
        if rows == 0 || cols == 0 {
            return Err(HelperFailureCode::InvalidTerminalSize);
        }
        let master = self
            .master
            .lock()
            .map_err(|_| HelperFailureCode::Internal)?;
        let master = master.as_ref().ok_or(HelperFailureCode::TerminalClosed)?;
        tcsetwinsize(
            master,
            Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .map_err(|_| HelperFailureCode::TerminalClosed)
    }

    fn write_input(&self, input: &[u8]) -> Result<(usize, bool), HelperFailureCode> {
        if self.stdin_closed.load(Ordering::Acquire) {
            return Err(HelperFailureCode::TerminalClosed);
        }
        let master = self
            .master
            .lock()
            .map_err(|_| HelperFailureCode::Internal)?;
        let master = master.as_ref().ok_or(HelperFailureCode::TerminalClosed)?;
        match fd_write(master, input) {
            Ok(written) => Ok((written, written < input.len())),
            Err(Errno::AGAIN) => Ok((0, true)),
            Err(_) => Err(HelperFailureCode::TerminalClosed),
        }
    }

    fn close_stdin(&self) -> Result<bool, HelperFailureCode> {
        if self.stdin_closed.load(Ordering::Acquire) {
            return Ok(true);
        }
        let (written, backpressured) = self.write_input(&[0x04])?;
        if written == 1 && !backpressured {
            self.stdin_closed.store(true, Ordering::Release);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn wait(&self, timeout: Duration) -> TerminalWaitResult {
        let process = self
            .process
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if process.terminal_status.is_some() || timeout.is_zero() {
            return TerminalWaitResult {
                running: process.terminal_status.is_none(),
                terminal_status: process.terminal_status.clone(),
            };
        }
        let (process, _) = self
            .process_changed
            .wait_timeout(process, timeout)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        TerminalWaitResult {
            running: process.terminal_status.is_none(),
            terminal_status: process.terminal_status.clone(),
        }
    }
}

pub(crate) fn run_shell_supervisor() -> Result<(), ShellSupervisorError> {
    let spec = read_supervisor_spec()?;
    let uid = get_current_uid();
    let user = get_user_by_uid(uid).ok_or(ShellSupervisorError::InvalidSpec)?;
    if spec.initial_rows == 0
        || spec.initial_cols == 0
        || spec.output_ring_bytes == 0
        || spec.output_ring_bytes > DEFAULT_SHELL_OUTPUT_RING_BYTES
        || !spec.cwd.is_absolute()
        || user.home_dir() != spec.cwd
    {
        return Err(ShellSupervisorError::InvalidSpec);
    }
    let listener = OwnedShellListener::bind(&spec.runtime_directory, &spec.supervisor_id, uid)
        .map_err(|_| ShellSupervisorError::RuntimeUnavailable)?;
    listener
        .listener()
        .set_nonblocking(true)
        .map_err(|_| ShellSupervisorError::RuntimeUnavailable)?;
    let (master, mut child) = spawn_login_shell(&spec)?;
    ioctl_fionbio(&master, true).map_err(|_| ShellSupervisorError::RuntimeUnavailable)?;
    let ring =
        Arc::new(OutputRing::new(spec.output_ring_bytes).ok_or(ShellSupervisorError::InvalidSpec)?);
    let state = Arc::new(SupervisorState::new(master, Arc::clone(&ring)));
    drop(spec);
    start_pty_reader(Arc::clone(&state));
    write_ready(1)?;

    while !state.kill_requested.load(Ordering::Acquire) {
        match listener.listener().accept() {
            Ok((stream, _)) => {
                if state.control_connections.fetch_add(1, Ordering::AcqRel)
                    >= MAX_CONTROL_CONNECTIONS
                {
                    state.control_connections.fetch_sub(1, Ordering::AcqRel);
                    drop(stream);
                    continue;
                }
                let worker_state = Arc::clone(&state);
                if std::thread::Builder::new()
                    .name("d2b-shell-control".to_owned())
                    .spawn(move || {
                        handle_supervisor_connection(stream, Arc::clone(&worker_state));
                        worker_state
                            .control_connections
                            .fetch_sub(1, Ordering::AcqRel);
                    })
                    .is_err()
                {
                    state.control_connections.fetch_sub(1, Ordering::AcqRel);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(ShellSupervisorError::RuntimeUnavailable),
        }
        match child.try_wait() {
            Ok(Some(status)) => state.set_terminal_status(exit_status(status)),
            Ok(None) => {}
            Err(_) => state.set_terminal_status(TerminalStatus::Error {
                slug: "wait-failed".to_owned(),
            }),
        }
        std::thread::sleep(SUPERVISOR_POLL_INTERVAL);
    }
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => {
                state.set_terminal_status(exit_status(status));
                break;
            }
            Ok(None) => std::thread::sleep(SUPERVISOR_POLL_INTERVAL),
            Err(_) => break,
        }
    }
    drop(listener);
    Ok(())
}

fn read_supervisor_spec() -> Result<ShellSupervisorSpec, ShellSupervisorError> {
    let mut prefix = [0u8; 4];
    std::io::stdin()
        .read_exact(&mut prefix)
        .map_err(|_| ShellSupervisorError::InvalidSpec)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_SUPERVISOR_SPEC_BYTES {
        return Err(ShellSupervisorError::InvalidSpec);
    }
    let mut body = vec![0u8; length];
    std::io::stdin()
        .read_exact(&mut body)
        .map_err(|_| ShellSupervisorError::InvalidSpec)?;
    let spec = serde_json::from_slice(&body).map_err(|_| ShellSupervisorError::InvalidSpec)?;
    let mut release = [0u8; 1];
    std::io::stdin()
        .read_exact(&mut release)
        .map_err(|_| ShellSupervisorError::InvalidSpec)?;
    if release != [1] {
        return Err(ShellSupervisorError::InvalidSpec);
    }
    Ok(spec)
}

fn write_ready(value: u8) -> Result<(), ShellSupervisorError> {
    std::io::stdout()
        .write_all(&[value])
        .and_then(|()| std::io::stdout().flush())
        .map_err(|_| ShellSupervisorError::RuntimeUnavailable)
}

fn spawn_login_shell(spec: &ShellSupervisorSpec) -> Result<(OwnedFd, Child), ShellSupervisorError> {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)
        .map_err(|_| ShellSupervisorError::SpawnFailed)?;
    grantpt(&master).map_err(|_| ShellSupervisorError::SpawnFailed)?;
    unlockpt(&master).map_err(|_| ShellSupervisorError::SpawnFailed)?;
    let slave_path = ptsname(&master, Vec::new()).map_err(|_| ShellSupervisorError::SpawnFailed)?;
    let slave = rustix::fs::open(
        &slave_path,
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ShellSupervisorError::SpawnFailed)?;
    let (status_read, status_write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        .map_err(|_| ShellSupervisorError::SpawnFailed)?;
    let executable = std::env::current_exe().map_err(|_| ShellSupervisorError::SpawnFailed)?;
    let mut command = Command::new(executable);
    command
        .arg("--tty-exec")
        .arg("--rows")
        .arg(spec.initial_rows.to_string())
        .arg("--cols")
        .arg(spec.initial_cols.to_string())
        .env_clear()
        .envs(&spec.environment)
        .current_dir(&spec.cwd)
        .stdin(Stdio::from(slave))
        .stdout(Stdio::from(status_write))
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| ShellSupervisorError::SpawnFailed)?;
    drop(command);

    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("d2b-shell-exec-ready".to_owned())
        .spawn(move || {
            let mut status = File::from(status_read);
            let mut byte = [0u8; 1];
            let result = status.read(&mut byte);
            let _ = sender.send(result);
        })
        .map_err(|_| ShellSupervisorError::RuntimeUnavailable)?;
    match receiver.recv_timeout(SHELL_SUPERVISOR_READY_TIMEOUT) {
        Ok(Ok(0)) => Ok((master, child)),
        Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(ShellSupervisorError::SpawnFailed)
        }
    }
}

fn start_pty_reader(state: Arc<SupervisorState>) {
    let _ = std::thread::Builder::new()
        .name("d2b-shell-pty-reader".to_owned())
        .spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                let result = {
                    let master = state
                        .master
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let Some(master) = master.as_ref() else {
                        break;
                    };
                    fd_read(master, &mut buffer)
                };
                match result {
                    Ok(0) => {
                        state.ring.close();
                        break;
                    }
                    Ok(count) => state.ring.append(&buffer[..count]),
                    Err(Errno::AGAIN) => std::thread::sleep(SUPERVISOR_POLL_INTERVAL),
                    Err(Errno::INTR) => {}
                    Err(_) => {
                        state.ring.close();
                        break;
                    }
                }
            }
        });
}

fn handle_supervisor_connection(mut stream: UnixStream, state: Arc<SupervisorState>) {
    let Ok(peer) = getsockopt(&stream, PeerCredentials) else {
        return;
    };
    if peer.uid() != get_current_uid() {
        return;
    }
    if stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(2))))
        .is_err()
    {
        return;
    }
    let Ok(request) = read_supervisor_frame::<SupervisorRequest>(&mut stream) else {
        return;
    };
    if validate_supervisor_request(&request).is_err() {
        return;
    }
    let result = match request.action {
        SupervisorAction::Status => {
            let (running, attached, terminal_status) = state.status();
            SupervisorResult::Status {
                running,
                attached,
                terminal_status,
            }
        }
        SupervisorAction::Attach {
            force,
            initial_terminal_size,
        } => {
            if !state.status().0 {
                SupervisorResult::Rejected {
                    code: SupervisorFailure::Closed,
                }
            } else {
                match state.register_attachment(&stream, force) {
                    Ok((generation, force_evicted)) => {
                        if state
                            .resize(initial_terminal_size.rows, initial_terminal_size.cols)
                            .is_err()
                        {
                            state.clear_attachment(generation);
                            SupervisorResult::Rejected {
                                code: SupervisorFailure::InvalidRequest,
                            }
                        } else {
                            let response = SupervisorResponse {
                                version: SUPERVISOR_PROTOCOL_VERSION,
                                request_id: request.request_id,
                                result: SupervisorResult::Attached { force_evicted },
                            };
                            if write_supervisor_frame(&mut stream, &response).is_ok() {
                                if stream
                                    .set_read_timeout(None)
                                    .and_then(|()| {
                                        stream.set_write_timeout(Some(Duration::from_secs(2)))
                                    })
                                    .is_err()
                                {
                                    state.clear_attachment(generation);
                                    return;
                                }
                                serve_terminal(stream, state, generation);
                            } else {
                                state.clear_attachment(generation);
                            }
                            return;
                        }
                    }
                    Err(code) => SupervisorResult::Rejected { code },
                }
            }
        }
        SupervisorAction::Detach => SupervisorResult::Detached {
            detached: state.detach_current(),
        },
        SupervisorAction::Kill => {
            state.close_for_kill();
            let response = SupervisorResponse {
                version: SUPERVISOR_PROTOCOL_VERSION,
                request_id: request.request_id,
                result: SupervisorResult::KillAccepted,
            };
            let _ = write_supervisor_frame(&mut stream, &response);
            state.finish_kill();
            return;
        }
    };
    let response = SupervisorResponse {
        version: SUPERVISOR_PROTOCOL_VERSION,
        request_id: request.request_id,
        result,
    };
    let _ = write_supervisor_frame(&mut stream, &response);
}

fn serve_terminal(stream: UnixStream, state: Arc<SupervisorState>, generation: u64) {
    let Ok(mut reader) = stream.try_clone() else {
        state.clear_attachment(generation);
        return;
    };
    let mut writer = stream;
    let protocol_state = Arc::new(Mutex::new(AttachmentProtocolState::default()));
    let (responses_tx, responses_rx) =
        mpsc::sync_channel::<(HelperTerminalResponse, bool)>(MAX_TERMINAL_WORKERS * 2);
    let writer_thread = std::thread::Builder::new()
        .name("d2b-shell-terminal-writer".to_owned())
        .spawn(move || {
            while let Ok((response, close)) = responses_rx.recv() {
                let Ok(frame) = encode_unsafe_local_terminal_frame(&response) else {
                    break;
                };
                if writer.write_all(&frame).is_err() {
                    break;
                }
                if close {
                    let _ = writer.shutdown(std::net::Shutdown::Both);
                    break;
                }
            }
        });
    if writer_thread.is_err() {
        state.clear_attachment(generation);
        return;
    }

    let limiter = Arc::clone(&state.terminal_workers);
    while state.attachment_is_current(generation) {
        let request = match read_terminal_request(&mut reader) {
            Ok(request) => request,
            Err(_) => break,
        };
        let request_id = request.request_id();
        if let Err(code) = request.validate_bounds() {
            if responses_tx
                .send((
                    HelperTerminalResponse::Rejected(HelperTerminalRejected { request_id, code }),
                    false,
                ))
                .is_err()
            {
                break;
            }
            continue;
        }
        if !terminal_request_may_run_concurrently(&request) {
            let (response, close) =
                process_terminal_request(request, &state, &protocol_state, generation);
            if responses_tx.send((response, close)).is_err() || close {
                break;
            }
            continue;
        }
        let (count, available) = &*limiter;
        let mut active = count
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *active >= MAX_TERMINAL_WORKERS {
            active = available
                .wait(active)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *active += 1;
        drop(active);

        let state = Arc::clone(&state);
        let protocol_state = Arc::clone(&protocol_state);
        let responses = responses_tx.clone();
        let limiter = Arc::clone(&limiter);
        if std::thread::Builder::new()
            .name("d2b-shell-terminal-operation".to_owned())
            .spawn(move || {
                let (response, close) = if state.attachment_is_current(generation) {
                    process_terminal_request(request, &state, &protocol_state, generation)
                } else {
                    (
                        HelperTerminalResponse::Rejected(HelperTerminalRejected {
                            request_id,
                            code: HelperFailureCode::TerminalClosed,
                        }),
                        true,
                    )
                };
                let _ = responses.send((response, close));
                let (count, available) = &*limiter;
                if let Ok(mut active) = count.lock() {
                    *active = active.saturating_sub(1);
                    available.notify_one();
                }
            })
            .is_err()
        {
            let mut active = count
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *active = active.saturating_sub(1);
            available.notify_one();
            break;
        }
    }
    state.clear_attachment(generation);
    drop(responses_tx);
}

fn terminal_request_may_run_concurrently(request: &HelperTerminalRequest) -> bool {
    matches!(
        request,
        HelperTerminalRequest::ReadOutput(_) | HelperTerminalRequest::Wait(_)
    )
}

fn read_terminal_request(
    stream: &mut UnixStream,
) -> Result<HelperTerminalRequest, HelperFailureCode> {
    let mut prefix = [0u8; 4];
    stream
        .read_exact(&mut prefix)
        .map_err(|_| HelperFailureCode::TerminalClosed)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE {
        return Err(HelperFailureCode::InvalidRequest);
    }
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&prefix);
    frame.resize(length + 4, 0);
    stream
        .read_exact(&mut frame[4..])
        .map_err(|_| HelperFailureCode::InvalidRequest)?;
    decode_unsafe_local_terminal_frame(&frame)
}

fn process_terminal_request(
    request: HelperTerminalRequest,
    state: &SupervisorState,
    protocol_state: &Mutex<AttachmentProtocolState>,
    generation: u64,
) -> (HelperTerminalResponse, bool) {
    let request_id = request.request_id();
    let result = match request {
        HelperTerminalRequest::WriteStdin(request) => {
            let bytes = match d2b_core::base64_codec::decode(request.chunk_base64.as_str()) {
                Ok(bytes) => bytes,
                Err(_) => {
                    return (
                        rejected(request_id, HelperFailureCode::InvalidRequest),
                        false,
                    );
                }
            };
            let mut protocol = match protocol_state.lock() {
                Ok(protocol) => protocol,
                Err(_) => return (rejected(request_id, HelperFailureCode::Internal), false),
            };
            if request.offset != protocol.input_offset {
                return (
                    rejected(request_id, HelperFailureCode::TerminalOffsetMismatch),
                    false,
                );
            }
            match state.write_input(&bytes) {
                Ok((accepted, mut backpressured)) => {
                    protocol.input_offset = protocol.input_offset.saturating_add(accepted as u64);
                    let mut stdin_closed = state.stdin_closed.load(Ordering::Acquire);
                    if request.eof && accepted == bytes.len() {
                        match state.close_stdin() {
                            Ok(closed) => {
                                stdin_closed = closed;
                                backpressured |= !closed;
                            }
                            Err(code) => return (rejected(request_id, code), false),
                        }
                    }
                    HelperTerminalResponse::WriteStdin(HelperTerminalOperationResult {
                        request_id,
                        result: TerminalWriteStdinResult {
                            accepted_len: accepted as u64,
                            next_offset: protocol.input_offset,
                            backpressured,
                            stdin_closed,
                        },
                    })
                }
                Err(code) => rejected(request_id, code),
            }
        }
        HelperTerminalRequest::ReadOutput(request) => {
            let result = match request.stream {
                TerminalStream::Stdout => {
                    let read = state.ring.read(
                        request.cursor,
                        request.max_len as usize,
                        request.wait,
                        Duration::from_millis(request.timeout_ms),
                    );
                    HelperTerminalReadOutputResult {
                        data_base64:
                            match d2b_contracts::unsafe_local_wire::HelperTerminalChunkBase64::new(
                                d2b_core::base64_codec::encode(&read.data),
                            ) {
                                Ok(data) => data,
                                Err(code) => return (rejected(request_id, code), false),
                            },
                        next_cursor: read.next_cursor,
                        eof: read.eof,
                        dropped_bytes: read.dropped_bytes,
                        truncated: read.truncated,
                        timed_out: read.timed_out,
                    }
                }
                TerminalStream::Stderr => HelperTerminalReadOutputResult {
                    data_base64: d2b_contracts::unsafe_local_wire::HelperTerminalChunkBase64::new(
                        "",
                    )
                    .expect("empty base64 is valid"),
                    next_cursor: request.cursor,
                    eof: true,
                    dropped_bytes: 0,
                    truncated: false,
                    timed_out: false,
                },
            };
            HelperTerminalResponse::ReadOutput(HelperTerminalOperationResult { request_id, result })
        }
        HelperTerminalRequest::Resize(request) => {
            let mut protocol = match protocol_state.lock() {
                Ok(protocol) => protocol,
                Err(_) => return (rejected(request_id, HelperFailureCode::Internal), false),
            };
            if request.control_sequence <= protocol.control_sequence {
                return (
                    rejected(request_id, HelperFailureCode::InvalidRequest),
                    false,
                );
            }
            match state.resize(request.rows, request.cols) {
                Ok(()) => {
                    protocol.control_sequence = request.control_sequence;
                    HelperTerminalResponse::Resize(HelperTerminalControlResponse {
                        request_id,
                        control_sequence: request.control_sequence,
                        result: TerminalControlResult { delivered: true },
                    })
                }
                Err(code) => rejected(request_id, code),
            }
        }
        HelperTerminalRequest::Wait(request) => {
            let timeout = request
                .timeout_ms
                .min(MAX_UNSAFE_LOCAL_TERMINAL_WAIT_TIMEOUT_MS);
            HelperTerminalResponse::Wait(HelperTerminalOperationResult {
                request_id,
                result: state.wait(Duration::from_millis(timeout)),
            })
        }
        HelperTerminalRequest::CloseStdin(request) => {
            let mut protocol = match protocol_state.lock() {
                Ok(protocol) => protocol,
                Err(_) => return (rejected(request_id, HelperFailureCode::Internal), false),
            };
            if request.control_sequence <= protocol.control_sequence {
                return (
                    rejected(request_id, HelperFailureCode::InvalidRequest),
                    false,
                );
            }
            match state.close_stdin() {
                Ok(stdin_closed) => {
                    protocol.control_sequence = request.control_sequence;
                    HelperTerminalResponse::CloseStdin(HelperTerminalControlResponse {
                        request_id,
                        control_sequence: request.control_sequence,
                        result: TerminalCloseResult { stdin_closed },
                    })
                }
                Err(code) => rejected(request_id, code),
            }
        }
        HelperTerminalRequest::CloseAttachment(request) => {
            let mut protocol = match protocol_state.lock() {
                Ok(protocol) => protocol,
                Err(_) => return (rejected(request_id, HelperFailureCode::Internal), false),
            };
            if request.control_sequence <= protocol.control_sequence {
                return (
                    rejected(request_id, HelperFailureCode::InvalidRequest),
                    false,
                );
            }
            protocol.control_sequence = request.control_sequence;
            state.clear_attachment(generation);
            return (
                HelperTerminalResponse::CloseAttachment(HelperTerminalControlResponse {
                    request_id,
                    control_sequence: request.control_sequence,
                    result: HelperTerminalAttachmentClosed {
                        detached: true,
                        cause: Some(d2b_contracts::public_wire::ShellCloseCause::ClientDetach),
                    },
                }),
                true,
            );
        }
    };
    (result, false)
}

fn rejected(request_id: u64, code: HelperFailureCode) -> HelperTerminalResponse {
    HelperTerminalResponse::Rejected(HelperTerminalRejected { request_id, code })
}

fn exit_status(status: std::process::ExitStatus) -> TerminalStatus {
    if let Some(code) = status.code() {
        TerminalStatus::Exited { code }
    } else if let Some(signal) = status.signal() {
        TerminalStatus::Signaled {
            signal: signal as u32,
        }
    } else {
        TerminalStatus::Error {
            slug: "unknown-exit".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::terminal_wire::TerminalSize;
    use d2b_contracts::unsafe_local_wire::{
        HelperTerminalChunkBase64, HelperTerminalControl, HelperTerminalReadOutput,
        HelperTerminalResize, HelperTerminalWait, HelperTerminalWriteStdin,
    };

    #[test]
    fn supervisor_spec_debug_redacts_every_sensitive_field() {
        let canary = "private-supervisor-canary";
        let spec = ShellSupervisorSpec {
            supervisor_id: HelperSupervisorId::new(canary).unwrap(),
            runtime_directory: PathBuf::from(format!("/{canary}")),
            environment: BTreeMap::from([("PRIVATE".to_owned(), canary.to_owned())]),
            cwd: PathBuf::from(format!("/{canary}")),
            initial_rows: 24,
            initial_cols: 80,
            output_ring_bytes: 1024,
        };
        assert!(!format!("{spec:?}").contains(canary));
    }

    #[test]
    fn input_offsets_and_control_sequences_are_strictly_monotonic() {
        let master =
            openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).unwrap();
        let state = SupervisorState::new(master, Arc::new(OutputRing::new(1024).unwrap()));
        let protocol = Mutex::new(AttachmentProtocolState::default());

        let mismatch = HelperTerminalRequest::WriteStdin(HelperTerminalWriteStdin {
            request_id: 1,
            offset: 1,
            chunk_base64: HelperTerminalChunkBase64::new("").unwrap(),
            eof: false,
        });
        assert!(matches!(
            process_terminal_request(mismatch, &state, &protocol, 1).0,
            HelperTerminalResponse::Rejected(HelperTerminalRejected {
                code: HelperFailureCode::TerminalOffsetMismatch,
                ..
            })
        ));

        let first = HelperTerminalRequest::Resize(HelperTerminalResize {
            request_id: 2,
            control_sequence: 1,
            rows: 24,
            cols: 80,
        });
        let _ = process_terminal_request(first, &state, &protocol, 1);
        let duplicate = HelperTerminalRequest::CloseStdin(HelperTerminalControl {
            request_id: 3,
            control_sequence: 1,
        });
        assert!(matches!(
            process_terminal_request(duplicate, &state, &protocol, 1).0,
            HelperTerminalResponse::Rejected(HelperTerminalRejected {
                code: HelperFailureCode::InvalidRequest,
                ..
            })
        ));
    }

    #[test]
    fn only_observation_requests_may_run_concurrently() {
        let chunk = d2b_contracts::unsafe_local_wire::HelperTerminalChunkBase64::new("").unwrap();
        assert!(!terminal_request_may_run_concurrently(
            &HelperTerminalRequest::WriteStdin(HelperTerminalWriteStdin {
                request_id: 1,
                offset: 0,
                chunk_base64: chunk,
                eof: false,
            })
        ));
        assert!(terminal_request_may_run_concurrently(
            &HelperTerminalRequest::ReadOutput(HelperTerminalReadOutput {
                request_id: 2,
                stream: TerminalStream::Stdout,
                cursor: 0,
                max_len: 1,
                wait: false,
                timeout_ms: 0,
            })
        ));
        assert!(terminal_request_may_run_concurrently(
            &HelperTerminalRequest::Wait(HelperTerminalWait {
                request_id: 3,
                timeout_ms: 0,
            })
        ));
        assert!(!terminal_request_may_run_concurrently(
            &HelperTerminalRequest::Resize(HelperTerminalResize {
                request_id: 4,
                control_sequence: 1,
                rows: 24,
                cols: 80,
            })
        ));
    }

    #[test]
    fn supervisor_attach_protocol_carries_no_name_or_path() {
        let request = SupervisorRequest {
            version: SUPERVISOR_PROTOCOL_VERSION,
            request_id: 1,
            action: SupervisorAction::Attach {
                force: false,
                initial_terminal_size: TerminalSize { rows: 24, cols: 80 },
            },
        };
        let encoded = serde_json::to_string(&request).unwrap();
        for forbidden in ["name", "path", "pid", "unit", "argv", "environment"] {
            assert!(!encoded.contains(forbidden));
        }
    }
}
