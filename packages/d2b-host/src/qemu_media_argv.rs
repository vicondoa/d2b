//! QEMU media runtime argv scaffold.
//!
//! The qemu-media process starts paused with a minimal, fd-backed baseline:
//! defaults and user config are disabled, the host TAP is referenced only by an
//! inherited fd, and media source paths are hotplugged later through QMP.

use serde::{Deserialize, Serialize};

/// All inputs required to render the QEMU media scaffold argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaArgvInput {
    /// Absolute store path to the host QEMU system binary.
    pub qemu_binary_path: String,
    /// VM name; used by [`exec_arg0`] and the QEMU process name.
    pub vm_name: String,
    /// QMP socket QEMU should listen on.
    pub qmp_socket_path: String,
    /// Guest MAC address for the fd-backed TAP netdev.
    pub mac_address: String,
    /// Inherited TAP fd number. The broker opens the host TAP and passes it.
    #[serde(default = "default_tap_fd")]
    pub tap_fd: i32,
    /// Guest RAM in MiB.
    #[serde(default = "default_memory_mib")]
    pub memory_mib: u32,
    /// Guest vCPU count.
    #[serde(default = "default_vcpu")]
    pub vcpu: u32,
    /// Lock guest RAM into host memory with QEMU overcommit mem-lock.
    #[serde(default)]
    pub lock_memory: bool,
    /// Exclude guest RAM from host/QEMU core dumps.
    #[serde(default = "default_true")]
    pub exclude_memory_from_core_dump: bool,
    /// Disable KSM merging for guest RAM.
    #[serde(default = "default_true")]
    pub disable_memory_merge: bool,
    /// Broker-opened console fd (host end of a socketpair). When `Some`,
    /// the argv emits `-chardev fd,id=con0,fd=N -serial chardev:con0`
    /// so QEMU's serial console is connected to the fd-backed stream
    /// rather than discarded with `-serial none`. The broker creates the
    /// socketpair, passes one end to QEMU via this fd, and retains the
    /// other end for the drainer (ADR 0041).
    ///
    /// The fd must be ≥ 3 (not stdin/stdout/stderr).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_fd: Option<i32>,
}

/// Errors the QEMU media argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum QemuMediaArgvError {
    InvalidQemuBinaryPath { path: String },
    EmptyVmName,
    InvalidQmpSocketPath { path: String },
    InvalidMacAddress { value: String },
    InvalidTapFd { fd: i32 },
    InvalidMemoryMiB { value: u32 },
    InvalidVcpu { value: u32 },
    /// `console_fd` must be ≥ 3 to avoid colliding with stdin/stdout/stderr.
    InvalidConsoleFd { fd: i32 },
}

