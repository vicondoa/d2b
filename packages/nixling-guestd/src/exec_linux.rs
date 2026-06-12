//! Production Linux process spawner for the guest exec runtime.
//!
//! Spawns each exec as a leader of its own process group via the safe
//! `tokio::process::Command::process_group` API, with stdin redirected to
//! `/dev/null`, stdout/stderr captured on separate pipes, and the environment
//! rebuilt from the validated request (no inherited host environment, no
//! shell, no PATH-based lookup). Group termination uses `rustix`'s safe
//! `kill_process_group`. No first-party `unsafe` is used.

use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use rustix::process::{kill_process_group, Pid, Signal};
use tokio::process::{Child, Command};

use crate::exec::{
    ExecError, ExitOutcome, ProcessKiller, ProcessSpawner, ProcessWaiter, SpawnedProcess,
    ValidatedCommand,
};

/// Spawns validated commands into their own process group.
#[derive(Default)]
pub struct LinuxProcessSpawner;

#[async_trait]
impl ProcessSpawner for LinuxProcessSpawner {
    async fn spawn(&self, command: ValidatedCommand) -> Result<SpawnedProcess, ExecError> {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args)
            .current_dir(&command.cwd)
            .env_clear()
            .envs(command.env.iter().map(|(k, v)| (k.clone(), v.clone())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // New process group with the child as leader (pgid == pid), so the
            // whole subtree can be signalled as a unit.
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

