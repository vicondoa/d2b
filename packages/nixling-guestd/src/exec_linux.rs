//! Production Linux process spawner for the guest exec runtime.
//!
//! Non-interactive (attached, non-TTY) execs run the requested command as the
//! VM's host-fixed **workload user** (never root) inside a real PAM login
//! session, via `systemd-run --pipe --wait --property=PAMName=login
//! --uid=<user>`. `pam_systemd` provisions `XDG_RUNTIME_DIR` and the login
//! shell sources the profile, so the command sees the same environment an SSH
//! session would. stdout/stderr stream back over separate pipes and the
//! wrapper exit status carries the unit result. Group termination uses
//! `rustix`'s safe `kill_process_group`. No first-party `unsafe` is used.
//!
//! When no workload user is configured the spawner is disabled and every
//! non-TTY create fails closed (`ExecError::ExecDisabled`) — there is no
//! root-exec fallback.

use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use rustix::process::{kill_process_group, Pid, Signal};
use tokio::process::{Child, Command};

use crate::exec::{
    ExecError, ExitOutcome, ProcessKiller, ProcessSpawner, ProcessWaiter, SpawnedProcess,
    ValidatedCommand,
};
use crate::login_session::{login_session_systemd_run_args, SessionMode};

/// Host-fixed workload-user spawn configuration for the non-TTY path.
#[derive(Clone)]
pub struct WorkloadUserSpawn {
    pub systemd_run_path: PathBuf,
    pub login_shell_path: PathBuf,
    pub exec_user: String,
}

/// Spawns validated commands into their own process group, as the workload
/// user in a PAM login session. Disabled (no workload user) => fail closed.
#[derive(Default)]
pub struct LinuxProcessSpawner {
    workload: Option<WorkloadUserSpawn>,
}

impl LinuxProcessSpawner {
    /// Production constructor: run non-TTY execs as the workload user.
    pub fn new(workload: WorkloadUserSpawn) -> Self {
        Self {
            workload: Some(workload),
        }
    }

    /// Disabled spawner: no workload user configured, every non-TTY create
    /// fails closed. Used when exec is off and in tests that never reach a
    /// real non-TTY spawn.
    pub fn disabled() -> Self {
        Self { workload: None }
    }
}

#[async_trait]
impl ProcessSpawner for LinuxProcessSpawner {
    async fn spawn(&self, command: ValidatedCommand) -> Result<SpawnedProcess, ExecError> {
        let Some(workload) = self.workload.as_ref() else {
            // No workload user => non-TTY exec is not served (never root).
            return Err(ExecError::ExecDisabled);
        };

        let session_args = login_session_systemd_run_args(
            &workload.login_shell_path,
            &workload.exec_user,
            SessionMode::Pipe,
            &command,
        );

        // The direct child is `systemd-run --pipe --wait`: it forwards the
        // unit's stdout/stderr onto our captured pipes and exits with the
        // unit's result. The supervised command runs as the workload user in a
        // transient unit; the new process group lets us signal the wrapper as a
        // unit, and `--pipe` close on teardown propagates to the unit.
        let mut cmd = Command::new(&workload.systemd_run_path);
        cmd.args(&session_args)
            .current_dir("/")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|_| ExecError::SpawnFailed)?;
        let pid = child.id().ok_or(ExecError::SpawnFailed)? as i32;
        let stdout = child.stdout.take().ok_or(ExecError::SpawnFailed)?;
        let stderr = child.stderr.take().ok_or(ExecError::SpawnFailed)?;

        Ok(SpawnedProcess {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            killer: Arc::new(LinuxProcessKiller { pgid: pid }),
            waiter: Box::new(LinuxProcessWaiter { child: Some(child) }),
        })
    }
}

/// Lock-free, idempotent process-group terminator.
struct LinuxProcessKiller {
    pgid: i32,
}

impl ProcessKiller for LinuxProcessKiller {
    fn kill_group(&self) {
        // Idempotent best-effort group termination. A process group persists
        // while any member is alive, so the PGID cannot be reused underneath
        // this signal as long as descendants remain; once the group is empty
        // the signal is a harmless ESRCH no-op.
        if let Some(pid) = Pid::from_raw(self.pgid) {
            let _ = kill_process_group(pid, Signal::Kill);
        }
    }
}

/// Owns the direct child; `wait` reaps it (tokio semantics). Dropping the
/// waiter before `wait` completes tears the child down via `kill_on_drop`.
struct LinuxProcessWaiter {
    child: Option<Child>,
}

#[async_trait]
impl ProcessWaiter for LinuxProcessWaiter {
    async fn wait(&mut self) -> ExitOutcome {
        match self.child.as_mut() {
            Some(child) => match child.wait().await {
                Ok(status) => {
                    if let Some(code) = status.code() {
                        ExitOutcome::Exited(code)
                    } else if let Some(signal) = status.signal() {
                        ExitOutcome::Signaled(signal as u32)
                    } else {
                        ExitOutcome::Exited(-1)
                    }
                }
                Err(_) => ExitOutcome::Exited(-1),
            },
            None => ExitOutcome::Exited(-1),
        }
    }
}