/// Render the paused fd-backed qemu-media baseline argv.
pub fn generate_qemu_media_argv(
    input: &QemuMediaArgvInput,
) -> Result<Vec<String>, QemuMediaArgvError> {
    if input.qemu_binary_path.is_empty() || !input.qemu_binary_path.starts_with('/') {
        return Err(QemuMediaArgvError::InvalidQemuBinaryPath {
            path: input.qemu_binary_path.clone(),
        });
    }
    if input.vm_name.is_empty() {
        return Err(QemuMediaArgvError::EmptyVmName);
    }
    if !valid_qmp_socket_path(&input.qmp_socket_path) {
        return Err(QemuMediaArgvError::InvalidQmpSocketPath {
            path: input.qmp_socket_path.clone(),
        });
    }
    if !valid_mac_address(&input.mac_address) {
        return Err(QemuMediaArgvError::InvalidMacAddress {
            value: input.mac_address.clone(),
        });
    }
    if input.tap_fd < 3 {
        return Err(QemuMediaArgvError::InvalidTapFd { fd: input.tap_fd });
    }
    if input.memory_mib == 0 {
        return Err(QemuMediaArgvError::InvalidMemoryMiB {
            value: input.memory_mib,
        });
    }
    if input.vcpu == 0 {
        return Err(QemuMediaArgvError::InvalidVcpu { value: input.vcpu });
    }
    if let Some(fd) = input.console_fd {
        if fd < 3 {
            return Err(QemuMediaArgvError::InvalidConsoleFd { fd });
        }
    }

    let mut memory_backend = vec![
        "memory-backend-ram".to_owned(),
        "id=nlram".to_owned(),
        format!("size={}M", input.memory_mib),
        format!(
            "dump={}",
            if input.exclude_memory_from_core_dump {
                "off"
            } else {
                "on"
            }
        ),
        format!(
            "merge={}",
            if input.disable_memory_merge {
                "off"
            } else {
                "on"
            }
        ),
    ];
    if input.lock_memory {
        memory_backend.push("prealloc=on".to_owned());
    }

    let mut argv = vec![
        input.qemu_binary_path.clone(),
        "-nodefaults".to_owned(),
        "-no-user-config".to_owned(),
        "-S".to_owned(),
        "-object".to_owned(),
        memory_backend.join(","),
        "-machine".to_owned(),
        "q35,accel=kvm,usb=off,memory-backend=nlram".to_owned(),
        "-m".to_owned(),
        format!("{}M", input.memory_mib),
        "-smp".to_owned(),
        input.vcpu.to_string(),
    ];
    if input.lock_memory {
        argv.extend(["-overcommit".to_owned(), "mem-lock=on".to_owned()]);
    }
    argv.extend([
        "-device".to_owned(),
        "usb-ehci,id=ehci".to_owned(),
        "-device".to_owned(),
        "virtio-vga".to_owned(),
        "-display".to_owned(),
        "gtk,gl=off,show-cursor=on".to_owned(),
        "-device".to_owned(),
        "usb-kbd,bus=ehci.0".to_owned(),
        "-device".to_owned(),
        "usb-tablet,bus=ehci.0".to_owned(),
        "-netdev".to_owned(),
        format!("tap,id=nl0,fd={},vhost=off", input.tap_fd),
        "-device".to_owned(),
        format!("virtio-net-pci,netdev=nl0,mac={}", input.mac_address),
        "-qmp".to_owned(),
        format!("unix:{},server=on,wait=off", input.qmp_socket_path),
        "-monitor".to_owned(),
        "none".to_owned(),
    ]);
    // Emit console chardev when a broker-owned fd is provided; otherwise
    // suppress the serial port entirely (-serial none).
    if let Some(fd) = input.console_fd {
        argv.extend([
            "-chardev".to_owned(),
            format!("fd,id=con0,fd={fd}"),
            "-serial".to_owned(),
            "chardev:con0".to_owned(),
        ]);
    } else {
        argv.extend(["-serial".to_owned(), "none".to_owned()]);
    }
    argv.extend([
        "-parallel".to_owned(),
        "none".to_owned(),
        "-name".to_owned(),
        format!("d2b-{}-qemu-media", input.vm_name),
    ]);
    Ok(argv)
}

/// `arg0` for the qemu-media runner.
pub fn exec_arg0(input: &QemuMediaArgvInput) -> Result<String, QemuMediaArgvError> {
    if input.vm_name.is_empty() {
        return Err(QemuMediaArgvError::EmptyVmName);
    }
    Ok(format!("d2b-qemu-media@{}", input.vm_name))
}

fn valid_qmp_socket_path(path: &str) -> bool {
    path.starts_with("/run/d2b/vms/") && path.ends_with("/qmp.sock") && !path.contains('\n')
}

fn default_tap_fd() -> i32 {
    10
}

fn default_memory_mib() -> u32 {
    4096
}

fn default_vcpu() -> u32 {
    2
}

fn default_true() -> bool {
    true
}

