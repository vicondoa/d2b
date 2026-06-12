//! Interactive TTY exec helper (`--tty-exec`).
//!
//! guestd opens a PTY master/slave pair and spawns this helper with the slave
//! on stdin (fd 0) and an `O_CLOEXEC` status pipe on stdout (fd 1). The helper
//! runs entirely in safe `rustix` and `std` (no `unsafe`, no `pre_exec`):
//!
//! 1. `fcntl(F_DUPFD_CLOEXEC)` the status fd to a high number so the upcoming
//!    `dup2` onto fd 1 cannot clobber it and so it auto-closes on a successful
//!    `execve`,
//! 2. `setsid()` to become a session leader with no controlling terminal,
//! 3. `ioctl(TIOCSCTTY)` on the slave (fd 0) to acquire it as the controlling
//!    terminal of the new session,
//! 4. `tcsetwinsize` the initial rows/cols,
//! 5. `dup2` the slave onto fds 1 and 2 (it is already on fd 0),
//! 6. `execve` the target argv, inheriting the env/cwd guestd set on the helper
//!    `Command`.
//!
//! On success the `O_CLOEXEC` status fd closes during `execve`, which guestd
//! observes as EOF on the status pipe's read end. On any setup or `exec`
//! failure the helper writes exactly one typed status byte to that fd and
//! exits; guestd maps the byte to a typed `ExecCreate` error. The helper never
//! logs the target argv and never writes anything to the status fd on success.
//!
//! This module is the binary half of the crate and may use `rustix`; the
//! library half stays dependency-pure.

use std::os::unix::process::CommandExt;
use std::process::Command;

use rustix::io::{fcntl_dupfd_cloexec, write};
use rustix::process::{ioctl_tiocsctty, setsid};
use rustix::stdio::{dup2_stderr, dup2_stdout, stdin, stdout};
use rustix::termios::{tcsetwinsize, Winsize};

/// Lowest fd the inherited status pipe is duplicated to. Kept clear of the
/// standard fds (0/1/2) so the `dup2` of the slave onto stdout/stderr cannot
/// clobber the status channel.
const STATUS_DUP_MIN_FD: i32 = 10;

/// Exit code used when the status channel itself is unusable, so the helper
/// cannot signal a typed failure byte. guestd treats a closed status pipe with
/// no byte as a generic helper failure.
const EXIT_NO_STATUS_CHANNEL: i32 = 71;
/// Exit code accompanying any typed failure byte. The byte is authoritative;
/// the exit code is only a fallback for diagnostics.
const EXIT_SETUP_FAILED: i32 = 72;

/// Typed setup/exec failure. The single status byte guestd reads is the
/// `as_byte()` discriminant; the values are a stable wire contract between the
/// helper and guestd's PTY spawner. guestd maps ANY status byte to a typed
/// `ExecCreate` spawn/setup failure, and only a bare EOF (the `O_CLOEXEC` status
/// fd closing on a successful `execve`) to success — so every non-exec exit path
/// MUST write a byte first, or guestd would misread the exit as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelperFailure {
    /// `setsid()` failed (helper was unexpectedly already a group leader, or
    /// the kernel refused). guestd never sets `process_group(0)` on the helper
    /// precisely so this cannot happen in the supported path.
    Setsid = 1,
    /// `ioctl(TIOCSCTTY)` failed: the slave could not be acquired as the
    /// controlling terminal.
    Ctty = 2,
    /// `tcsetwinsize` failed applying the initial geometry.
    Winsize = 3,
    /// `dup2` of the slave onto stdout/stderr failed.
    Dup = 4,
    /// `execve` of the target failed (ENOENT/EACCES/ENOEXEC/...).
    Exec = 5,
    /// Argument parsing failed (malformed `--tty-exec` invocation). Reported
    /// over the still-attached status fd (no `dup2` has happened yet).
    Args = 6,
    /// Duplicating the inherited status fd to a high CLOEXEC fd failed, so the
    /// handshake fd could not be preserved across the slave wiring. Reported on
    /// the still-attached fd 1 before exit.
    StatusDup = 7,
}

impl HelperFailure {
    pub(crate) fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Parsed `--tty-exec --rows R --cols C -- <argv...>` arguments.
pub(crate) struct TtyHelperArgs {
    pub rows: u16,
    pub cols: u16,
    pub argv: Vec<String>,
}

impl TtyHelperArgs {
    /// Parse the args that follow `--tty-exec`. Grammar is fixed and
    /// order-sensitive: `--rows R --cols C -- <argv...>` with a non-empty,
    /// absolute argv. Returns `None` on any deviation (fail closed).
    pub(crate) fn parse(args: &[String]) -> Option<Self> {
        let mut rows: Option<u16> = None;
        let mut cols: Option<u16> = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--rows" => {
                    rows = Some(args.get(idx + 1)?.parse().ok()?);
                    idx += 2;
                }
                "--cols" => {
                    cols = Some(args.get(idx + 1)?.parse().ok()?);
                    idx += 2;
                }
                "--" => {
                    idx += 1;
                    break;
                }
                _ => return None,
            }
        }
        let argv: Vec<String> = args[idx..].to_vec();
        if argv.is_empty() || !argv[0].starts_with('/') {
            return None;
        }
        Some(Self {
            rows: rows?,
            cols: cols?,
            argv,
        })
    }
}

