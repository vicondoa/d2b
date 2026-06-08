//! W4-H3: swtpm argv generator (UNIX-socket TPM 2.0 backend).
//!
//! `swtpm` is the per-VM software TPM sidecar nixling spawns for VMs
//! that declare `nixling.vms.<vm>.tpm.enable = true`. The W0b
//! runner-shape audit notes that microvm.nix's TPM component passes
//! the CH TPM socket via `microvm.cloud-hypervisor.extraArgs`, but the
//! sidecar process is shaped as a standalone systemd unit:
//!
//! ```text
//! swtpm socket \
//!   --tpm2 \
//!   --tpmstate dir=<state-dir> \
//!   --ctrl type=unixio,path=<state-dir>/ctrl.sock,mode=0660,uid=<uid>,gid=<gid> \
//!   --server type=unixio,path=<vm>-tpm.sock,mode=0660,uid=<uid>,gid=<gid> \
//!   --flags startup-clear \
//!   --log file=<state-dir>/swtpm.log,level=20 \
//!   --pid file=<state-dir>/swtpm.pid \
//!   --daemon=false
//! ```
//!
//! Plus a pre-start flush invocation per the W3 invariants
//! (`processes::VmProcessInvariants::swtpm_pre_start_flush = true`):
//!
//! ```text
//! swtpm_ioctl -i --unix <state-dir>/ctrl.sock
//! ```
//!
//! …followed by a clean shutdown command (`-s`) before the supervisor
//! starts the long-lived `swtpm socket` process.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

/// All inputs required to render the long-lived `swtpm socket ...`
/// argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwtpmArgvInput {
    /// Absolute store path to the `swtpm` binary.
    pub swtpm_binary_path: String,
    /// VM name (used by [`exec_arg0`] and the server-socket filename
    /// derivation).
    pub vm_name: String,
    /// Absolute path to the per-VM TPM state directory. swtpm writes
    /// `tpm2-00.permall` plus its log/pid in here.
    pub state_dir: String,
    /// Absolute path to the swtpm control socket (`--ctrl`). CH never
    /// connects to this one — the daemon uses it for shutdown/flush.
    pub ctrl_socket_path: String,
    /// Absolute path to the swtpm server socket (`--server`). CH
    /// connects to this one through `--tpm`.
    pub server_socket_path: String,
    /// Numeric uid the swtpm process drops to.
    pub uid: u32,
    /// Numeric gid the swtpm process drops to (also the socket group).
    pub gid: u32,
    /// `--log file=<path>` value; usually `<state_dir>/swtpm.log`.
    pub log_path: String,
    /// `--log level=<N>` value. swtpm accepts 1..20; nixling defaults
    /// to 20 (debug) during alpha and clamps in the daemon caller.
    pub log_level: u8,
    /// `--pid file=<path>` value; usually `<state_dir>/swtpm.pid`.
    pub pid_path: String,
    /// `--flags startup-clear` is emitted when this is `true`. Audit
    /// + W3 invariant: on startup the supervisor runs `swtpm_ioctl -i`
    ///   first, so the long-lived process boots clean.
    pub startup_clear: bool,
    /// Free-form additional swtpm args. Caller is responsible for
    /// quoting; each entry is emitted as-is in order at the end.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// All inputs required to render the pre-start
/// `swtpm_ioctl -i --unix <ctrl-socket>` flush argv. Pairs with the
/// W3 `VmProcessInvariants::swtpm_pre_start_flush` invariant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwtpmIoctlFlushInput {
    /// Absolute store path to the `swtpm_ioctl` binary.
    pub swtpm_ioctl_binary_path: String,
    /// VM name (used by [`exec_arg0_flush`] only).
    pub vm_name: String,
    /// Absolute path to the swtpm control socket the flush command
    /// will speak to.
    pub ctrl_socket_path: String,
}

/// Errors the swtpm argv generators can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum SwtpmArgvError {
    /// Binary path was empty or non-absolute.
    InvalidBinaryPath { path: String },
    /// `vm_name` was empty.
    EmptyVmName,
    /// `state_dir` was empty or non-absolute.
    InvalidStateDir { path: String },
    /// `ctrl_socket_path` or `server_socket_path` was empty.
    EmptySocketPath { which: String },
    /// `log_path` or `pid_path` was empty.
    EmptyFilePath { which: String },
    /// `log_level` was outside 1..=20.
    LogLevelOutOfRange { level: u8 },
}