fn valid_mac_address(value: &str) -> bool {
    let parts: Vec<_> = value.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> QemuMediaArgvInput {
        QemuMediaArgvInput {
            qemu_binary_path: "/nix/store/QEMUQEMUQEMU-qemu/bin/qemu-system-x86_64".to_owned(),
            vm_name: "media".to_owned(),
            qmp_socket_path: "/run/d2b/vms/media/qmp.sock".to_owned(),
            mac_address: "02:76:53:AE:57:2A".to_owned(),
            tap_fd: 10,
            memory_mib: 4096,
            vcpu: 2,
            lock_memory: false,
            exclude_memory_from_core_dump: true,
            disable_memory_merge: true,
            console_fd: None,
        }
    }

    #[test]
    fn baseline_argv_is_paused_fd_backed_and_not_live_media() {
        let argv = generate_qemu_media_argv(&input()).unwrap();
        let joined = argv.join(" ");

        assert!(argv[0].ends_with("/qemu-system-x86_64"));
        assert!(joined.contains("-nodefaults -no-user-config -S"));
        assert!(
            joined.contains("-object memory-backend-ram,id=nlram,size=4096M,dump=off,merge=off")
        );
        assert!(joined.contains("-machine q35,accel=kvm,usb=off,memory-backend=nlram"));
        assert!(joined.contains("-m 4096M"));
        assert!(joined.contains("-smp 2"));
        assert!(!joined.contains("mem-lock=on"));
        assert!(joined.contains("-device usb-ehci,id=ehci"));
        assert!(joined.contains("-device usb-kbd,bus=ehci.0"));
        assert!(joined.contains("-device usb-tablet,bus=ehci.0"));
        assert!(joined.contains("-display gtk,gl=off,show-cursor=on"));
        assert!(joined.contains("-netdev tap,id=nl0,fd=10,vhost=off"));
        assert!(joined.contains("-device virtio-net-pci,netdev=nl0,mac=02:76:53:AE:57:2A"));
        assert!(joined.contains("-qmp unix:/run/d2b/vms/media/qmp.sock,server=on,wait=off"));
        assert!(joined.contains("-name d2b-media-qemu-media"));
        assert!(!joined.contains("/var/lib/d2b/media"));
        assert!(!joined.contains("/dev/vhost-net"));
        assert!(!joined.contains("vhostfd="));
        assert!(!joined.contains("-drive"));
        assert!(!joined.contains("-blockdev"));
    }

    #[test]
    fn lock_memory_adds_qemu_memlock_and_preallocation() {
        let mut input = input();
        input.lock_memory = true;
        let argv = generate_qemu_media_argv(&input).unwrap();
        let joined = argv.join(" ");

        assert!(
            joined
                .contains("memory-backend-ram,id=nlram,size=4096M,dump=off,merge=off,prealloc=on")
        );
        assert!(joined.contains("-overcommit mem-lock=on"));
    }

    #[test]
    fn exec_arg0_matches_runner_role() {
        assert_eq!(exec_arg0(&input()).unwrap(), "d2b-qemu-media@media");
    }

    #[test]
    fn rejects_relative_binary_path() {
        let mut input = input();
        input.qemu_binary_path = "qemu-system-x86_64".to_owned();
        assert!(matches!(
            generate_qemu_media_argv(&input),
            Err(QemuMediaArgvError::InvalidQemuBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_non_d2b_qmp_socket() {
        let mut input = input();
        input.qmp_socket_path = "/var/lib/d2b/media/qmp.sock".to_owned();
        assert!(matches!(
            generate_qemu_media_argv(&input),
            Err(QemuMediaArgvError::InvalidQmpSocketPath { .. })
        ));
    }

    #[test]
    fn rejects_invalid_tap_fd_and_mac_address() {
        let mut low_fd_input = input();
        low_fd_input.tap_fd = 2;
        assert!(matches!(
            generate_qemu_media_argv(&low_fd_input),
            Err(QemuMediaArgvError::InvalidTapFd { fd: 2 })
        ));

        let mut bad_mac_input = input();
        bad_mac_input.mac_address = "not-a-mac".to_owned();
        assert!(matches!(
            generate_qemu_media_argv(&bad_mac_input),
            Err(QemuMediaArgvError::InvalidMacAddress { .. })
        ));
    }

    #[test]
    fn without_console_fd_serial_is_none() {
        let argv = generate_qemu_media_argv(&input()).unwrap();
        let joined = argv.join(" ");
        assert!(joined.contains("-serial none"), "serial should be none without console_fd");
        assert!(!joined.contains("-chardev"), "no chardev without console_fd");
    }

    #[test]
    fn with_console_fd_emits_chardev_and_serial_chardev() {
        let mut inp = input();
        inp.console_fd = Some(11);
        let argv = generate_qemu_media_argv(&inp).unwrap();
        let joined = argv.join(" ");
        assert!(
            joined.contains("-chardev fd,id=con0,fd=11"),
            "expected fd chardev: {joined}"
        );
        assert!(
            joined.contains("-serial chardev:con0"),
            "expected serial chardev ref: {joined}"
        );
        assert!(
            !joined.contains("-serial none"),
            "should not have -serial none when console_fd is set"
        );
    }

    #[test]
    fn rejects_console_fd_below_3() {
        let mut inp = input();
        inp.console_fd = Some(2);
        assert!(matches!(
            generate_qemu_media_argv(&inp),
            Err(QemuMediaArgvError::InvalidConsoleFd { fd: 2 })
        ));
        inp.console_fd = Some(0);
        assert!(matches!(
            generate_qemu_media_argv(&inp),
            Err(QemuMediaArgvError::InvalidConsoleFd { fd: 0 })
        ));
    }

    #[test]
    fn console_fd_3_is_valid_boundary() {
        let mut inp = input();
        inp.console_fd = Some(3);
        let argv = generate_qemu_media_argv(&inp).unwrap();
        assert!(argv.join(" ").contains("-chardev fd,id=con0,fd=3"));
    }
}
