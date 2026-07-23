use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use command_fds::{CommandFdExt, FdMapping};

use crate::framing::{FramingError, decode_frame};
use crate::policy::ReasonCode;
use crate::protocol::PickerToDaemonMessage;
use thiserror::Error;

pub const PICKER_IPC_FD: i32 = 3;
pub const PICKER_ENV_ALLOWLIST: &[&str] = &[
    "WAYLAND_DISPLAY",
    "WAYLAND_SOCKET",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_DIRS",
    "GDK_BACKEND",
    "GTK_THEME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerCommand {
    pub program: OsString,
    pub args: Vec<OsString>,
}

#[derive(Debug)]
pub struct PickerLaunch {
    pub command: PickerCommand,
    pub argv: Vec<OsString>,
    pub env: BTreeMap<OsString, OsString>,
    pub ipc_fd_number: i32,
    pub child_ipc_fd: OwnedFd,
}

pub trait PickerProcess {
    fn id(&self) -> u32;
    fn terminate(&mut self);
    fn kill(&mut self);
    fn try_reap(&mut self) -> bool;
}

impl PickerProcess for Child {
    fn id(&self) -> u32 {
        Child::id(self)
    }

    fn terminate(&mut self) {
        if let Some(pid) = rustix::process::Pid::from_raw(self.id() as i32) {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::Term);
        }
    }

    fn kill(&mut self) {
        let _ = Child::kill(self);
    }

    fn try_reap(&mut self) -> bool {
        matches!(self.try_wait(), Ok(Some(_)) | Err(_))
    }
}

const PICKER_TERMINATE_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Default)]
pub struct CommandPickerSpawner;

impl PickerSpawner for CommandPickerSpawner {
    type Process = Child;

    fn spawn(&mut self, launch: PickerLaunch) -> Result<Self::Process, PickerError> {
        let mut command = Command::new(&launch.command.program);
        command.args(&launch.argv);
        command.env_clear();
        command.envs(&launch.env);
        command
            .fd_mappings(vec![FdMapping {
                parent_fd: launch.child_ipc_fd,
                child_fd: launch.ipc_fd_number,
            }])
            .map_err(|err| PickerError::FdFlags(err.to_string()))?;
        command
            .spawn()
            .map_err(|err| PickerError::Spawn(err.to_string()))
    }
}

pub trait PickerSpawner {
    type Process: PickerProcess;

