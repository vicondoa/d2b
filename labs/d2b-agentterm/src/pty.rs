//! The child process and its pseudoterminal.
//!
//! Two deliberate departures from `ht`, whose PTY layer this replaces rather
//! than vendors:
//!
//! 1. **argv is preserved as a vector.** `ht` joins arguments with spaces and
//!    re-parses them through `/bin/sh -c`, which destroys quoting: `ht echo
//!    "a  b"` reaches the child as two arguments. We `execvpe` the vector
//!    directly.
//! 2. **The master descriptor has exactly one owner.** `ht` builds both a
//!    `File::from_raw_fd` and an `AsyncFd<OwnedFd>` over the same descriptor,
//!    so both try to close it. Here `AsyncFd<OwnedFd>` is the sole owner.
//!
//! Resize propagation is the third difference and the most consequential: `ht`
//! resizes only its virtual terminal and never issues `TIOCSWINSZ`, so the
//! child keeps its original size for the life of the session. See
//! [`Pty::resize`].

use std::collections::HashMap;
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::pty::{ForkptyResult, forkpty};
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, execvpe};
use tokio::io::unix::AsyncFd;

use crate::tty::TtySize;

/// A running child attached to a PTY master.
pub struct Pty {
    child: Pid,
    master: AsyncFd<OwnedFd>,
}

impl Pty {
    /// Fork a child on a new PTY and exec `command` in it.
    ///
    /// `command[0]` is resolved through `PATH`.
    pub fn spawn(
        command: &[String],
        size: TtySize,
        extra_env: &HashMap<String, String>,
    ) -> anyhow::Result<Self> {
        if command.is_empty() {
            anyhow::bail!("no command given");
        }

        let winsize = size.to_winsize();

        // Build argv and envp before forking. Allocation after fork in a
        // multi-threaded process is not async-signal-safe, so everything the
        // child needs must already exist.
        let argv = to_cstrings(command)?;
        let envp = build_env(extra_env)?;

        // SAFETY: forkpty is called from the parent before any child-side
        // allocation. The child branch below performs only execvpe and _exit,
        // both async-signal-safe, using vectors built above the fork.
        let result = unsafe { forkpty(Some(&winsize), None) }?;

        match result {
            ForkptyResult::Parent { child, master } => {
                set_nonblocking(&master)?;
                Ok(Self {
                    child,
                    master: AsyncFd::new(master)?,
                })
            }

            ForkptyResult::Child => {
                // Restore SIGPIPE. Rust's runtime ignores it, but a child that
                // inherits SIG_IGN misbehaves in a pipeline.
                // SAFETY: setting a disposition to the default handler in the
                // freshly forked child, before exec.
                let _ = unsafe { signal(Signal::SIGPIPE, SigHandler::SigDfl) };

                let _ = execvpe(&argv[0], &argv, &envp);

                // execvpe only returns on failure. The parent will observe the
                // exit status; there is nothing safe to print from here.
                unsafe { libc::_exit(127) }
            }
        }
    }

    pub fn child_pid(&self) -> i32 {
        self.child.as_raw()
    }

