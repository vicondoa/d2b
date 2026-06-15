//! Build the argv that runs a validated guest command as the VM's workload
//! user inside a real PAM login session, via `systemd-run`.
//!
//! This reproduces the environment an SSH login (the retired `vm konsole`
//! transport) produced: `systemd-run -p PAMName=login` opens a PAM session so
//! `pam_systemd` provisions `XDG_RUNTIME_DIR`, and the command is run through
//! the workload user's login shell so the profile (`/etc/set-environment`,
//! `WAYLAND_DISPLAY`, …) is sourced. Graphical clients (e.g. a browser) then
//! find the running compositor's `wayland-*` socket under
//! `/run/user/<uid>`.
//!
//! Security: the target user is the host-fixed `exec_user` (never root, never
//! the wire `user` field). The client argv is passed as shell **positional
//! parameters** (`exec "$@"`), never string-joined into a `-c` script, so an
//! untrusted argv cannot inject shell syntax.

use std::ffi::OsString;
use std::path::Path;

use crate::exec::ValidatedCommand;

/// Whether the session attaches a PTY (interactive) or pipes (non-interactive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    /// Interactive: `systemd-run --pty`. The caller already owns the outer PTY
    /// (guestd's master/slave); `--pty` is inherently synchronous so no
    /// `--wait` is added.
    Pty,
    /// Non-interactive: `systemd-run --pipe --wait`, so stdout/stderr stream
    /// back over pipes and the wrapper exit status carries the unit result.
    Pipe,
}

/// Sentinel `$0` for the login-shell `exec "$@"` wrapper. Never interpreted as
/// a path; it only occupies the shell's `$0` slot so the client program lands
/// in `$1`.
const ARGV0_SENTINEL: &str = "nl-exec";

