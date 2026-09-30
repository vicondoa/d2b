//! swtpm argv generator (UNIX-socket TPM 2.0 backend).
//!
//! `swtpm` is the per-VM software TPM sidecar d2b spawns for VMs
//! that declare `d2b.vms.<vm>.tpm.enable = true`. The guest-side
//! TPM module passes the CH TPM socket via
//! `d2b.vms.<vm>.runner.hypervisor.extraArgs`, and the sidecar
//! process is shaped as a standalone broker-spawned worker:
//!
//! ```text
//! swtpm socket \
//!   --tpm2 \
//!   --tpmstate dir=<state-dir> \
//!   --ctrl type=unixio,path=<state-dir>/ctrl.sock,mode=0660 \
//!   --server type=unixio,path=<vm>-tpm.sock,mode=0660 \
//!   --flags startup-clear \
//!   --log file=<state-dir>/swtpm.log,level=20 \
//!   --pid file=<state-dir>/swtpm.pid
//! ```
//!
//! `swtpm socket` stays in the foreground unless `-d|--daemon` is
//! given, and that flag takes no argument: swtpm 0.10.1 refuses
//! `--daemon=false` outright
//! (`socket: option '--daemon' doesn't allow an argument`) and exits
//! before it ever binds a socket. Foreground operation is what the
//! supervisor needs anyway (it holds the pidfd), so the flag is not
//! emitted at all.
//!
//! Plus a pre-start flush invocation per the process invariants
//! (`processes::VmProcessInvariants::swtpm_pre_start_flush = true`):
//!
//! ```text
//! swtpm_ioctl -i --unix <state-dir>/ctrl.sock
//! ```
//!
//! …followed by a clean shutdown command (`-s`) before the supervisor
//! starts the long-lived `swtpm socket` process.
//!
//! # What the argv may not carry
//!
//! swtpm used to accept a caller-supplied `uid=`/`gid=` socket owner and a
//! free-form `extra_args` tail. Both are authority by another name: a
//! numerical broker identity in a launch argument is identity a caller
//! selects, and a free-form tail lets the same caller name a socket or a
//! device node the admitted relationship does not carry. Neither exists any
//! more. The socket is created owned by the uid swtpm already runs as, which
//! is what the state directory's ACL and `mode=0660` are written against, and
//! the admitted socket paths are the only filesystem inputs.
//!
//! An argument that names a device node is refused outright: a TPM worker's
//! device capability is delivered by its own admitted `DeviceBinding`, never
//! by a path in its command line.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

use crate::{MAX_SWTPM_LOG_LEVEL, MIN_SWTPM_LOG_LEVEL};

/// All inputs required to render the long-lived `swtpm socket ...`
/// argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwtpmArgvInput {
    /// Absolute store path to the `swtpm` binary.
    pub swtpm_binary_path: String,
    /// VM name; the Provider refuses an empty one.
    pub vm_name: String,
    /// Absolute path to the per-VM TPM state directory. swtpm writes
    /// `tpm2-00.permall` plus its log/pid in here.
    pub state_dir: String,
    /// Absolute path to the swtpm control socket (`--ctrl`). CH never
    /// connects to this one - the daemon uses it for shutdown/flush.
    pub ctrl_socket_path: String,
    /// Absolute path to the swtpm server socket (`--server`). CH
    /// connects to this one through `--tpm`.
    pub server_socket_path: String,
    /// `--log file=<path>` value; usually `<state_dir>/swtpm.log`.
    pub log_path: String,
    /// `--log level=<N>` value. swtpm accepts 1..20; d2b defaults
    /// to 20 (debug) during alpha and clamps in the daemon caller.
    pub log_level: u8,
    /// `--pid file=<path>` value; usually `<state_dir>/swtpm.pid`.
    pub pid_path: String,
    /// `--flags startup-clear` is emitted when this is `true`. On
    /// startup the supervisor runs `swtpm_ioctl -i` first, so the
    /// long-lived process boots clean.
    pub startup_clear: bool,
}

/// All inputs required to render the pre-start
/// `swtpm_ioctl -i --unix <ctrl-socket>` flush argv. Pairs with the
/// `VmProcessInvariants::swtpm_pre_start_flush` invariant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwtpmIoctlFlushInput {
    /// Absolute store path to the `swtpm_ioctl` binary.
    pub swtpm_ioctl_binary_path: String,
    /// VM name; the Provider refuses an empty one.
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
    InvalidBinaryPath {
        /// The offending path.
        path: String,
    },
    /// `vm_name` was empty.
    EmptyVmName,
    /// One path argument named a device node. A device capability is
    /// delivered by the worker's own admitted `DeviceBinding`, so a
    /// command line is never the way to reach one.
    DeviceNodeArgumentRefused,
    /// `state_dir` was empty or non-absolute.
    InvalidStateDir {
        /// The offending path.
        path: String,
    },
    /// `ctrl_socket_path` or `server_socket_path` was empty.
    EmptySocketPath {
        /// The empty field name.
        which: String,
    },
    /// `log_path` or `pid_path` was empty.
    EmptyFilePath {
        /// The empty field name.
        which: String,
    },
    /// `log_level` was outside 1..=20.
    LogLevelOutOfRange {
        /// The offending log level.
        level: u8,
    },
}