fn validate_absolute(path: &str, field: &str) -> Result<(), SwtpmArgvError> {
    if path.is_empty() || !path.starts_with('/') {
        Err(SwtpmArgvError::InvalidStateDir {
            path: format!("{field}={path}"),
        })
    } else {
        Ok(())
    }
}

/// Render the long-lived swtpm argv.
pub fn generate_swtpm_argv(input: &SwtpmArgvInput) -> Result<Vec<String>, SwtpmArgvError> {
    if input.swtpm_binary_path.is_empty() || !input.swtpm_binary_path.starts_with('/') {
        return Err(SwtpmArgvError::InvalidBinaryPath {
            path: input.swtpm_binary_path.clone(),
        });
    }
    if input.vm_name.is_empty() {
        return Err(SwtpmArgvError::EmptyVmName);
    }
    validate_absolute(&input.state_dir, "state_dir")?;
    if input.ctrl_socket_path.is_empty() {
        return Err(SwtpmArgvError::EmptySocketPath {
            which: "ctrl_socket_path".to_owned(),
        });
    }
    if input.server_socket_path.is_empty() {
        return Err(SwtpmArgvError::EmptySocketPath {
            which: "server_socket_path".to_owned(),
        });
    }
    if input.log_path.is_empty() {
        return Err(SwtpmArgvError::EmptyFilePath {
            which: "log_path".to_owned(),
        });
    }
    if input.pid_path.is_empty() {
        return Err(SwtpmArgvError::EmptyFilePath {
            which: "pid_path".to_owned(),
        });
    }
    if !(1..=20).contains(&input.log_level) {
        return Err(SwtpmArgvError::LogLevelOutOfRange {
            level: input.log_level,
        });
    }

    let mut argv: Vec<String> = Vec::with_capacity(20);
    argv.push(input.swtpm_binary_path.clone());
    argv.push("socket".to_owned());
    argv.push("--tpm2".to_owned());

    argv.push("--tpmstate".to_owned());
    argv.push(format!("dir={}", input.state_dir));

    argv.push("--ctrl".to_owned());
    argv.push(format!(
        "type=unixio,path={},mode=0660,uid={},gid={}",
        input.ctrl_socket_path, input.uid, input.gid
    ));

    argv.push("--server".to_owned());
    argv.push(format!(
        "type=unixio,path={},mode=0660,uid={},gid={}",
        input.server_socket_path, input.uid, input.gid
    ));

    if input.startup_clear {
        argv.push("--flags".to_owned());
        argv.push("startup-clear".to_owned());
    }

    argv.push("--log".to_owned());
    argv.push(format!("file={},level={}", input.log_path, input.log_level));

    argv.push("--pid".to_owned());
    argv.push(format!("file={}", input.pid_path));

    // swtpm forks by default when it is `socket`-mode; the W4
    // supervisor controls lifetime via pidfd, so it forces foreground
    // operation.
    argv.push("--daemon=false".to_owned());

    for extra in &input.extra_args {
        argv.push(extra.clone());
    }

    Ok(argv)
}

/// Render the pre-start `swtpm_ioctl -i --unix <ctrl>` flush argv.
pub fn generate_swtpm_ioctl_flush_argv(
    input: &SwtpmIoctlFlushInput,
) -> Result<Vec<String>, SwtpmArgvError> {
    if input.swtpm_ioctl_binary_path.is_empty() || !input.swtpm_ioctl_binary_path.starts_with('/') {
        return Err(SwtpmArgvError::InvalidBinaryPath {
            path: input.swtpm_ioctl_binary_path.clone(),
        });
    }
    if input.vm_name.is_empty() {
        return Err(SwtpmArgvError::EmptyVmName);
    }
    if input.ctrl_socket_path.is_empty() {
        return Err(SwtpmArgvError::EmptySocketPath {
            which: "ctrl_socket_path".to_owned(),
        });
    }
    Ok(vec![
        input.swtpm_ioctl_binary_path.clone(),
        "-i".to_owned(),
        "--unix".to_owned(),
        input.ctrl_socket_path.clone(),
    ])
}

/// `arg0` for the long-lived swtpm process: `microvm-swtpm@<vm>`.
pub fn exec_arg0(input: &SwtpmArgvInput) -> Result<String, SwtpmArgvError> {
    if input.vm_name.is_empty() {
        return Err(SwtpmArgvError::EmptyVmName);
    }
    Ok(format!("microvm-swtpm@{}", input.vm_name))
}

