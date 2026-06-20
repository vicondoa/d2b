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

    Ok(vec![
        input.qemu_binary_path.clone(),
        "-nodefaults".to_owned(),
        "-no-user-config".to_owned(),
        "-S".to_owned(),
        "-machine".to_owned(),
        "q35,accel=kvm,usb=off".to_owned(),
        "-device".to_owned(),
        "qemu-xhci,id=xhci,p2=15,p3=15".to_owned(),
        "-device".to_owned(),
        "virtio-vga".to_owned(),
        "-display".to_owned(),
        "gtk,gl=off,show-cursor=on".to_owned(),
        "-device".to_owned(),
        "usb-kbd,bus=xhci.0,port=1".to_owned(),
        "-device".to_owned(),
        "usb-tablet,bus=xhci.0,port=2".to_owned(),
        "-netdev".to_owned(),
        format!("tap,id=nl0,fd={},vhost=off", input.tap_fd),
        "-device".to_owned(),
        format!("virtio-net-pci,netdev=nl0,mac={}", input.mac_address),
        "-qmp".to_owned(),
        format!("unix:{},server=on,wait=off", input.qmp_socket_path),
        "-monitor".to_owned(),
        "none".to_owned(),
        "-serial".to_owned(),
        "none".to_owned(),
        "-parallel".to_owned(),
        "none".to_owned(),
        "-name".to_owned(),
        format!("nixling-{}-qemu-media", input.vm_name),
    ])
}

/// `arg0` for the qemu-media runner.
pub fn exec_arg0(input: &QemuMediaArgvInput) -> Result<String, QemuMediaArgvError> {
    if input.vm_name.is_empty() {
        return Err(QemuMediaArgvError::EmptyVmName);
    }
    Ok(format!("nixling-qemu-media@{}", input.vm_name))
}

fn valid_qmp_socket_path(path: &str) -> bool {
    path.starts_with("/run/nixling/vms/") && path.ends_with("/qmp.sock") && !path.contains('\n')
}

fn default_tap_fd() -> i32 {
    10
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
            qmp_socket_path: "/run/nixling/vms/media/qmp.sock".to_owned(),
            mac_address: "02:76:53:AE:57:2A".to_owned(),
            tap_fd: 10,
        }
    }

    #[test]
    fn baseline_argv_is_paused_fd_backed_and_not_live_media() {
        let argv = generate_qemu_media_argv(&input()).unwrap();
        let joined = argv.join(" ");

        assert!(argv[0].ends_with("/qemu-system-x86_64"));
        assert!(joined.contains("-nodefaults -no-user-config -S"));
        assert!(joined.contains("-machine q35,accel=kvm,usb=off"));
        assert!(joined.contains("-device qemu-xhci,id=xhci,p2=15,p3=15"));
        assert!(joined.contains("-device usb-kbd,bus=xhci.0,port=1"));
        assert!(joined.contains("-device usb-tablet,bus=xhci.0,port=2"));
        assert!(joined.contains("-display gtk,gl=off,show-cursor=on"));
        assert!(joined.contains("-netdev tap,id=nl0,fd=10,vhost=off"));
        assert!(joined.contains("-device virtio-net-pci,netdev=nl0,mac=02:76:53:AE:57:2A"));
        assert!(joined.contains("-qmp unix:/run/nixling/vms/media/qmp.sock,server=on,wait=off"));
        assert!(joined.contains("-name nixling-media-qemu-media"));
        assert!(!joined.contains("/var/lib/nixling/media"));
        assert!(!joined.contains("/dev/vhost-net"));
        assert!(!joined.contains("vhostfd="));
        assert!(!joined.contains("-drive"));
        assert!(!joined.contains("-blockdev"));
    }

    #[test]
    fn exec_arg0_matches_runner_role() {
        assert_eq!(exec_arg0(&input()).unwrap(), "nixling-qemu-media@media");
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
    fn rejects_non_nixling_qmp_socket() {
        let mut input = input();
        input.qmp_socket_path = "/var/lib/nixling/media/qmp.sock".to_owned();
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
}