    /// Propagate a new window size to the child.
    ///
    /// This is the `TIOCSWINSZ` that `ht` omits. Without it a full-screen TUI
    /// renders at the size it saw at startup and never reflows, which makes
    /// every resize test fail in a way that looks like an emulator bug.
    pub fn resize(&self, size: TtySize) -> io::Result<()> {
        let ws = size.to_winsize();
        // SAFETY: `ws` is a live winsize and the fd is owned by `self`.
        let rc = unsafe {
            libc::ioctl(
                self.master.get_ref().as_raw_fd(),
                libc::TIOCSWINSZ,
                &raw const ws,
            )
        };

        if rc < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    /// Read child output.
    ///
    /// A PTY master reports `EIO` rather than end-of-file once the last slave
    /// closes, so that is translated to `Ok(0)`.
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.master.readable().await?;
            match guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                // SAFETY: `buf` is a live uniquely borrowed slice; `fd` is
                // owned by `self`.
                let n =
                    unsafe { libc::read(fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(Err(err)) if err.raw_os_error() == Some(libc::EIO) => return Ok(0),
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    /// Write to the child's input.
    pub async fn write(&self, buf: &[u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.master.writable().await?;
            match guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                // SAFETY: `buf` is a live borrowed slice; `fd` is owned by
                // `self`.
                let n = unsafe { libc::write(fd, buf.as_ptr().cast::<libc::c_void>(), buf.len()) };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    /// Ask the child to terminate.
    pub fn hangup(&self) {
        let _ = nix::sys::signal::kill(self.child, Signal::SIGHUP);
    }

    /// Reap the child, returning its exit status if it has finished.
    pub fn try_wait(&self) -> Option<i32> {
        match waitpid(self.child, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(_, code)) => Some(code),
            Ok(WaitStatus::Signaled(_, sig, _)) => Some(128 + sig as i32),
            _ => None,
        }
    }

    /// Block until the child exits, returning its status.
    pub fn wait(&self) -> Option<i32> {
        match waitpid(self.child, None) {
            Ok(WaitStatus::Exited(_, code)) => Some(code),
            Ok(WaitStatus::Signaled(_, sig, _)) => Some(128 + sig as i32),
            _ => None,
        }
    }
}

fn set_nonblocking(fd: &OwnedFd) -> anyhow::Result<()> {
    let flags = fcntl(fd, FcntlArg::F_GETFL)?;
    let mut flags = OFlag::from_bits_truncate(flags);
    flags.insert(OFlag::O_NONBLOCK);
    fcntl(fd, FcntlArg::F_SETFL(flags))?;
    Ok(())
}

fn to_cstrings(items: &[String]) -> anyhow::Result<Vec<CString>> {
    items
        .iter()
        .map(|s| CString::new(s.as_bytes()).map_err(anyhow::Error::from))
        .collect()
}

/// Build the child's environment: the current environment, plus overrides.
///
/// Using `execvpe` with an explicit environment avoids `std::env::set_var`,
/// which is `unsafe` in edition 2024 and genuinely unsound to call after a fork
/// in a multi-threaded process.
fn build_env(extra: &HashMap<String, String>) -> anyhow::Result<Vec<CString>> {
    let mut merged: HashMap<String, String> = std::env::vars().collect();
    for (key, value) in extra {
        merged.insert(key.clone(), value.clone());
    }

    let mut out: Vec<String> = merged
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    // Deterministic order keeps the child's environment reproducible.
    out.sort();

    to_cstrings(&out)
}

#[cfg(test)]
mod tests {
    use super::{build_env, to_cstrings};
    use std::collections::HashMap;

    #[test]
    fn argv_is_preserved_verbatim() {
        // The case ht's join-and-reshell corrupts: an argument containing
        // whitespace must survive as one argument.
        let argv = vec!["echo".to_string(), "a  b".to_string()];
        let c = to_cstrings(&argv).unwrap_or_default();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].to_bytes(), b"a  b");
    }

    #[test]
    fn argv_rejects_interior_nul() {
        let argv = vec!["echo".to_string(), "a\0b".to_string()];
        assert!(to_cstrings(&argv).is_err());
    }

    #[test]
    fn extra_env_overrides_inherited() {
        let mut extra = HashMap::new();
        extra.insert("TERM".to_string(), "xterm-256color".to_string());

        let env = build_env(&extra).unwrap_or_default();
        let rendered: Vec<String> = env
            .iter()
            .map(|c| String::from_utf8_lossy(c.to_bytes()).into_owned())
            .collect();

        assert_eq!(
            rendered.iter().filter(|e| e.starts_with("TERM=")).count(),
            1
        );
        assert!(rendered.contains(&"TERM=xterm-256color".to_string()));
    }

    #[test]
    fn env_is_sorted_for_determinism() {
        let env = build_env(&HashMap::new()).unwrap_or_default();
        let rendered: Vec<String> = env
            .iter()
            .map(|c| String::from_utf8_lossy(c.to_bytes()).into_owned())
            .collect();
        let mut sorted = rendered.clone();
        sorted.sort();
        assert_eq!(rendered, sorted);
    }
}