/// `arg0` for the pre-start flush: `microvm-swtpm-flush@<vm>`.
pub fn exec_arg0_flush(input: &SwtpmIoctlFlushInput) -> Result<String, SwtpmArgvError> {
    if input.vm_name.is_empty() {
        return Err(SwtpmArgvError::EmptyVmName);
    }
    Ok(format!("microvm-swtpm-flush@{}", input.vm_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit_swtpm_input() -> SwtpmArgvInput {
        SwtpmArgvInput {
            swtpm_binary_path: "/nix/store/SWTPMSWTPMSWTPMSWTPMSWTPM-swtpm-0.10.0/bin/swtpm"
                .to_owned(),
            vm_name: "corp-vm".to_owned(),
            state_dir: "/var/lib/nixling/vms/corp-vm/tpm".to_owned(),
            ctrl_socket_path: "/var/lib/nixling/vms/corp-vm/tpm/ctrl.sock".to_owned(),
            server_socket_path: "/run/nixling/vms/corp-vm/swtpm.sock".to_owned(),
            uid: 1100,
            gid: 1100,
            log_path: "/var/lib/nixling/vms/corp-vm/tpm/swtpm.log".to_owned(),
            log_level: 20,
            pid_path: "/var/lib/nixling/vms/corp-vm/tpm/swtpm.pid".to_owned(),
            startup_clear: true,
            extra_args: Vec::new(),
        }
    }

    fn audit_flush_input() -> SwtpmIoctlFlushInput {
        SwtpmIoctlFlushInput {
            swtpm_ioctl_binary_path:
                "/nix/store/SWTPMSWTPMSWTPMSWTPMSWTPM-swtpm-0.10.0/bin/swtpm_ioctl".to_owned(),
            vm_name: "corp-vm".to_owned(),
            ctrl_socket_path: "/var/lib/nixling/vms/corp-vm/tpm/ctrl.sock".to_owned(),
        }
    }

    /// P1 byte-parity oracle for the long-lived swtpm argv.
    ///
    /// The golden file `tests/golden/runner-shape/swtpm-argv-minimal.txt`
    /// contains a leading comment block (lines starting with `#`)
    /// followed by the argv vector joined by `'\n'`, one argument per
    /// line. This test strips the comment block and asserts byte-parity
    /// with the live generator output, locking the shape the
    /// `ph1-p1-swtpm-persistence` validator and the minijail profile
    /// downstream reason about.
    #[test]
    fn audit_swtpm_input_parity_golden() {
        let golden = include_str!(
            "../../../tests/golden/runner-shape/swtpm-argv-minimal.txt"
        );
        let expected: String = golden
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let expected = expected.trim_matches('\n').to_owned();

        let argv = generate_swtpm_argv(&audit_swtpm_input()).unwrap();
        let observed = argv.join("\n");

        assert_eq!(
            observed, expected,
            "swtpm argv golden drift — regenerate tests/golden/runner-shape/swtpm-argv-minimal.txt\n--- expected ---\n{expected}\n--- observed ---\n{observed}\n"
        );
    }

    #[test]
    fn long_lived_argv_has_expected_shape() {
        let argv = generate_swtpm_argv(&audit_swtpm_input()).unwrap();
        assert!(argv[0].ends_with("/swtpm"));
        assert_eq!(argv[1], "socket");
        assert_eq!(argv[2], "--tpm2");

        let joined = argv.join(" ");
        assert!(joined.contains("--tpmstate dir=/var/lib/nixling/vms/corp-vm/tpm"));
        assert!(joined.contains(
            "--ctrl type=unixio,path=/var/lib/nixling/vms/corp-vm/tpm/ctrl.sock,mode=0660,uid=1100,gid=1100"
        ));
        assert!(joined.contains(
            "--server type=unixio,path=/run/nixling/vms/corp-vm/swtpm.sock,mode=0660,uid=1100,gid=1100"
        ));
        assert!(joined.contains("--flags startup-clear"));
        assert!(joined.contains("--log file=/var/lib/nixling/vms/corp-vm/tpm/swtpm.log,level=20"));
        assert!(joined.contains("--pid file=/var/lib/nixling/vms/corp-vm/tpm/swtpm.pid"));
        assert!(joined.contains("--daemon=false"));
    }

    #[test]
    fn flush_argv_matches_w3_invariant() {
        let argv = generate_swtpm_ioctl_flush_argv(&audit_flush_input()).unwrap();
        assert_eq!(
            argv,
            vec![
                "/nix/store/SWTPMSWTPMSWTPMSWTPMSWTPM-swtpm-0.10.0/bin/swtpm_ioctl".to_owned(),
                "-i".to_owned(),
                "--unix".to_owned(),
                "/var/lib/nixling/vms/corp-vm/tpm/ctrl.sock".to_owned(),
            ]
        );
    }

    #[test]
    fn exec_arg0_for_long_lived() {
        assert_eq!(
            exec_arg0(&audit_swtpm_input()).unwrap(),
            "microvm-swtpm@corp-vm"
        );
    }

    #[test]
    fn exec_arg0_for_flush() {
        assert_eq!(
            exec_arg0_flush(&audit_flush_input()).unwrap(),
            "microvm-swtpm-flush@corp-vm"
        );
    }

    #[test]
    fn omits_startup_clear_when_disabled() {
        let mut input = audit_swtpm_input();
        input.startup_clear = false;
        let argv = generate_swtpm_argv(&input).unwrap();
        assert!(!argv.iter().any(|a| a == "--flags"));
        assert!(!argv.iter().any(|a| a == "startup-clear"));
    }

    #[test]
    fn extra_args_appended_at_end() {
        let mut input = audit_swtpm_input();
        input.extra_args = vec!["--migration-key".to_owned(), "file=/tmp/mig.key".to_owned()];
        let argv = generate_swtpm_argv(&input).unwrap();
        let last_two = &argv[argv.len() - 2..];
        assert_eq!(last_two, &["--migration-key", "file=/tmp/mig.key"]);
    }

    #[test]
    fn rejects_invalid_binary_path() {
        let mut input = audit_swtpm_input();
        input.swtpm_binary_path = "swtpm".to_owned();
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::InvalidBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_vm_name() {
        let mut input = audit_swtpm_input();
        input.vm_name.clear();
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn rejects_non_absolute_state_dir() {
        let mut input = audit_swtpm_input();
        input.state_dir = "tpm".to_owned();
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::InvalidStateDir { .. })
        ));
    }

    #[test]
    fn rejects_empty_ctrl_socket() {
        let mut input = audit_swtpm_input();
        input.ctrl_socket_path.clear();
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::EmptySocketPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_server_socket() {
        let mut input = audit_swtpm_input();
        input.server_socket_path.clear();
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::EmptySocketPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_log_path() {
        let mut input = audit_swtpm_input();
        input.log_path.clear();
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::EmptyFilePath { .. })
        ));
    }

    #[test]
    fn rejects_empty_pid_path() {
        let mut input = audit_swtpm_input();
        input.pid_path.clear();
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::EmptyFilePath { .. })
        ));
    }

    #[test]
    fn rejects_log_level_out_of_range() {
        let mut input = audit_swtpm_input();
        input.log_level = 0;
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::LogLevelOutOfRange { level: 0 })
        ));
        input.log_level = 21;
        assert!(matches!(
            generate_swtpm_argv(&input),
            Err(SwtpmArgvError::LogLevelOutOfRange { level: 21 })
        ));
    }

    #[test]
    fn flush_rejects_invalid_inputs() {
        let mut input = audit_flush_input();
        input.swtpm_ioctl_binary_path.clear();
        assert!(matches!(
            generate_swtpm_ioctl_flush_argv(&input),
            Err(SwtpmArgvError::InvalidBinaryPath { .. })
        ));
        let mut input = audit_flush_input();
        input.vm_name.clear();
        assert!(matches!(
            generate_swtpm_ioctl_flush_argv(&input),
            Err(SwtpmArgvError::EmptyVmName)
        ));
        let mut input = audit_flush_input();
        input.ctrl_socket_path.clear();
        assert!(matches!(
            generate_swtpm_ioctl_flush_argv(&input),
            Err(SwtpmArgvError::EmptySocketPath { .. })
        ));
    }

    #[test]
    fn argv_is_round_trip_serializable() {
        let input = audit_swtpm_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: SwtpmArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }

    #[test]
    fn flush_input_round_trip_serializable() {
        let input = audit_flush_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: SwtpmIoctlFlushInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }
}