/// Build the `systemd-run` argument vector (everything AFTER the `systemd-run`
/// binary path) that runs `command` as `exec_user` in a PAM login session.
///
/// The returned argv is appended to the `systemd-run` program by the caller.
pub fn login_session_systemd_run_args(
    login_shell: &Path,
    exec_user: &str,
    mode: SessionMode,
    command: &ValidatedCommand,
) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        OsString::from(format!("--uid={exec_user}")),
        OsString::from("--quiet"),
        OsString::from("--collect"),
    ];
    match mode {
        SessionMode::Pty => argv.push(OsString::from("--pty")),
        SessionMode::Pipe => {
            argv.push(OsString::from("--pipe"));
            argv.push(OsString::from("--wait"));
        }
    }
    argv.push(OsString::from("--property=PAMName=login"));
    argv.push(OsString::from(format!(
        "--working-directory={}",
        command.cwd.display()
    )));
    // Client-declared environment overrides. The login shell establishes the
    // base session env; these explicit `KEY=VALUE`s (the operator's `--env`
    // and the forwarded `TERM`) are layered on top. Each is a single argv
    // element, so values cannot inject additional options.
    for (key, value) in &command.env {
        argv.push(OsString::from("--setenv"));
        argv.push(OsString::from(format!("{key}={value}")));
    }
    // The login shell sources the profile, then `exec "$@"` replaces it with
    // the client command IN that environment. Positional params, never a
    // joined string.
    argv.push(OsString::from("--"));
    argv.push(login_shell.as_os_str().to_owned());
    argv.push(OsString::from("-l"));
    argv.push(OsString::from("-c"));
    argv.push(OsString::from(r#"exec "$@""#));
    argv.push(OsString::from(ARGV0_SENTINEL));
    argv.push(command.program.as_os_str().to_owned());
    for arg in &command.args {
        argv.push(OsString::from(arg));
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cmd(program: &str, args: &[&str], cwd: &str, env: &[(&str, &str)]) -> ValidatedCommand {
        ValidatedCommand {
            program: PathBuf::from(program),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: PathBuf::from(cwd),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    fn as_strs(argv: &[OsString]) -> Vec<String> {
        argv.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn pipe_mode_runs_command_through_login_shell_as_user() {
        let argv = login_session_systemd_run_args(
            Path::new("/run/current-system/sw/bin/bash"),
            "john",
            SessionMode::Pipe,
            &cmd("/run/current-system/sw/bin/id", &["-u"], "/", &[]),
        );
        let s = as_strs(&argv);
        assert_eq!(s[0], "--uid=john");
        assert!(s.contains(&"--pipe".to_owned()));
        assert!(s.contains(&"--wait".to_owned()));
        assert!(s.contains(&"--property=PAMName=login".to_owned()));
        assert!(s.contains(&"--working-directory=/".to_owned()));
        // The command lands as positional params after the login shell.
        let dd = s.iter().position(|a| a == "--").unwrap();
        assert_eq!(s[dd + 1], "/run/current-system/sw/bin/bash");
        assert_eq!(s[dd + 2], "-l");
        assert_eq!(s[dd + 3], "-c");
        assert_eq!(s[dd + 4], r#"exec "$@""#);
        assert_eq!(s[dd + 5], "nl-exec");
        assert_eq!(s[dd + 6], "/run/current-system/sw/bin/id");
        assert_eq!(s[dd + 7], "-u");
    }

    #[test]
    fn pty_mode_uses_pty_and_omits_wait() {
        let argv = login_session_systemd_run_args(
            Path::new("/run/current-system/sw/bin/bash"),
            "john",
            SessionMode::Pty,
            &cmd("/run/current-system/sw/bin/bash", &[], "/", &[]),
        );
        let s = as_strs(&argv);
        assert!(s.contains(&"--pty".to_owned()));
        assert!(!s.contains(&"--pipe".to_owned()));
        assert!(!s.contains(&"--wait".to_owned()));
    }

    #[test]
    fn client_env_becomes_setenv_pairs_never_joined() {
        let argv = login_session_systemd_run_args(
            Path::new("/bin/bash"),
            "john",
            SessionMode::Pipe,
            &cmd("/bin/true", &[], "/tmp", &[("TERM", "xterm"), ("FOO", "bar baz")]),
        );
        let s = as_strs(&argv);
        // Each value is a single argv element (no shell splitting of "bar baz").
        let term_idx = s.iter().position(|a| a == "TERM=xterm").unwrap();
        assert_eq!(s[term_idx - 1], "--setenv");
        let foo_idx = s.iter().position(|a| a == "FOO=bar baz").unwrap();
        assert_eq!(s[foo_idx - 1], "--setenv");
    }

    #[test]
    fn untrusted_argv_cannot_inject_shell_syntax() {
        // A malicious-looking client argv (its own `-c "rm -rf / ; echo $(...)"`)
        // stays literal positional parameters. The login shell's script is the
        // FIXED `exec "$@"`; the client program + args land after the sentinel,
        // so the client cannot inject syntax into the wrapper script.
        let argv = login_session_systemd_run_args(
            Path::new("/bin/bash"),
            "john",
            SessionMode::Pipe,
            &cmd("/bin/sh", &["-c", "rm -rf / ; echo $(whoami)"], "/", &[]),
        );
        let s = as_strs(&argv);
        // Structure after `--`: <login-shell> -l -c 'exec "$@"' <sentinel> <program> <args...>
        let dd = s.iter().position(|a| a == "--").unwrap();
        assert_eq!(s[dd + 1], "/bin/bash");
        assert_eq!(s[dd + 2], "-l");
        assert_eq!(s[dd + 3], "-c");
        // The ONLY shell script is the fixed wrapper, never client-derived.
        assert_eq!(s[dd + 4], r#"exec "$@""#);
        assert_eq!(s[dd + 5], "nl-exec");
        assert_eq!(s[dd + 6], "/bin/sh");
        // The client's own `-c` + injected string are inert positional params.
        assert_eq!(s[dd + 7], "-c");
        assert_eq!(s[dd + 8], "rm -rf / ; echo $(whoami)");
    }

    #[test]
    fn cwd_is_threaded_as_working_directory() {
        let argv = login_session_systemd_run_args(
            Path::new("/bin/bash"),
            "john",
            SessionMode::Pipe,
            &cmd("/bin/true", &[], "/home/john/work", &[]),
        );
        let s = as_strs(&argv);
        assert!(s.contains(&"--working-directory=/home/john/work".to_owned()));
    }
}
