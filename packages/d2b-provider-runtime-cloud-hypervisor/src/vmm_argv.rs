//! Cloud Hypervisor argv builder.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One virtiofs share.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChNetIface {
    /// Guest MAC.
    pub mac: String,
    /// Inherited child-fd slot passed by the broker.
    pub tap_fd: i32,
}

impl fmt::Debug for ChNetIface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChNetIface")
            .field("mac", &"<redacted>")
            .field("tap_fd", &"<redacted>")
            .finish()
    }
}

/// Primary vsock descriptor.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ChArgvError {
    /// VM name was empty.
    EmptyVmName,
    /// Binary path was not absolute.
    InvalidBinary,
    /// VCPU count was zero.
    ZeroCpus,
    /// Kernel was absent.
    EmptyKernel,
    /// Child fd was not supplied.
    TapFdMissing,
}

fn host_input(input: &ChArgvInput) -> d2b_host_argv::ChArgvInput {
    d2b_host_argv::ChArgvInput {
        vm_name: input.vm_name.clone(),
        ch_binary_path: input.ch_binary_path.clone(),
        cpus: input.cpus,
        watchdog: input.watchdog,
        kernel_path: input.kernel_path.clone(),
        initramfs_path: input.initramfs_path.clone(),
        cmdline: input.cmdline.clone(),
        seccomp: input.seccomp.clone(),
        memory: input.memory.clone(),
        platform_oem_strings: input.platform_oem_strings.clone(),
        console: input.console.clone(),
        serial: input.serial.clone(),
        primary_vsock: input
            .primary_vsock
            .as_ref()
            .map(|vsock| d2b_host_argv::ChVsock {
                cid: vsock.cid,
                socket: vsock.socket.clone(),
            }),
        extra_vsock: input.extra_vsock.clone(),
        fs_shares: input
            .fs_shares
            .iter()
            .map(|share| d2b_host_argv::ChFsShare {
                socket: share.socket.clone(),
                tag: share.tag.clone(),
            })
            .collect(),
        api_socket_path: input.api_socket_path.clone(),
        net_ifaces: input
            .net_ifaces
            .iter()
            .map(|iface| d2b_host_argv::ChNetIface {
                mac: iface.mac.clone(),
                tap_fd: iface.tap_fd,
            })
            .collect(),
        extra_args: input.extra_args.clone(),
    }
}

/// Render the argv vector in deterministic order.
pub fn generate_ch_argv(input: &ChArgvInput) -> Result<Vec<String>, ChArgvError> {
    d2b_host_argv::generate_ch_argv(&host_input(input)).map_err(|error| match error {
        d2b_host_argv::ChArgvError::EmptyVmName => ChArgvError::EmptyVmName,
        d2b_host_argv::ChArgvError::InvalidChBinaryPath { .. } => ChArgvError::InvalidBinary,
        d2b_host_argv::ChArgvError::ZeroCpus => ChArgvError::ZeroCpus,
        d2b_host_argv::ChArgvError::EmptyKernelPath => ChArgvError::EmptyKernel,
        d2b_host_argv::ChArgvError::TapFdMissing { .. } => ChArgvError::TapFdMissing,
    })
}
