//! Cloud Hypervisor argv builder.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Network handoff selected by Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChNetHandoff {
    /// Pass a pre-opened TAP fd.
    TapFd,
    /// Use a broker-owned persistent TAP attachment.
    PersistentTap,
}

/// One virtiofs share.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChFsShare {
    /// Opaque effect-resolved socket locator.
    pub socket: String,
    /// Guest mount tag.
    pub tag: String,
}

impl fmt::Debug for ChFsShare {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChFsShare")
            .field("socket", &"<redacted>")
            .field("tag", &"<redacted>")
            .finish()
    }
}

/// One network interface descriptor.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChNetIface {
    /// Guest MAC.
    pub mac: String,
    /// Persistent TAP name when selected by Core.
    pub tap_ifname: String,
    /// Inherited fd slot when selected by Core.
    pub tap_fd: Option<i32>,
}

impl fmt::Debug for ChNetIface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChNetIface")
            .field("mac", &"<redacted>")
            .field("tap_ifname", &"<redacted>")
            .field("tap_fd", &"<redacted>")
            .finish()
    }
}

/// Primary vsock descriptor.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChVsock {
    /// Guest CID.
    pub cid: u32,
    /// Effect-resolved socket token.
    pub socket: String,
}

impl fmt::Debug for ChVsock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChVsock")
            .field("cid", &"<redacted>")
            .field("socket", &"<redacted>")
            .finish()
    }
}

/// All effect-resolved VMM argv inputs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChArgvInput {
    /// VM display name used only for `arg0`.
    pub vm_name: String,
    /// Resolved Cloud Hypervisor executable.
    pub ch_binary_path: String,
    /// VCPU count.
    pub cpus: u32,
    /// Watchdog flag.
    pub watchdog: bool,
    /// Resolved kernel artifact.
    pub kernel_path: String,
    /// Optional initramfs.
    pub initramfs_path: Option<String>,
    /// Signed command line.
    pub cmdline: String,
    /// Seccomp mode.
    pub seccomp: String,
    /// Memory setting.
    pub memory: String,
    /// OEM strings.
    pub platform_oem_strings: Vec<String>,
    /// Console.
    pub console: String,
    /// Serial.
    pub serial: String,
    /// Primary vsock.
    pub primary_vsock: Option<ChVsock>,
    /// Additional vsock sockets.
    pub extra_vsock: Vec<String>,
    /// Virtiofs shares.
    pub fs_shares: Vec<ChFsShare>,
    /// API socket.
    pub api_socket_path: String,
    /// Network interfaces.
    pub net_ifaces: Vec<ChNetIface>,
    /// Handoff mode.
    pub net_handoff: ChNetHandoff,
    /// Signed extra args.
    pub extra_args: Vec<String>,
}

impl fmt::Debug for ChArgvInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChArgvInput")
            .field("vm_name", &"<redacted>")
            .field("cpus", &self.cpus)
            .field("watchdog", &self.watchdog)
            .field("console", &self.console)
            .field("serial", &self.serial)
            .field("net_ifaces", &self.net_ifaces.len())
            .field("fs_shares", &self.fs_shares.len())
            .finish()
    }
}

/// VMM argv validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChArgvError {
    /// VM name was empty.
    EmptyVmName,
    /// Binary path was not absolute.
    InvalidBinary,
    /// VCPU count was zero.
    ZeroCpus,
    /// Kernel was absent.
    EmptyKernel,
    /// TAP fd was not supplied.
    TapFdMissing,
    /// TAP interface was not supplied.
    TapIfnameMissing,
}

/// Render the argv vector in deterministic order.
pub fn generate_ch_argv(input: &ChArgvInput) -> Result<Vec<String>, ChArgvError> {
    if input.vm_name.is_empty() {
        return Err(ChArgvError::EmptyVmName);
    }
    if input.ch_binary_path.is_empty() || !input.ch_binary_path.starts_with('/') {
        return Err(ChArgvError::InvalidBinary);
    }
    if input.cpus == 0 {
        return Err(ChArgvError::ZeroCpus);
    }
    if input.kernel_path.is_empty() {
        return Err(ChArgvError::EmptyKernel);
    }
    for iface in &input.net_ifaces {
        match input.net_handoff {
            ChNetHandoff::TapFd if iface.tap_fd.is_none() => return Err(ChArgvError::TapFdMissing),
            ChNetHandoff::PersistentTap if iface.tap_ifname.is_empty() => {
                return Err(ChArgvError::TapIfnameMissing);
            }
            _ => {}
        }
    }
    let mut argv = vec![input.ch_binary_path.clone()];
    argv.extend(["--cpus".to_owned(), format!("boot={}", input.cpus)]);
    if input.watchdog {
        argv.push("--watchdog".to_owned());
    }
    argv.extend(["--kernel".to_owned(), input.kernel_path.clone()]);
    if let Some(initramfs) = &input.initramfs_path {
        argv.extend(["--initramfs".to_owned(), initramfs.clone()]);
    }
    argv.extend([
        "--cmdline".to_owned(),
        input.cmdline.clone(),
        "--seccomp".to_owned(),
        input.seccomp.clone(),
        "--memory".to_owned(),
        input.memory.clone(),
    ]);
    if !input.platform_oem_strings.is_empty() {
        argv.extend([
            "--platform".to_owned(),
            format!("oem_strings=[{}]", input.platform_oem_strings.join(",")),
        ]);
    }
    argv.extend([
        "--console".to_owned(),
        input.console.clone(),
        "--serial".to_owned(),
        input.serial.clone(),
    ]);
    if let Some(vsock) = &input.primary_vsock {
        argv.extend([
            "--vsock".to_owned(),
            format!("cid={},socket={}", vsock.cid, vsock.socket),
        ]);
    }
    for socket in &input.extra_vsock {
        argv.extend(["--vsock".to_owned(), format!("socket={socket}")]);
    }
    if !input.fs_shares.is_empty() {
        argv.push("--fs".to_owned());
        argv.extend(
            input
                .fs_shares
                .iter()
                .map(|share| format!("socket={},tag={}", share.socket, share.tag)),
        );
    }
    argv.extend(["--api-socket".to_owned(), input.api_socket_path.clone()]);
    if !input.net_ifaces.is_empty() {
        argv.push("--net".to_owned());
        for iface in &input.net_ifaces {
            let value = match input.net_handoff {
                ChNetHandoff::TapFd => format!(
                    "fd={},mac={}",
                    iface.tap_fd.expect("validated above"),
                    iface.mac
                ),
                ChNetHandoff::PersistentTap => {
                    format!("mac={},tap={}", iface.mac, iface.tap_ifname)
                }
            };
            argv.push(value);
        }
    }
    argv.extend(input.extra_args.iter().cloned());
    Ok(argv)
}
