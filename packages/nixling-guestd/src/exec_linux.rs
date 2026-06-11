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
use std::sync::Mutex;

use async_trait::async_trait;
use rustix::process::{kill_process_group, Pid, Signal};
use tokio::process::{Child, Command};

use crate::exec::{
    ExecError, ExitOutcome, ProcessHandle, ProcessSpawner, SpawnedProcess, ValidatedCommand,
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
            // whole subtree can be signalled and the leader anchors the PGID.
            .process_group(0)
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|_| ExecError::SpawnFailed)?;
        let pid = child.id().ok_or(ExecError::SpawnFailed)? as i32;
        let stdout = child.stdout.take().ok_or(ExecError::SpawnFailed)?;
        let stderr = child.stderr.take().ok_or(ExecError::SpawnFailed)?;

        let handle = LinuxProcessHandle {
            pgid: pid,
            child: Mutex::new(Some(child)),
        };
        Ok(SpawnedProcess {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            handle: Box::new(handle),
        })
    }
}

struct LinuxProcessHandle {
    pgid: i32,
    child: Mutex<Option<Child>>,
}

#[async_trait]
impl ProcessHandle for LinuxProcessHandle {
    async fn wait(&mut self) -> ExitOutcome {
        let mut child = {
            let mut guard = self
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.take()
        };
        match child.as_mut() {
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

    fn kill_group(&self) {
        // Idempotent best-effort group termination. A process group persists
        // while any member is alive, so the PGID cannot be reused underneath
        // this signal as long as descendants remain; once the group is empty
        // the signal is a harmless ESRCH no-op.
        if let Some(pid) = Pid::from_raw(self.pgid) {
            let _ = kill_process_group(pid, Signal::Kill);
        }
    }

    async fn reap(&mut self) {
        // tokio reaps the direct child inside `wait`. Any remaining handle is
        // dropped here, which (with kill_on_drop) also tears down a child that
        // was never waited on.
        let _ = {
            let mut guard = self
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.take()
        };
    }
}
