//! The bounded host probe (U5): the capability classes, the minijail
//! platform gate, and the bounded metadata observations the Host reconciler
//! reads the machine through.
//!
//! The probe moved wholesale into this crate with the family's daemon-side
//! effects: every call that touches the machine - `/proc`,
//! `/etc/os-release`, `/dev`, `/sys/fs/cgroup`, `/run/user` - lives here,
//! and the one daemon-owned read (the minijail platform gate) arrives as
//! the declared [`crate::facets::MinijailPlatformGateSource`] facet whose
//! implementation the daemon host supplies. The bounded read and
//! socket-presence helpers the probe uses also moved with it, so this crate
//! reaches host state through no daemon runtime.

use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use d2b_provider_system_core::{
    HostCapabilityClass, HostProbeEffectPort, HostProbeMetadata, MinijailPlatformGate,
    SystemCoreError,
};

use crate::facets::MinijailPlatformGateSource;

/// The pipewire runtime socket name the probe looks for under the caller's
/// runtime dir (`/run/user/<uid>/pipewire-0`).
///
/// A host-state path name the probe reads, spelled here instead of imported
/// from the audio-pipewire family's vocabulary: the probe checks the socket
/// under the caller's own runtime dir and needs no sibling crate. The name
/// is shared by contract - the audio-pipewire family declares the same
/// kernel-visible name - and the agreement is pinned by the daemon's
/// composition-root test.
pub const PIPEWIRE_RUNTIME_SOCKET: &str = "pipewire-0";

/// The kernel module the USBIP core stack loads as (`/sys/module/usbip_core`).
///
/// A host-state module name the probe reads, spelled here instead of
/// imported from the device-usbip family's vocabulary: the probe checks the
/// module directory under `/sys/module` and needs no sibling crate. The name
/// is shared by contract - the device-usbip family declares the same
/// kernel-visible name - and the agreement is pinned by the daemon's
/// composition-root test.
pub const USBIP_CORE_MODULE: &str = "usbip_core";

/// The kernel module the USBIP host driver loads as (`/sys/module/usbip_host`).
///
/// A host-state module name the probe reads, spelled here instead of
/// imported from the device-usbip family's vocabulary: the probe checks the
/// module directory under `/sys/module` and needs no sibling crate. The name
/// is shared by contract - the device-usbip family declares the same
/// kernel-visible name - and the agreement is pinned by the daemon's
/// composition-root test.
pub const USBIP_HOST_MODULE: &str = "usbip_host";

/// The bounded probe the Host effects service reads the machine through:
/// capability classes, the minijail platform gate, and the bounded metadata
/// observations.
pub(crate) struct HostProbe {
    user_uid: u32,
    /// The daemon-supplied minijail platform gate source (U5): the same
    /// bounded gate the daemon's minijail Provider is constructed from.
    minijail_gate: Arc<dyn MinijailPlatformGateSource>,
}

impl HostProbe {
    /// Build the probe for the current process, reading the `Pidfd`
    /// capability and the platform gate through the daemon-supplied facet.
    pub(crate) fn new(minijail_gate: Arc<dyn MinijailPlatformGateSource>) -> Self {
        Self {
            user_uid: nix::unistd::Uid::current().as_raw(),
            minijail_gate,
        }
    }
}

/// Build the crate's production probe over the daemon-supplied minijail
/// platform gate source (U5): the bounded probe the family's effects run
/// over, reading host state itself and the one daemon-owned read (the
/// platform gate) through the facet.
pub fn production_probe(
    minijail_gate: Arc<dyn MinijailPlatformGateSource>,
) -> Arc<dyn HostProbeEffectPort> {
    Arc::new(HostProbe::new(minijail_gate))
}

impl HostProbe {
    fn kernel_release() -> Result<String, SystemCoreError> {
        read_bounded("/proc/sys/kernel/osrelease", 64)
            .map(|release| release.trim().to_owned())
            .map_err(|_| SystemCoreError::HostProbeFailed)
    }

    fn os_name() -> Result<String, SystemCoreError> {
        let release =
            read_bounded("/etc/os-release", 16 * 1024).map_err(|_| SystemCoreError::HostProbeFailed)?;
        Ok(release
            .lines()
            .find_map(|line| line.strip_prefix("NAME="))
            .map(|name| name.trim_matches('"').to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "unknown".to_owned()))
    }

    fn runtime_path(&self, name: &str) -> PathBuf {
        Path::new("/run/user")
            .join(self.user_uid.to_string())
            .join(name)
    }

    async fn has_dri_node(prefix: &str) -> bool {
        let Ok(mut entries) = tokio::fs::read_dir("/dev/dri").await else {
            return false;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(prefix))
            {
                return true;
            }
        }
        false
    }

    async fn active_process_count() -> Result<u32, SystemCoreError> {
        let mut count = 0_u32;
        let Ok(mut entries) = tokio::fs::read_dir("/proc").await else {
            return Err(SystemCoreError::HostProbeFailed);
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.bytes().all(|byte| byte.is_ascii_digit()))
            {
                count = count.saturating_add(1);
            }
        }
        Ok(count)
    }
}

