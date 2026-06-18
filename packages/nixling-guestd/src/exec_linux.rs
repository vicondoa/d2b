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
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::process::{Child, Command};

use crate::exec::{
    ExecError, ExitOutcome, ProcessKiller, ProcessSpawner, ProcessWaiter, SpawnedProcess,
    ValidatedCommand,
};
use crate::login_session::{
    SessionMode, login_session_systemd_run_args, sibling_systemctl_path, systemctl_kill_unit,
    unique_exec_unit_name,
};

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

        let unit_name = unique_exec_unit_name();
        let systemctl_path = sibling_systemctl_path(&workload.systemd_run_path);

        let session_args = login_session_systemd_run_args(
            &workload.login_shell_path,
            &workload.exec_user,
            &unit_name,
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
            killer: Arc::new(LinuxProcessKiller {
                pgid: pid,
                systemctl_path,
                unit_name,
            }),
            waiter: Box::new(LinuxProcessWaiter { child: Some(child) }),
        })
    }
}

/// Lock-free, idempotent process-group + transient-unit terminator.
struct LinuxProcessKiller {
    pgid: i32,
    /// Path to `systemctl`, used to SIGKILL the named transient unit's cgroup.
    systemctl_path: PathBuf,
    /// The `--unit=` name of the workload's transient unit.
    unit_name: String,
}

impl ProcessKiller for LinuxProcessKiller {
    fn kill_group(&self) {
        // 1. LOCAL first: SIGKILL the `systemd-run` wrapper's process group. This
        //    fires regardless of systemd health (teardown is never stranded by a
        //    wedged PID 1) and ensures the wrapper cannot issue a further
        //    StartTransientUnit once teardown has begun. A process group persists
        //    while any member is alive, so the PGID cannot be reused underneath
        //    this signal; once empty the signal is a harmless ESRCH no-op.
        if let Some(pid) = Pid::from_raw(self.pgid) {
            let _ = kill_process_group(pid, Signal::Kill);
        }
        // 2. SYSTEMD: the workload runs in a PID 1-owned transient unit, NOT in
        //    the wrapper's process group, so step 1 alone would leave a quiet
        //    non-TTY command (e.g. `sleep 3600`) running. SIGKILL the whole unit
        //    cgroup by name. Bounded + idempotent (see `systemctl_kill_unit`).
        systemctl_kill_unit(&self.systemctl_path, &self.unit_name);
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