/// Fence one absolute path argument.
///
/// The path has to be absolute, and it must not name a device node: swtpm's
/// inputs are directories and Unix sockets inside the admitted state view,
/// never a character or block device. A `/dev/...` component is refused here
/// rather than handed to a launch that would then hold a device the binding
/// path never admitted.
fn validate_absolute(path: &str, field: &str) -> Result<(), SwtpmArgvError> {
    if path.is_empty() || !path.starts_with('/') {
        return Err(SwtpmArgvError::InvalidStateDir {
            path: format!("{field}={path}"),
        });
    }
    if path
        .split('/')
        .any(|component| component == "dev" || component == "proc" || component == "sys")
    {
        return Err(SwtpmArgvError::DeviceNodeArgumentRefused);
    }
    Ok(())
}

/// Render the long-lived swtpm argv.
///
/// # Errors
///
/// Returns [`SwtpmArgvError::InvalidBinaryPath`] for an empty or
/// non-absolute swtpm binary path, [`SwtpmArgvError::EmptyVmName`] for an
/// empty VM name, [`SwtpmArgvError::InvalidStateDir`] for an empty or
/// non-absolute state directory, [`SwtpmArgvError::EmptySocketPath`] or
/// [`SwtpmArgvError::EmptyFilePath`] for empty socket or file paths, and
/// [`SwtpmArgvError::LogLevelOutOfRange`] when the log level is outside the
/// frozen bound.
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
    validate_absolute(&input.ctrl_socket_path, "ctrl_socket_path")?;
    validate_absolute(&input.server_socket_path, "server_socket_path")?;
    validate_absolute(&input.log_path, "log_path")?;
    validate_absolute(&input.pid_path, "pid_path")?;
    if !(MIN_SWTPM_LOG_LEVEL..=MAX_SWTPM_LOG_LEVEL).contains(&input.log_level) {
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

    // No socket owner entry is emitted. swtpm `chown()`s each socket to
    // whatever id it is handed, and inside the launch's user namespace that
    // chown is refused ("Could not change ownership of UnixIO socket to 0:0
    // Operation not permitted"), so swtpm exits 1 before it binds the data
    // socket and the state directory is left with no log, no pid file and no
    // NVRAM. Reproduced with the exact device-worker artifact binary: this
    // argv runs to a live swtpm, the same argv naming an owner aborts. The
    // socket is therefore created owned by the uid swtpm already runs as,
    // which is what `mode=0660` and the state directory's ACL are written
    // against - and no caller chooses a numerical identity any more.
    argv.push("--ctrl".to_owned());
    argv.push(format!(
        "type=unixio,path={},mode=0660",
        input.ctrl_socket_path
    ));

    argv.push("--server".to_owned());
    argv.push(format!(
        "type=unixio,path={},mode=0660",
        input.server_socket_path
    ));

    if input.startup_clear {
        argv.push("--flags".to_owned());
        argv.push("startup-clear".to_owned());
    }

    argv.push("--log".to_owned());
    argv.push(format!("file={},level={}", input.log_path, input.log_level));

    argv.push("--pid".to_owned());
    argv.push(format!("file={}", input.pid_path));

    // The supervisor controls lifetime via pidfd and swtpm's `socket`
    // mode already runs in the foreground: `--daemon` is the opt-in
    // (argument-less) daemonize flag, so nothing is emitted here. The
    // pre-0.10 spelling `--daemon=false` is rejected by the installed
    // swtpm ("option '--daemon' doesn't allow an argument"), which
    // killed the worker before it bound a socket.

    Ok(argv)
}

