//! Build the argv that runs a validated guest command as the VM's workload
//! user inside a real PAM login session, via `systemd-run`.
//!
//! This reproduces the environment an interactive login (driven by
//! `vm exec -it`) produces: `systemd-run -p PAMName=login` opens a PAM session so
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

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
///
/// `unit_name` names the transient unit (`--unit=`) so teardown can reach the
/// workload directly. The workload runs in a PID 1-owned transient unit, NOT in
/// the `systemd-run` wrapper's process group, so naming the unit is what lets
/// [`systemctl_kill_unit`] SIGKILL the whole workload cgroup on disconnect /
/// cancel / runtime ceiling. Use [`unique_exec_unit_name`] to mint it.
pub fn login_session_systemd_run_args(
    login_shell: &Path,
    exec_user: &str,
    unit_name: &str,
    mode: SessionMode,
    command: &ValidatedCommand,
) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        OsString::from(format!("--uid={exec_user}")),
        OsString::from(format!("--unit={unit_name}")),
        OsString::from("--quiet"),
        OsString::from("--collect"),
        // Pass argv through verbatim: systemd must NOT expand `$VAR`/`${VAR}` in
        // the unit's ExecStart at unit-load time. The login-shell `exec "$@"`
        // wrapper handles any intended expansion in the workload-user session;
        // disabling systemd's own expansion keeps an untrusted client argv
        // literal (a `$(...)`/`$VAR` element stays inert) and does not affect
        // explicit `--setenv=KEY=VALUE` pairs.
        OsString::from("--expand-environment=no"),
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

/// Process-unique counter component for transient unit names.
static EXEC_UNIT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mint a process-unique transient-unit name for an exec session,
/// e.g. `nixling-exec-<pid>-<counter>-<salt>.service`.
///
/// `pid + counter + nanos` is already practically unique; the OS-seeded
/// `RandomState` salt additionally guards the theoretical collision after a
/// guestd restart / PID reuse / clock rollback / VM snapshot restore. The name
/// only uses systemd-legal characters and stays well under the unit-name limit.
pub fn unique_exec_unit_name() -> String {
    let counter = EXEC_UNIT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(counter);
    hasher.write_u32(pid);
    hasher.write_u128(nanos);
    let salt = hasher.finish();
    format!("nixling-exec-{pid}-{counter}-{salt:016x}.service")
}

/// Resolve the `systemctl` path sitting next to a given `systemd-run` path.
/// Both ship from the same systemd package, so they share a `bin` directory.
pub fn sibling_systemctl_path(systemd_run_path: &Path) -> PathBuf {
    match systemd_run_path.parent() {
        Some(dir) => dir.join("systemctl"),
        None => PathBuf::from("systemctl"),
    }
}

/// Bounded wall-clock budget for one `systemctl kill` invocation. A wedged
/// PID 1 / D-Bus MUST NOT hang teardown, so the child is force-killed past it.
const SYSTEMCTL_KILL_TIMEOUT: Duration = Duration::from_secs(2);
/// Brief pause before the single retry that closes the spawn/teardown race
/// where the transient unit is not yet registered when the first kill runs.
const SYSTEMCTL_KILL_RETRY_DELAY: Duration = Duration::from_millis(50);

/// Best-effort, bounded SIGKILL of the whole transient-unit control group.
///
/// The workload runs in a PID 1-owned transient unit, NOT in the `systemd-run`
/// wrapper's process group, so a wrapper/session SIGKILL alone leaves a quiet
/// non-TTY command running indefinitely. Callers MUST issue their local
/// (wrapper-PGID / `/proc`-session) kill FIRST — both so teardown can never be
/// stranded by a wedged PID 1 (the local path always fires) and so the wrapper
/// cannot send a further `StartTransientUnit` after teardown begins. This then
/// SIGKILLs the named unit's entire cgroup (`--kill-whom=all`).
///
/// Bounded: a wedged PID 1 / D-Bus cannot hang teardown — the `systemctl` child
/// is force-killed past [`SYSTEMCTL_KILL_TIMEOUT`]. Idempotent: a missing unit
/// is a no-op. Retries once after a non-success result to catch the startup
/// race where the unit is not yet registered.
///
/// Boundary: this kills everything inside THIS exec transient unit (including
/// `setsid`/double-fork descendants, which stay in the unit cgroup). A workload
/// that successfully asks another manager to spawn work into a different cgroup
/// (e.g. a lingering `systemd --user` service) is out of scope for this kill.
pub fn systemctl_kill_unit(systemctl_path: &Path, unit_name: &str) {
    for attempt in 0..2 {
        // `Some(true)` => unit existed and was signalled; done.
        if let Some(true) = run_systemctl_kill_once(systemctl_path, unit_name) {
            return;
        }
        // Non-zero (unit not loaded) or timed out => maybe the unit is not
        // registered yet (spawn/teardown race). Pause briefly, then retry once.
        if attempt == 0 {
            std::thread::sleep(SYSTEMCTL_KILL_RETRY_DELAY);
        }
    }
}

