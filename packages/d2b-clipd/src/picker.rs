//! Picker process lifecycle for the authenticated local service.
//!
//! The supervisor passes a connected transport descriptor to the picker. The
//! service composition layer performs the ComponentSession handshake on that
//! transport; this module never interprets packets and has no pathname or
//! legacy frame fallback.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use command_fds::{CommandFdExt, FdMapping};
use thiserror::Error;

use crate::policy::ReasonCode;
use crate::protocol::OpaquePickerId;

pub const PICKER_SESSION_FD: i32 = 3;
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
    pub session_fd_number: i32,
    pub child_session_fd: OwnedFd,
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
                parent_fd: launch.child_session_fd,
                child_fd: launch.session_fd_number,
            }])
            .map_err(|_| PickerError::Launch)?;
        command.spawn().map_err(|_| PickerError::Launch)
    }
}

pub trait PickerSpawner {
    type Process: PickerProcess;

    fn spawn(&mut self, launch: PickerLaunch) -> Result<Self::Process, PickerError>;
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum PickerError {
    #[error("clipboard-picker-not-configured")]
    NotConfigured,
    #[error("clipboard-picker-forbidden-environment")]
    ForbiddenEnvironment,
    #[error("clipboard-picker-already-active")]
    AlreadyActive,
    #[error("clipboard-picker-session-transport-failed")]
    SessionTransport,
    #[error("clipboard-picker-launch-failed")]
    Launch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerState {
    Idle,
    Active {
        selection_id: OpaquePickerId,
        pid: u32,
        deadline: Instant,
    },
}

#[derive(Debug)]
pub struct ActivePicker<P> {
    selection_id: OpaquePickerId,
    deadline: Instant,
    process: P,
    parent_session: UnixStream,
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
                selection_id: active.selection_id.clone(),
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

    pub fn active_session_transport(&self) -> Option<&UnixStream> {
        self.active.as_ref().map(|active| &active.parent_session)
    }

    pub fn launch(
        &mut self,
        selection_id: OpaquePickerId,
        command: Option<PickerCommand>,
        ambient_env: &BTreeMap<OsString, OsString>,
        timeout: Duration,
    ) -> Result<&UnixStream, PickerError> {
        if self.active.is_some() {
            return Err(PickerError::AlreadyActive);
        }
        if timeout.is_zero() {
            return Err(PickerError::SessionTransport);
        }
        let command = command.ok_or(PickerError::NotConfigured)?;
        let (parent_session, child_session) =
            UnixStream::pair().map_err(|_| PickerError::SessionTransport)?;
        parent_session
            .set_nonblocking(true)
            .map_err(|_| PickerError::SessionTransport)?;
        let child_session_fd = OwnedFd::from(child_session);
        let session_fd_number = PICKER_SESSION_FD;
        let argv = picker_argv(&command, session_fd_number);
        let env = sanitize_picker_env(ambient_env);
        assert_picker_env_excludes_niri_socket(&env)?;
        let deadline = Instant::now() + timeout;
        let launch = PickerLaunch {
            command,
            argv,
            env,
            session_fd_number,
            child_session_fd,
        };
        let process = self.spawner.spawn(launch)?;
        self.active = Some(ActivePicker {
            selection_id,
            deadline,
            process,
            parent_session,
        });
        Ok(&self.active.as_ref().expect("active").parent_session)
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

    pub fn reap_expired(&mut self, now: Instant) -> Option<ReasonCode> {
        let active = self.active.as_ref()?;
        if now < active.deadline {
            return None;
        }
        let mut active = self.active.take().expect("active");
        active.process.terminate();
        self.terminating.push(TerminatingPicker {
            process: active.process,
            kill_at: now + PICKER_TERMINATE_GRACE,
            killed: false,
        });
        Some(ReasonCode::PickerTimeout)
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
        Err(PickerError::ForbiddenEnvironment)
    } else {
        Ok(())
    }
}

pub fn picker_argv(command: &PickerCommand, actual_child_fd: i32) -> Vec<OsString> {
    let mut argv = command.args.clone();
    argv.push(OsString::from(format!(
        "--component-session-fd={actual_child_fd}"
    )));
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
            let flags =
                rustix::io::fcntl_getfd(&launch.child_session_fd).expect("session fd flags");
            assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));
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

    fn selection_id() -> OpaquePickerId {
        OpaquePickerId::parse("selection-1").unwrap()
    }

    #[test]
    fn environment_is_bounded_and_excludes_niri_socket() {
        let ambient = BTreeMap::from([
            (
                OsString::from("WAYLAND_DISPLAY"),
                OsString::from("wayland-1"),
            ),
            (
                OsString::from("NIRI_SOCKET"),
                OsString::from("/run/user/1000/niri.sock"),
            ),
            (OsString::from("SECRET_TOKEN"), OsString::from("secret")),
        ]);

        let sanitized = sanitize_picker_env(&ambient);
        assert!(sanitized.contains_key(OsStr::new("WAYLAND_DISPLAY")));
        assert!(!sanitized.contains_key(OsStr::new("NIRI_SOCKET")));
        assert!(!sanitized.contains_key(OsStr::new("SECRET_TOKEN")));
        assert_picker_env_excludes_niri_socket(&sanitized).unwrap();
    }

    #[test]
    fn supervisor_exposes_only_component_session_transport() {
        let mut supervisor = PickerSupervisor::new(FakeSpawner::default());
        supervisor
            .launch(
                selection_id(),
                Some(command()),
                &BTreeMap::new(),
                Duration::from_secs(1),
            )
            .unwrap();
        assert!(supervisor.active_session_transport().is_some());
        assert!(matches!(
            supervisor.state(),
            PickerState::Active { pid: 42, .. }
        ));
        assert_eq!(
            supervisor
                .launch(
                    selection_id(),
                    Some(command()),
                    &BTreeMap::new(),
                    Duration::from_secs(1),
                )
                .unwrap_err(),
            PickerError::AlreadyActive
        );
    }

    #[test]
    fn launch_argument_has_no_path_token_or_legacy_frame_flag() {
        let argv = picker_argv(&command(), 9);
        assert!(argv.iter().any(|arg| arg == "--component-session-fd=9"));
        assert!(!argv.iter().any(|arg| {
            let arg = arg.to_string_lossy();
            arg.contains("token") || arg.contains("socket") || arg.contains("--ipc-fd")
        }));
    }

    #[test]
    fn cancellation_and_timeout_are_bounded() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let spawner = FakeSpawner {
            last_launch: None,
            events: Rc::clone(&events),
        };
        let mut supervisor = PickerSupervisor::new(spawner);
        supervisor
            .launch(
                selection_id(),
                Some(command()),
                &BTreeMap::new(),
                Duration::from_millis(1),
            )
            .unwrap();
        assert_eq!(
            supervisor.reap_expired(Instant::now() + Duration::from_secs(1)),
            Some(ReasonCode::PickerTimeout)
        );
        assert_eq!(&*events.borrow(), &["terminate"]);
        supervisor.reap_terminated(Instant::now() + PICKER_TERMINATE_GRACE);
        assert_eq!(&*events.borrow(), &["terminate", "try_reap"]);
    }
}