    fn spawn(&mut self, launch: PickerLaunch) -> Result<Self::Process, PickerError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PickerError {
    #[error("picker is not configured")]
    NotConfigured,
    #[error("picker environment must not include {name}")]
    ForbiddenEnvironment { name: &'static str },
    #[error("picker is already active for request {request_id}")]
    AlreadyActive { request_id: String },
    #[error("failed to create picker socketpair: {0}")]
    Socketpair(String),
    #[error("picker spawn failed: {0}")]
    Spawn(String),
    #[error("picker fd flag update failed: {0}")]
    FdFlags(String),
    #[error("picker frame error: {0}")]
    Frame(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerPoll {
    Message(PickerToDaemonMessage),
    Incomplete,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerState {
    Idle,
    Active {
        request_id: String,
        pid: u32,
        deadline: Instant,
    },
}

#[derive(Debug)]
pub struct ActivePicker<P> {
    request_id: String,
    deadline: Instant,
    process: P,
    parent_socket: UnixStream,
    read_buffer: Vec<u8>,
}

#[derive(Debug)]
struct TerminatingPicker<P> {
    process: P,
    kill_at: Instant,
    killed: bool,
}

#[derive(Debug)]
pub struct PickerSupervisor<S: PickerSpawner> {
    spawner: S,
    active: Option<ActivePicker<S::Process>>,
    terminating: Vec<TerminatingPicker<S::Process>>,
}

impl<S: PickerSpawner> PickerSupervisor<S> {
    pub fn new(spawner: S) -> Self {
        Self {
            spawner,
            active: None,
            terminating: Vec::new(),
        }
    }

    pub fn state(&self) -> PickerState {
        match &self.active {
            Some(active) => PickerState::Active {
                request_id: active.request_id.clone(),
                pid: active.process.id(),
                deadline: active.deadline,
            },
            None => PickerState::Idle,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.active.as_ref().map(|active| active.deadline)
    }

    pub fn maintenance_deadline(&self) -> Option<Instant> {
        self.terminating
            .iter()
            .filter(|picker| !picker.killed)
            .map(|picker| picker.kill_at)
            .min()
    }

    pub fn active_socket(&self) -> Option<&UnixStream> {
        self.active.as_ref().map(|active| &active.parent_socket)
    }

    pub fn launch(
        &mut self,
        request_id: String,
        command: Option<PickerCommand>,
        ambient_env: &BTreeMap<OsString, OsString>,
        timeout: Duration,
    ) -> Result<&UnixStream, PickerError> {
        if let Some(active) = &self.active {
            return Err(PickerError::AlreadyActive {
                request_id: active.request_id.clone(),
            });
        }
        let command = command.ok_or(PickerError::NotConfigured)?;
        let (parent_socket, child_socket) =
            UnixStream::pair().map_err(|err| PickerError::Socketpair(err.to_string()))?;
        parent_socket
            .set_nonblocking(true)
            .map_err(|err| PickerError::Socketpair(err.to_string()))?;
        let child_ipc_fd = OwnedFd::from(child_socket);
        let child_fd_number = PICKER_IPC_FD;
        let argv = picker_argv(&command, child_fd_number);
        let env = sanitize_picker_env(ambient_env);
        let deadline = Instant::now() + timeout;
        let launch = PickerLaunch {
            command,
            argv,
            env,
            ipc_fd_number: child_fd_number,
            child_ipc_fd,
        };
        let process = self.spawner.spawn(launch)?;
        self.active = Some(ActivePicker {
            request_id,
            deadline,
            process,
            parent_socket,
            read_buffer: Vec::new(),
        });
        Ok(&self.active.as_ref().expect("active").parent_socket)
    }

    pub fn poll_active(&mut self, max_frame_bytes: usize) -> Result<PickerPoll, PickerError> {
        let Some(active) = self.active.as_mut() else {
            return Ok(PickerPoll::Incomplete);
        };
        if active.read_buffer.contains(&b'\n') {
            return Self::decode_next_picker_frame(active, max_frame_bytes);
        }
        loop {
            let mut buf = [0_u8; 512];
            match active.parent_socket.read(&mut buf) {
                Ok(0) if active.read_buffer.is_empty() => return Ok(PickerPoll::Closed),
                Ok(0) => {
                    return Err(PickerError::Frame(
                        "picker closed with incomplete frame".to_owned(),
                    ));
                }
                Ok(n) => {
                    active.read_buffer.extend_from_slice(&buf[..n]);
                    if let Some(newline) = active.read_buffer.iter().position(|byte| *byte == b'\n')
                    {
                        if newline > max_frame_bytes {
                            return Err(PickerError::Frame(
                                FramingError::FrameTooLong {
                                    max: max_frame_bytes,
                                }
                                .to_string(),
                            ));
                        }
                        break;
                    }
                    if active.read_buffer.len() > max_frame_bytes {
                        return Err(PickerError::Frame(
                            FramingError::FrameTooLong {
                                max: max_frame_bytes,
                            }
                            .to_string(),
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(PickerError::Frame(error.to_string())),
            }
        }

        Self::decode_next_picker_frame(active, max_frame_bytes)
    }

    pub fn cancel_active(&mut self, reason: ReasonCode) -> Option<ReasonCode> {
        let mut active = self.active.take()?;
        active.process.terminate();
        self.terminating.push(TerminatingPicker {
            process: active.process,
            kill_at: Instant::now() + PICKER_TERMINATE_GRACE,
            killed: false,
        });
        Some(reason)
    }

    fn decode_next_picker_frame<P: PickerProcess>(
        active: &mut ActivePicker<P>,
        max_frame_bytes: usize,
    ) -> Result<PickerPoll, PickerError> {
        let Some(newline) = active.read_buffer.iter().position(|byte| *byte == b'\n') else {
            return Ok(PickerPoll::Incomplete);
        };
        if newline > max_frame_bytes {
            return Err(PickerError::Frame(
                FramingError::FrameTooLong {
                    max: max_frame_bytes,
                }
                .to_string(),
            ));
        }
        let frame = active.read_buffer.drain(..=newline).collect::<Vec<_>>();
        let message = decode_frame::<PickerToDaemonMessage>(&frame, max_frame_bytes)
            .map_err(|err| PickerError::Frame(err.to_string()))?;
        Ok(PickerPoll::Message(message))
    }

    pub fn reap_expired(&mut self, now: Instant) -> Option<ReasonCode> {
        let active = self.active.as_ref()?;
        if now >= active.deadline {
            let mut active = self.active.take().expect("active");
            active.process.terminate();
            self.terminating.push(TerminatingPicker {
                process: active.process,
                kill_at: now + PICKER_TERMINATE_GRACE,
                killed: false,
            });
            Some(ReasonCode::PickerTimeout)
        } else {
            None
        }
    }

    pub fn reap_terminated(&mut self, now: Instant) {
        let mut still_running = Vec::new();
        for mut picker in self.terminating.drain(..) {
            if picker.process.try_reap() {
                continue;
            }
            if now >= picker.kill_at && !picker.killed {
                picker.process.kill();
                picker.killed = true;
            }
            if !picker.process.try_reap() {
                still_running.push(picker);
            }
        }
        self.terminating = still_running;
    }
}

pub fn sanitize_picker_env(ambient: &BTreeMap<OsString, OsString>) -> BTreeMap<OsString, OsString> {
    let allowlist: BTreeSet<&OsStr> = PICKER_ENV_ALLOWLIST.iter().map(OsStr::new).collect();
    ambient
        .iter()
        .filter(|(key, _)| allowlist.contains(key.as_os_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub fn assert_picker_env_excludes_niri_socket(
    picker_env: &BTreeMap<OsString, OsString>,
) -> Result<(), PickerError> {
    if picker_env.contains_key(OsStr::new("NIRI_SOCKET")) {
        Err(PickerError::ForbiddenEnvironment {
            name: "NIRI_SOCKET",
        })
    } else {
        Ok(())
    }
}

pub fn picker_argv(command: &PickerCommand, actual_child_fd: i32) -> Vec<OsString> {
    let mut argv = command.args.clone();
    argv.push(OsString::from(format!("--ipc-fd={actual_child_fd}")));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug, Default)]
    struct FakeProcess {
        pid: u32,
        events: Rc<RefCell<Vec<&'static str>>>,
        reap_after: usize,
        reap_calls: usize,
    }

    impl PickerProcess for FakeProcess {
        fn id(&self) -> u32 {
            self.pid
        }

        fn terminate(&mut self) {
            self.events.borrow_mut().push("terminate");
        }

        fn kill(&mut self) {
            self.events.borrow_mut().push("kill");
        }

        fn try_reap(&mut self) -> bool {
            self.events.borrow_mut().push("try_reap");
            self.reap_calls += 1;
            self.reap_calls >= self.reap_after
        }
    }

    #[derive(Debug, Default)]
    struct FakeSpawner {
        last_launch: Option<PickerLaunch>,
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl PickerSpawner for FakeSpawner {
        type Process = FakeProcess;

        fn spawn(&mut self, launch: PickerLaunch) -> Result<Self::Process, PickerError> {
            let flags = rustix::io::fcntl_getfd(&launch.child_ipc_fd).expect("child ipc fd flags");
            assert!(
                flags.contains(rustix::io::FdFlags::CLOEXEC),
                "parent must keep picker IPC fd CLOEXEC until child pre_exec"
            );
            self.last_launch = Some(launch);
            Ok(FakeProcess {
                pid: 42,
                events: Rc::clone(&self.events),
                reap_after: 1,
                reap_calls: 0,
            })
        }
    }

    fn command() -> PickerCommand {
        PickerCommand {
            program: OsString::from("fake-picker"),
            args: vec![OsString::from("--foreground")],
        }
    }

    #[test]
    fn environment_is_bounded_and_excludes_niri_socket() {
        let ambient = BTreeMap::from([
            (
                OsString::from("WAYLAND_DISPLAY"),
                OsString::from("wayland-1"),
            ),
            (
                OsString::from("XDG_RUNTIME_DIR"),
                OsString::from("/run/user/1000"),
            ),
            (
                OsString::from("NIRI_SOCKET"),
                OsString::from("/run/user/1000/niri.sock"),
            ),
            (OsString::from("SECRET_TOKEN"), OsString::from("secret")),
        ]);

        let sanitized = sanitize_picker_env(&ambient);
        assert_eq!(
            sanitized.get(OsStr::new("WAYLAND_DISPLAY")),
            Some(&OsString::from("wayland-1"))
        );
        assert_eq!(
            sanitized.get(OsStr::new("XDG_RUNTIME_DIR")),
            Some(&OsString::from("/run/user/1000"))
        );
        assert!(!sanitized.contains_key(OsStr::new("NIRI_SOCKET")));
        assert!(!sanitized.contains_key(OsStr::new("SECRET_TOKEN")));
        assert_picker_env_excludes_niri_socket(&sanitized).expect("no niri socket");
        let forbidden = BTreeMap::from([(OsString::from("NIRI_SOCKET"), OsString::from("sock"))]);
        assert!(matches!(
            assert_picker_env_excludes_niri_socket(&forbidden),
            Err(PickerError::ForbiddenEnvironment {
                name: "NIRI_SOCKET"
            })
        ));
    }

    #[test]
    fn supervisor_uses_socketpair_and_single_active_picker() {
        let spawner = FakeSpawner::default();
        let mut supervisor = PickerSupervisor::new(spawner);
        let ambient = BTreeMap::new();

        supervisor
            .launch(
                "req-1".to_owned(),
                Some(command()),
                &ambient,
                Duration::from_secs(1),
            )
            .expect("launch");
        assert!(matches!(
            supervisor.state(),
            PickerState::Active { pid: 42, .. }
        ));
        let err = supervisor
            .launch(
                "req-2".to_owned(),
                Some(command()),
                &ambient,
                Duration::from_secs(1),
            )
            .expect_err("single active");
        assert!(matches!(
            err,
            PickerError::AlreadyActive {
                request_id
            } if request_id == "req-1"
        ));
    }

    #[test]
    fn timeout_terminates_then_kills_picker() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let spawner = FakeSpawner {
            last_launch: None,
            events: Rc::clone(&events),
        };
        let mut supervisor = PickerSupervisor::new(spawner);
        supervisor
            .launch(
                "req-1".to_owned(),
                Some(command()),
                &BTreeMap::new(),
                Duration::from_millis(0),
            )
            .expect("launch");

        assert_eq!(
            supervisor.reap_expired(Instant::now()),
            Some(ReasonCode::PickerTimeout)
        );
        assert_eq!(&*events.borrow(), &["terminate"]);
        assert_eq!(supervisor.state(), PickerState::Idle);
        supervisor.reap_terminated(Instant::now() + PICKER_TERMINATE_GRACE);
        assert_eq!(&*events.borrow(), &["terminate", "try_reap"]);
    }

    #[test]
    fn cancel_terminates_picker_without_blocking_reap() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let spawner = FakeSpawner {
            last_launch: None,
            events: Rc::clone(&events),
        };
        let mut supervisor = PickerSupervisor::new(spawner);
        supervisor
            .launch(
                "req-1".to_owned(),
                Some(command()),
                &BTreeMap::new(),
                Duration::from_secs(1),
            )
            .expect("launch");

        assert_eq!(
            supervisor.cancel_active(ReasonCode::PolicyDenied),
            Some(ReasonCode::PolicyDenied)
        );
        assert_eq!(&*events.borrow(), &["terminate"]);
        assert_eq!(supervisor.state(), PickerState::Idle);
    }

    #[test]
    fn terminating_picker_gets_kill_only_after_grace_deadline() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let spawner = FakeSpawner {
            last_launch: None,
            events: Rc::clone(&events),
        };
        let mut supervisor = PickerSupervisor::new(spawner);
        supervisor
            .launch(
                "req-1".to_owned(),
                Some(command()),
                &BTreeMap::new(),
                Duration::from_millis(0),
            )
            .expect("launch");
        supervisor
            .active
            .as_mut()
            .expect("active")
            .process
            .reap_after = usize::MAX;

        assert_eq!(
            supervisor.reap_expired(Instant::now()),
            Some(ReasonCode::PickerTimeout)
        );
        assert_eq!(&*events.borrow(), &["terminate"]);
        let kill_at = supervisor.maintenance_deadline().expect("maintenance");
        supervisor.reap_terminated(kill_at - Duration::from_millis(1));
        assert_eq!(&*events.borrow(), &["terminate", "try_reap", "try_reap"]);
        supervisor.reap_terminated(kill_at);
        assert_eq!(
            &*events.borrow(),
            &[
                "terminate",
                "try_reap",
                "try_reap",
                "try_reap",
                "kill",
                "try_reap"
            ]
        );
    }

    #[test]
    fn argv_uses_ipc_fd_flag_and_no_token() {
        let argv = picker_argv(&command(), 9);
        assert!(argv.iter().any(|arg| arg == "--ipc-fd=9"));
        assert!(
            !argv
                .iter()
                .any(|arg| arg.to_string_lossy().contains("token"))
        );
    }

    #[test]
    fn buffered_complete_frame_is_decoded_before_eof() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let process = FakeProcess {
            pid: 7,
            events,
            reap_after: 1,
            reap_calls: 0,
        };
        let (parent_socket, child_socket) = UnixStream::pair().expect("pair");
        parent_socket.set_nonblocking(true).expect("nonblocking");
        let mut supervisor = PickerSupervisor::<FakeSpawner> {
            spawner: FakeSpawner::default(),
            active: Some(ActivePicker {
                request_id: "req".to_owned(),
                deadline: Instant::now() + Duration::from_secs(30),
                process,
                parent_socket,
                read_buffer: b"{\"type\":\"cancel\",\"selected_protocol_version\":1,\"request_id\":\"req\"}\n"
                    .to_vec(),
            }),
            terminating: Vec::new(),
        };
        drop(child_socket);
        assert!(matches!(
            supervisor.poll_active(4096).expect("frame"),
            PickerPoll::Message(PickerToDaemonMessage::Cancel(_))
        ));
    }

    #[test]
    fn frame_length_uses_first_newline_not_total_buffer() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let process = FakeProcess {
            pid: 7,
            events,
            reap_after: 1,
            reap_calls: 0,
        };
        let (parent_socket, _child_socket) = UnixStream::pair().expect("pair");
        parent_socket.set_nonblocking(true).expect("nonblocking");
        let mut buffer =
            b"{\"type\":\"cancel\",\"selected_protocol_version\":1,\"request_id\":\"req\"}\n"
                .to_vec();
        buffer.extend(vec![b'x'; 5000]);
        let mut supervisor = PickerSupervisor::<FakeSpawner> {
            spawner: FakeSpawner::default(),
            active: Some(ActivePicker {
                request_id: "req".to_owned(),
                deadline: Instant::now() + Duration::from_secs(30),
                process,
                parent_socket,
                read_buffer: buffer,
            }),
            terminating: Vec::new(),
        };
        assert!(matches!(
            supervisor
                .poll_active(4096)
                .expect("first frame remains valid"),
            PickerPoll::Message(PickerToDaemonMessage::Cancel(_))
        ));
    }
}
