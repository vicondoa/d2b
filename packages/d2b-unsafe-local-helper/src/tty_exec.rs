use rustix::io::{fcntl_dupfd_cloexec, write};
use rustix::process::{ioctl_tiocsctty, setsid};
use rustix::stdio::{dup2_stderr, dup2_stdout, stdin, stdout};
use rustix::termios::{Winsize, tcsetwinsize};
use std::os::unix::process::CommandExt;
use std::process::Command;
use uzers::os::unix::UserExt;
use uzers::{get_current_uid, get_user_by_uid};

const STATUS_DUP_MIN_FD: i32 = 10;
const EXIT_NO_STATUS_CHANNEL: i32 = 71;
const EXIT_SETUP_FAILED: i32 = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TtyExecFailure {
    Identity = 1,
    Setsid = 2,
    Ctty = 3,
    Winsize = 4,
    Dup = 5,
    Exec = 6,
    Args = 7,
    StatusDup = 8,
}

impl TtyExecFailure {
    fn as_byte(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TtyExecArgs {
    rows: u16,
    cols: u16,
}

impl TtyExecArgs {
    fn parse(args: &[String]) -> Option<Self> {
        match args {
            [rows_flag, rows, cols_flag, cols]
                if rows_flag == "--rows" && cols_flag == "--cols" =>
            {
                let rows = rows.parse().ok()?;
                let cols = cols.parse().ok()?;
                (rows > 0 && cols > 0).then_some(Self { rows, cols })
            }
            _ => None,
        }
    }
}

pub(crate) fn run(args: &[String]) -> i32 {
    let Some(args) = TtyExecArgs::parse(args) else {
        let _ = write(stdout(), &[TtyExecFailure::Args.as_byte()]);
        return 64;
    };
    let status = match fcntl_dupfd_cloexec(stdout(), STATUS_DUP_MIN_FD) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = write(stdout(), &[TtyExecFailure::StatusDup.as_byte()]);
            return EXIT_NO_STATUS_CHANNEL;
        }
    };
    let failure = setup_and_exec(args);
    let _ = write(&status, &[failure.as_byte()]);
    EXIT_SETUP_FAILED
}

fn setup_and_exec(args: TtyExecArgs) -> TtyExecFailure {
    let uid = get_current_uid();
    let Some(user) = get_user_by_uid(uid) else {
        return TtyExecFailure::Identity;
    };
    let shell = user.shell();
    if !shell.is_absolute() {
        return TtyExecFailure::Identity;
    }
    if setsid().is_err() {
        return TtyExecFailure::Setsid;
    }
    let slave = stdin();
    if ioctl_tiocsctty(slave).is_err() {
        return TtyExecFailure::Ctty;
    }
    let winsize = Winsize {
        ws_row: args.rows,
        ws_col: args.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if tcsetwinsize(slave, winsize).is_err() {
        return TtyExecFailure::Winsize;
    }
    if dup2_stdout(slave).is_err() || dup2_stderr(slave).is_err() {
        return TtyExecFailure::Dup;
    }
    let mut command = Command::new(shell);
    command.arg("-l");
    let _ = command.exec();
    TtyExecFailure::Exec
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    #[test]
    fn tty_exec_accepts_only_fixed_geometry_arguments() {
        assert_eq!(
            TtyExecArgs::parse(&args(&["--rows", "24", "--cols", "80"])),
            Some(TtyExecArgs { rows: 24, cols: 80 })
        );
        assert!(TtyExecArgs::parse(&args(&["--cols", "80", "--rows", "24"])).is_none());
        assert!(
            TtyExecArgs::parse(&args(&["--rows", "24", "--cols", "80", "--", "/bin/sh"])).is_none()
        );
        assert!(TtyExecArgs::parse(&args(&["--rows", "0", "--cols", "80"])).is_none());
    }

    #[test]
    fn failure_bytes_are_stable() {
        assert_eq!(TtyExecFailure::Identity.as_byte(), 1);
        assert_eq!(TtyExecFailure::Setsid.as_byte(), 2);
        assert_eq!(TtyExecFailure::Ctty.as_byte(), 3);
        assert_eq!(TtyExecFailure::Winsize.as_byte(), 4);
        assert_eq!(TtyExecFailure::Dup.as_byte(), 5);
        assert_eq!(TtyExecFailure::Exec.as_byte(), 6);
        assert_eq!(TtyExecFailure::Args.as_byte(), 7);
        assert_eq!(TtyExecFailure::StatusDup.as_byte(), 8);
    }
}