#[async_trait::async_trait]
impl HostProbeEffectPort for HostProbe {
    async fn probe(&self, capability: HostCapabilityClass) -> Result<bool, SystemCoreError> {
        let available = match capability {
            HostCapabilityClass::Kvm => Path::new("/dev/kvm").is_file(),
            HostCapabilityClass::Pidfd => {
                // The same bounded gate the daemon's minijail Provider is
                // constructed from, supplied as the declared facet.
                let gate = self.minijail_gate.platform_gate();
                gate.kernel_major > 5 || (gate.kernel_major == 5 && gate.kernel_minor >= 3)
            }
            HostCapabilityClass::CgroupV2 => {
                Path::new("/sys/fs/cgroup/cgroup.controllers").is_file()
            }
            HostCapabilityClass::UserNamespace => Path::new("/proc/self/ns/user").exists(),
            HostCapabilityClass::Virtiofs => Path::new("/dev/fuse").is_file(),
            HostCapabilityClass::AudioPipewire => {
                is_socket(&self.runtime_path(PIPEWIRE_RUNTIME_SOCKET))
            }
            HostCapabilityClass::Wayland => {
                is_socket(&self.runtime_path("wayland-0"))
            }
            HostCapabilityClass::GpuRender => Self::has_dri_node("renderD").await,
            HostCapabilityClass::GpuDrm => Self::has_dri_node("card").await,
            HostCapabilityClass::Tpm2 => {
                Path::new("/dev/tpmrm0").is_file() || Path::new("/dev/tpm0").is_file()
            }
            HostCapabilityClass::Usbip => [USBIP_CORE_MODULE, USBIP_HOST_MODULE]
                .iter()
                .any(|module| Path::new(&format!("/sys/module/{}", module)).exists()),
        };
        Ok(available)
    }

    async fn platform(&self) -> Result<MinijailPlatformGate, SystemCoreError> {
        Ok(self.minijail_gate.platform_gate())
    }

    async fn metadata(&self) -> Result<HostProbeMetadata, SystemCoreError> {
        Ok(HostProbeMetadata {
            kernel_release: Self::kernel_release()?,
            os_name: Self::os_name()?,
            user_manager_available: self.runtime_path("systemd").is_dir(),
            active_process_count: Self::active_process_count().await?,
        })
    }
}

/// Bounded file read: at most `limit` bytes, UTF-8, or refuse.
///
/// Moved out of the daemon's runtime support with the probe (U5): the host
/// family's probe reaches `/proc` and `/etc/os-release` through the same
/// bounded seat it always did, now inside the owning crate.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn read_bounded(path: impl AsRef<Path>, limit: usize) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(limit.min(4096));
    file.by_ref()
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bounded host probe exceeded limit",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "host probe was not utf-8"))
}

/// Whether `path` exists and is a socket.
///
/// Moved out of the daemon's runtime support with the probe (U5): the
/// pipewire/wayland runtime socket checks now live inside the owning crate.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn is_socket(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_provider_system_core::MinijailPlatformGate;
    use crate::test_support::RecordingMinijailGate;

    /// The production probe returns the bounded host observations (the
    /// daemon-side test moved with the family's effects, U5): the metadata
    /// is bounded, the platform gate comes from the supplied facet, and the
    /// pidfd capability agrees with the gate the report carries.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn production_host_probe_returns_bounded_host_observations() {
        let gate = RecordingMinijailGate::new(MinijailPlatformGate::new(6, 9, true));
        let probe = production_probe(gate);
        let metadata = probe
            .metadata()
            .await
            .expect("the local host metadata probe succeeds");
        assert!(!metadata.kernel_release.is_empty());
        assert!(metadata.kernel_release.len() <= 64);
        assert!(metadata.os_name.len() <= 128);
        // Non-degenerate guard, not a value check: the same live `/proc`
        // enumeration the daemon binding test guards; a regression that
        // degenerates the count to a constant zero would otherwise pass this
        // crate's own suite. Any running machine has at least one process -
        // this probe runs inside one - so a genuine count is never below 1
        // and this cannot flake; no machine-tied range is asserted.

        assert!(
            metadata.active_process_count >= 1,
            "process count is degenerate: {}",
            metadata.active_process_count
        );
        let platform = probe
            .platform()
            .await
            .expect("the local platform probe succeeds");
        assert!(platform.kernel_major > 0);
        let pidfd = probe
            .probe(HostCapabilityClass::Pidfd)
            .await
            .expect("the pidfd capability probe succeeds");
        assert_eq!(
            pidfd,
            platform.kernel_major > 5
                || (platform.kernel_major == 5 && platform.kernel_minor >= 3)
        );
    }
}