/// Run one bounded `systemctl kill`. `Some(true)` = exit 0; `Some(false)` = ran
/// but non-zero; `None` = spawn failed or timed out (child force-killed). The
/// explicit `--system` + `--kill-whom=all` + `--signal=SIGKILL` argv is spelled
/// out because this teardown is security-sensitive (no reliance on defaults).
fn run_systemctl_kill_once(systemctl_path: &Path, unit_name: &str) -> Option<bool> {
    let mut child = Command::new(systemctl_path)
        .arg("--system")
        .arg("--no-ask-password")
        .arg("--quiet")
        .arg("--kill-whom=all")
        .arg("--signal=SIGKILL")
        .arg("kill")
        .arg(unit_name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + SYSTEMCTL_KILL_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
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
        argv.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn pipe_mode_runs_command_through_login_shell_as_user() {
        let argv = login_session_systemd_run_args(
            Path::new("/run/current-system/sw/bin/bash"),
            "john",
            "nixling-exec-1-0-abcdef.service",
            SessionMode::Pipe,
            &cmd("/run/current-system/sw/bin/id", &["-u"], "/", &[]),
        );
        let s = as_strs(&argv);
        assert_eq!(s[0], "--uid=john");
        // The unit is named so teardown can `systemctl kill` the workload cgroup,
        // and systemd must not expand `$VAR` in the unit ExecStart.
        assert!(s.contains(&"--unit=nixling-exec-1-0-abcdef.service".to_owned()));
        assert!(s.contains(&"--expand-environment=no".to_owned()));
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
            "nixling-exec-1-0-abcdef.service",
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
            "nixling-exec-1-0-abcdef.service",
            SessionMode::Pipe,
            &cmd(
                "/bin/true",
                &[],
                "/tmp",
                &[("TERM", "xterm"), ("FOO", "bar baz")],
            ),
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
            "nixling-exec-1-0-abcdef.service",
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
            "nixling-exec-1-0-abcdef.service",
            SessionMode::Pipe,
            &cmd("/bin/true", &[], "/home/john/work", &[]),
        );
        let s = as_strs(&argv);
        assert!(s.contains(&"--working-directory=/home/john/work".to_owned()));
    }

    #[test]
    fn literal_dollar_argv_is_preserved_with_expand_environment_no() {
        // A client argv element that LOOKS like a shell/systemd variable must
        // survive verbatim: `--expand-environment=no` stops systemd expanding
        // it at unit load, and the fixed `exec "$@"` wrapper keeps it positional.
        let argv = login_session_systemd_run_args(
            Path::new("/bin/bash"),
            "john",
            "nixling-exec-1-0-abcdef.service",
            SessionMode::Pipe,
            &cmd("/bin/echo", &["$HOME", "${PATH}", "$(id -u)"], "/", &[]),
        );
        let s = as_strs(&argv);
        assert!(s.contains(&"--expand-environment=no".to_owned()));
        // The literal variable-looking args land as inert positional params.
        assert!(s.contains(&"$HOME".to_owned()));
        assert!(s.contains(&"${PATH}".to_owned()));
        assert!(s.contains(&"$(id -u)".to_owned()));
    }

    #[test]
    fn unique_exec_unit_name_is_unique_and_well_formed() {
        let a = unique_exec_unit_name();
        let b = unique_exec_unit_name();
        assert_ne!(a, b, "successive unit names must differ");
        for name in [&a, &b] {
            assert!(name.starts_with("nixling-exec-"));
            assert!(name.ends_with(".service"));
            // Only systemd-legal unit-name characters.
            let stem = name.strip_suffix(".service").unwrap();
            assert!(stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')));
        }
    }

    #[test]
    fn sibling_systemctl_path_sits_next_to_systemd_run() {
        assert_eq!(
            sibling_systemctl_path(Path::new("/run/current-system/sw/bin/systemd-run")),
            PathBuf::from("/run/current-system/sw/bin/systemctl")
        );
        // A bare name (no parent) falls back to a PATH-resolved `systemctl`.
        assert_eq!(
            sibling_systemctl_path(Path::new("systemd-run")),
            PathBuf::from("systemctl")
        );
    }

    // --- kill-model teardown (hermetic, via a fake `systemctl`) -------------

    /// Scratch dir under the system temp dir (respects TMPDIR), never repo-relative.
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "guestd-kill-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write an executable fake `systemctl` that appends one record per
    /// invocation to `<dir>/calls.log` and exits with `exit_code`. Argv
    /// boundaries are preserved: each argument is written followed by a NUL
    /// byte, and each invocation record is terminated by a newline. This
    /// lets `read_calls` reconstruct the EXACT argv vector (order + count),
    /// so a regression that adds/reorders/duplicates flags is caught — a
    /// space-joined `$*` could not distinguish those shapes.
    fn write_fake_systemctl(dir: &Path, exit_code: i32) -> PathBuf {
        let log = dir.join("calls.log");
        let script = dir.join("systemctl");
        let body = format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\0' \"$a\"; done >> '{0}'\nprintf '\\n' >> '{0}'\nexit {1}\n",
            log.display(),
            exit_code
        );
        std::fs::write(&script, body).unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        script
    }

    /// One inner `Vec<String>` per fake-`systemctl` invocation, each holding
    /// that invocation's exact argv (excluding the program name), with
    /// boundaries preserved via the NUL-separated record format written by
    /// `write_fake_systemctl`.
    fn read_calls(dir: &Path) -> Vec<Vec<String>> {
        match std::fs::read_to_string(dir.join("calls.log")) {
            Ok(s) => s
                .lines()
                .map(|line| {
                    line.split('\0')
                        .filter(|tok| !tok.is_empty())
                        .map(|tok| tok.to_owned())
                        .collect()
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    #[test]
    fn run_systemctl_kill_once_spells_out_the_full_kill_argv() {
        let dir = scratch_dir("argv");
        let systemctl = write_fake_systemctl(&dir, 0);

        let unit = "nixling-exec-1-0-abcdef.service";
        assert_eq!(run_systemctl_kill_once(&systemctl, unit), Some(true));

        let calls = read_calls(&dir);
        assert_eq!(calls.len(), 1, "exit-0 means exactly one invocation");
        // Exact argv equality (order + count), not mere token presence: a
        // regression that adds an extra flag, duplicates a conflicting one,
        // or reorders `kill`/<unit> would change teardown semantics yet slip
        // past a subset check.
        assert_eq!(
            calls[0],
            vec![
                "--system".to_owned(),
                "--no-ask-password".to_owned(),
                "--quiet".to_owned(),
                "--kill-whom=all".to_owned(),
                "--signal=SIGKILL".to_owned(),
                "kill".to_owned(),
                unit.to_owned(),
            ],
            "kill argv must be exactly the spelled-out vector"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn systemctl_kill_unit_retries_once_after_a_failure() {
        // A unit not-yet-registered makes the first kill exit non-zero; the
        // teardown MUST retry exactly once (total two invocations).
        let dir = scratch_dir("retry");
        let systemctl = write_fake_systemctl(&dir, 1);

        systemctl_kill_unit(&systemctl, "nixling-exec-2-0-abcdef.service");

        let calls = read_calls(&dir);
        assert_eq!(calls.len(), 2, "non-success must retry exactly once");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn systemctl_kill_unit_does_not_retry_after_success() {
        // A first successful kill ends teardown immediately (no second call).
        let dir = scratch_dir("nostretch");
        let systemctl = write_fake_systemctl(&dir, 0);

        systemctl_kill_unit(&systemctl, "nixling-exec-3-0-abcdef.service");

        let calls = read_calls(&dir);
        assert_eq!(calls.len(), 1, "success must not retry");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_systemctl_kill_once_reports_none_when_spawn_fails() {
        // A non-existent systemctl path => spawn error => None (best-effort).
        let missing = Path::new("/nonexistent/guestd-kill/systemctl");
        assert_eq!(
            run_systemctl_kill_once(missing, "nixling-exec-4-0-abcdef.service"),
            None
        );
    }
}