/// Render the pre-start `swtpm_ioctl -i --unix <ctrl>` flush argv.
///
/// # Errors
///
/// Returns [`SwtpmArgvError::InvalidBinaryPath`] for an empty or
/// non-absolute swtpm_ioctl binary path, [`SwtpmArgvError::EmptyVmName`] for
/// an empty VM name, and [`SwtpmArgvError::EmptySocketPath`] for an empty
/// control socket path.
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
    validate_absolute(&input.ctrl_socket_path, "ctrl_socket_path")?;
    Ok(vec![
        input.swtpm_ioctl_binary_path.clone(),
        "-i".to_owned(),
        "--unix".to_owned(),
        input.ctrl_socket_path.clone(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit_swtpm_input() -> SwtpmArgvInput {
        SwtpmArgvInput {
            swtpm_binary_path: "/nix/store/SWTPMSWTPMSWTPMSWTPMSWTPM-swtpm-0.10.0/bin/swtpm"
                .to_owned(),
            vm_name: "corp-vm".to_owned(),
            state_dir: "/var/lib/d2b/vms/corp-vm/tpm".to_owned(),
            ctrl_socket_path: "/var/lib/d2b/vms/corp-vm/tpm/ctrl.sock".to_owned(),
            server_socket_path: "/run/d2b/vms/corp-vm/swtpm.sock".to_owned(),
            log_path: "/var/lib/d2b/vms/corp-vm/tpm/swtpm.log".to_owned(),
            log_level: 20,
            pid_path: "/var/lib/d2b/vms/corp-vm/tpm/swtpm.pid".to_owned(),
            startup_clear: true,
        }
    }

    fn audit_flush_input() -> SwtpmIoctlFlushInput {
        SwtpmIoctlFlushInput {
            swtpm_ioctl_binary_path:
                "/nix/store/SWTPMSWTPMSWTPMSWTPMSWTPM-swtpm-0.10.0/bin/swtpm_ioctl".to_owned(),
            vm_name: "corp-vm".to_owned(),
            ctrl_socket_path: "/var/lib/d2b/vms/corp-vm/tpm/ctrl.sock".to_owned(),
        }
    }

    /// Byte-parity oracle for the long-lived swtpm argv.
    ///
    /// The golden file `tests/golden/runner-shape/swtpm-argv-minimal.txt`
    /// contains a leading comment block (lines starting with `#`)
    /// followed by the argv vector joined by `'\n'`, one argument per
    /// line. This test strips the comment block and asserts byte-parity
    /// with the live generator output, locking the shape the persistence
    /// validator and the minijail profile downstream reason about.
    #[test]
    fn audit_swtpm_input_parity_golden() {
        let golden = include_str!("../../../tests/golden/runner-shape/swtpm-argv-minimal.txt");
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
            "swtpm argv golden drift - regenerate tests/golden/runner-shape/swtpm-argv-minimal.txt\n--- expected ---\n{expected}\n--- observed ---\n{observed}\n"
        );
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
                "/var/lib/d2b/vms/corp-vm/tpm/ctrl.sock".to_owned(),
            ]
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

    /// A swtpm worker reaches no device through its command line.
    ///
    /// The TPM worker's capability is delivered by its own admitted
    /// `DeviceBinding`; a path argument that names a device node is refused
    /// before the argv is rendered, so no caller can point the worker at a
    /// node its binding never admitted.
    #[test]
    fn refuses_a_path_argument_that_names_a_device_node() {
        for (mutate, field) in [
            (
                Box::new(|input: &mut SwtpmArgvInput| {
                    input.state_dir = "/dev/dri/renderD128".to_owned();
                }) as Box<dyn Fn(&mut SwtpmArgvInput)>,
                "state_dir",
            ),
            (
                Box::new(|input: &mut SwtpmArgvInput| {
                    input.ctrl_socket_path = "/dev/kvm".to_owned();
                }),
                "ctrl_socket_path",
            ),
            (
                Box::new(|input: &mut SwtpmArgvInput| {
                    input.server_socket_path = "/dev/dri/card0".to_owned();
                }),
                "server_socket_path",
            ),
            (
                Box::new(|input: &mut SwtpmArgvInput| {
                    input.log_path = "/dev/../var/lib/d2b/tpm/swtpm.log".to_owned();
                }),
                "log_path",
            ),
        ] {
            let mut input = audit_swtpm_input();
            mutate(&mut input);
            assert!(
                matches!(
                    generate_swtpm_argv(&input),
                    Err(SwtpmArgvError::DeviceNodeArgumentRefused)
                ),
                "{field} must not name a device node"
            );
        }

        let mut flush = audit_flush_input();
        flush.ctrl_socket_path = "/dev/dri/renderD128".to_owned();
        assert!(matches!(
            generate_swtpm_ioctl_flush_argv(&flush),
            Err(SwtpmArgvError::DeviceNodeArgumentRefused)
        ));
    }

    /// A declared payload cannot smuggle a socket owner or a free-form
    /// argument tail back in.
    ///
    /// `SwtpmArgvInput` denies unknown fields, so a serialized launch ticket
    /// that still carries the retired `uid`/`gid` owner or an `extraArgs`
    /// tail is refused at the type boundary rather than silently launching
    /// with a caller-chosen identity.
    #[test]
    fn a_declared_payload_cannot_carry_a_socket_owner_or_an_argument_tail() {
        let input = audit_swtpm_input();
        let mut payload = serde_json::to_value(&input).expect("the declared input serializes");
        payload["extraArgs"] = serde_json::json!(["--tpm2", "--daemon"]);
        assert!(
            serde_json::from_value::<SwtpmArgvInput>(payload).is_err(),
            "a free-form argv tail must not decode"
        );

        let mut payload = serde_json::to_value(&input).expect("the declared input serializes");
        payload["uid"] = serde_json::json!(0);
        assert!(
            serde_json::from_value::<SwtpmArgvInput>(payload).is_err(),
            "a caller-chosen socket owner must not decode"
        );
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

}
