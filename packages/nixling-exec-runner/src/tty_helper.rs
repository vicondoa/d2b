//! Interactive TTY exec helper (`--tty-exec`).
//!
//! guestd opens a PTY master/slave pair and spawns this helper with the slave
//! on stdin (fd 0) and a `O_CLOEXEC` status pipe on stdout (fd 1). The helper
//! runs entirely in safe `rustix`:
//!
//! 1. `fcntl(F_DUPFD_CLOEXEC)` the status fd to a high number so the upcoming
//!    `dup2` onto fd 1 cannot clobber it,
//! 2. `setsid()` to become a session leader with no controlling terminal,
//! 3. `ioctl(TIOCSCTTY)` on the slave (fd 0) to acquire it as the controlling
//!    terminal of the new session,
//! 4. `dup2` the slave onto fds 1 and 2,
//! 5. `tcsetwinsize` the initial rows/cols,
//! 6. `execve` the target argv.
//!
//! On success the `O_CLOEXEC` status fd closes during `execve`, which guestd
//! observes as EOF. On any setup or exec failure the helper writes a single
//! typed status byte and exits, which guestd maps to a typed `ExecCreate`
//! error. The helper never logs the target argv.
//!
//! This module is the binary half of the crate and may use `rustix`; the full
//! PTY setup + exec lands in the W14 implementation step. The skeleton wires
//! the dispatch, argument grammar, and status-byte contract.

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
        return 64;
    };
    // W14 (impl): perform the PTY session setup (F_DUPFD_CLOEXEC + setsid +
    // TIOCSCTTY + dup2 + tcsetwinsize) then exec `parsed.argv[0]`, writing a
    // typed status byte on failure. Until that lands this mode is inert and
    // guestd does not yet spawn it.
    let _ = (parsed.rows, parsed.cols, parsed.argv.len());
    eprintln!("nixling-exec-runner: --tty-exec not yet implemented");
    70
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
}