/// Run the `--tty-exec` helper. `args` are the tokens following `--tty-exec`.
pub(crate) fn run(args: &[String]) -> i32 {
    let Some(parsed) = TtyHelperArgs::parse(args) else {
        eprintln!("nixling-exec-runner: --tty-exec requires --rows R --cols C -- <abs-argv...>");
        // fd 1 is still the inherited status pipe write end (no dup2 onto it has
        // happened yet). Report a typed byte so guestd does not read the bare
        // EOF on helper exit as a successful exec.
        let _ = write(stdout(), &[HelperFailure::Args.as_byte()]);
        return 64;
    };
    // Duplicate the inherited status pipe (fd 1) to a high CLOEXEC fd BEFORE any
    // dup2 onto fd 1, so the status channel survives the slave wiring and closes
    // on a successful execve (guestd reads EOF == success).
    let status = match fcntl_dupfd_cloexec(stdout(), STATUS_DUP_MIN_FD) {
        Ok(fd) => fd,
        // The dup failed, so we have no high CLOEXEC copy — but fd 1 is still the
        // inherited status pipe (the slave wiring has not run). Report a typed
        // byte on it before exiting so a bare EOF is never misread as success.
        Err(_) => {
            let _ = write(stdout(), &[HelperFailure::StatusDup.as_byte()]);
            return EXIT_NO_STATUS_CHANNEL;
        }
    };
    // setup_and_exec only returns on failure; on success it has already replaced
    // the process image via execve.
    let failure = setup_and_exec(&parsed);
    // Best-effort: report the typed byte to guestd over the preserved status fd.
    let _ = write(&status, &[failure.as_byte()]);
    EXIT_SETUP_FAILED
}

/// Perform the controlling-terminal handshake and `execve` the target. Returns
/// only on failure (with the typed cause); on success the image is replaced and
/// control never returns here.
fn setup_and_exec(args: &TtyHelperArgs) -> HelperFailure {
    // 1. New session, detached from any controlling terminal.
    if setsid().is_err() {
        return HelperFailure::Setsid;
    }
    // The PTY slave guestd handed us is on stdin (fd 0).
    let slave = stdin();
    // 2. Acquire the slave as the controlling terminal of the new session
    //    (arg 0: do not steal another session's terminal).
    if ioctl_tiocsctty(slave).is_err() {
        return HelperFailure::Ctty;
    }
    // 3. Apply the initial window geometry before the target starts.
    let winsize = Winsize {
        ws_row: args.rows,
        ws_col: args.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if tcsetwinsize(slave, winsize).is_err() {
        return HelperFailure::Winsize;
    }
    // 4. Wire the slave onto stdout + stderr (it is already on stdin). This
    //    replaces the inherited status pipe on fd 1; the high CLOEXEC copy in
    //    `run` keeps the status channel alive.
    if dup2_stdout(slave).is_err() || dup2_stderr(slave).is_err() {
        return HelperFailure::Dup;
    }
    // 5. Replace the image with the target, inheriting the env/cwd guestd set on
    //    the helper Command. argv[0] is validated absolute (no PATH search).
    let mut cmd = Command::new(&args.argv[0]);
    cmd.args(&args.argv[1..]);
    let _ = cmd.exec();
    HelperFailure::Exec
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_well_formed() {
        let a = TtyHelperArgs::parse(&argv(&["--rows", "24", "--cols", "80", "--", "/bin/sh", "-i"]))
            .expect("parse");
        assert_eq!(a.rows, 24);
        assert_eq!(a.cols, 80);
        assert_eq!(a.argv, argv(&["/bin/sh", "-i"]));
    }

    #[test]
    fn rejects_missing_dimensions() {
        assert!(TtyHelperArgs::parse(&argv(&["--rows", "24", "--", "/bin/sh"])).is_none());
        assert!(TtyHelperArgs::parse(&argv(&["--cols", "80", "--", "/bin/sh"])).is_none());
    }

    #[test]
    fn rejects_relative_or_empty_argv() {
        assert!(TtyHelperArgs::parse(&argv(&["--rows", "24", "--cols", "80", "--", "sh"])).is_none());
        assert!(TtyHelperArgs::parse(&argv(&["--rows", "24", "--cols", "80", "--"])).is_none());
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(
            TtyHelperArgs::parse(&argv(&["--rows", "24", "--cols", "80", "--bogus", "--", "/bin/sh"]))
                .is_none()
        );
    }

    #[test]
    fn failure_bytes_are_stable_and_distinct() {
        // The byte values are a wire contract with guestd; pin them.
        assert_eq!(HelperFailure::Setsid.as_byte(), 1);
        assert_eq!(HelperFailure::Ctty.as_byte(), 2);
        assert_eq!(HelperFailure::Winsize.as_byte(), 3);
        assert_eq!(HelperFailure::Dup.as_byte(), 4);
        assert_eq!(HelperFailure::Exec.as_byte(), 5);
        assert_eq!(HelperFailure::Args.as_byte(), 6);
        assert_eq!(HelperFailure::StatusDup.as_byte(), 7);
    }
}
