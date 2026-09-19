//! Daemon-side Host/User family effects.
//!
//! The `d2b-provider-host` and `d2b-provider-user` crates own the drivers and
//! their effect ports; this module implements those ports over the preserved
//! system-core realization: the bounded capability/platform/metadata probe the
//! `HostReconciler` consumes (with its preserved degraded fallback for a probe
//! that cannot complete), and the local NSS discovery the `UserReconciler`
//! drives. Every call that touches the machine - `/proc`, `/etc/os-release`,
//! `/dev`, `/run/user`, and the account database - stays here, so the family
//! crates reach no host state of their own.

use std::collections::BTreeSet;
use std::path::Path;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourcePhase, ResourceRef, host::HostSpec, user::UserSpec};
use d2b_provider_host::HostDriverEffects;
use d2b_provider_system_core::{
    DiscoveredUser, HostCapabilityClass, HostObservationReport, HostProbeEffectPort,
    HostProbeMetadata, HostReconciler, MinijailPlatformGate, SystemCoreError, UserBinding,
    UserDiscoveryEffectPort, UserIdentityDigest, UserObservation, UserReconciler,
    UserStatusReport,
};
use d2b_provider_user::UserDriverEffects;

/// Production effects over the local host for the `Host` type: the bounded
/// probe adapter the old core runner wired, with its preserved fallback.
pub(crate) struct ProductionHostDriverEffects;

#[async_trait]
impl HostDriverEffects for ProductionHostDriverEffects {
    async fn observe_host(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
    ) -> Result<HostObservationReport, String> {
        match HostReconciler::new()
            .reconcile_with_probe(
                host_ref,
                provider_ref,
                spec,
                &HostProbe::current(),
                &BTreeSet::new(),
                false,
            )
            .await
        {
            Ok(report) => Ok(report),
            Err(probe_error) => {
                // Preserved fallback: a probe that cannot complete still
                // publishes the spec decision as a degraded observation
                // rather than failing the resource.
                let mut status = HostReconciler::new()
                    .reconcile(host_ref, provider_ref, spec)
                    .map_err(|error| format!("{probe_error}; {error}"))?;
                status.phase = ResourcePhase::Degraded;
                Ok(HostObservationReport {
                    status,
                    capabilities: Vec::new(),
                    kernel_release: "unknown".to_owned(),
                    os_name: "unknown".to_owned(),
                    user_manager_available: false,
                    active_process_count: 0,
                    minijail_ready: false,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Production host probe (moved with the family's daemon-side effects)
// ---------------------------------------------------------------------------

/// The bounded probe the preserved `HostReconciler` reads the machine
/// through: capability classes, the minijail platform gate, and the bounded
/// metadata observations.
struct HostProbe {
    user_uid: u32,
}

impl HostProbe {
    fn current() -> Self {
        Self {
            user_uid: nix::unistd::Uid::current().as_raw(),
        }
    }

    fn kernel_release() -> Result<String, SystemCoreError> {
        d2bd_runtime::resource_runtime_support::read_bounded("/proc/sys/kernel/osrelease", 64)
            .map(|release| release.trim().to_owned())
            .map_err(|_| SystemCoreError::HostProbeFailed)
    }

    fn os_name() -> Result<String, SystemCoreError> {
        let release =
            d2bd_runtime::resource_runtime_support::read_bounded("/etc/os-release", 16 * 1024)
                .map_err(|_| SystemCoreError::HostProbeFailed)?;
        Ok(release
            .lines()
            .find_map(|line| line.strip_prefix("NAME="))
            .map(|name| name.trim_matches('"').to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "unknown".to_owned()))
    }

    fn runtime_path(&self, name: &str) -> std::path::PathBuf {
        Path::new("/run/user")
            .join(self.user_uid.to_string())
            .join(name)
    }

    async fn has_render_node() -> bool {
        let Ok(mut entries) = tokio::fs::read_dir("/dev/dri").await else {
            return false;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("renderD"))
            {
                return true;
            }
        }
        false
    }

    async fn has_primary_drm_node() -> bool {
        let Ok(mut entries) = tokio::fs::read_dir("/dev/dri").await else {
            return false;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("card"))
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

impl HostProbeEffectPort for HostProbe {
    async fn probe(&self, capability: HostCapabilityClass) -> Result<bool, SystemCoreError> {
        let available = match capability {
            HostCapabilityClass::Kvm => Path::new("/dev/kvm").is_file(),
            HostCapabilityClass::Pidfd => {
                let gate = crate::process_provider_runtime::detect_minijail_platform_gate();
                gate.kernel_major > 5 || (gate.kernel_major == 5 && gate.kernel_minor >= 3)
            }
            HostCapabilityClass::CgroupV2 => {
                Path::new("/sys/fs/cgroup/cgroup.controllers").is_file()
            }
            HostCapabilityClass::UserNamespace => Path::new("/proc/self/ns/user").exists(),
            HostCapabilityClass::Virtiofs => Path::new("/dev/fuse").is_file(),
            HostCapabilityClass::AudioPipewire => {
                d2bd_runtime::resource_runtime_support::is_socket(&self.runtime_path("pipewire-0"))
            }
            HostCapabilityClass::Wayland => {
                d2bd_runtime::resource_runtime_support::is_socket(&self.runtime_path("wayland-0"))
            }
            HostCapabilityClass::GpuRender => Self::has_render_node().await,
            HostCapabilityClass::GpuDrm => Self::has_primary_drm_node().await,
            HostCapabilityClass::Tpm2 => {
                Path::new("/dev/tpmrm0").is_file() || Path::new("/dev/tpm0").is_file()
            }
            HostCapabilityClass::Usbip => {
                Path::new("/sys/module/usbip_core").exists()
                    || Path::new("/sys/module/usbip_host").exists()
            }
        };
        Ok(available)
    }

    async fn platform(&self) -> Result<MinijailPlatformGate, SystemCoreError> {
        let gate = crate::process_provider_runtime::detect_minijail_platform_gate();
        Ok(MinijailPlatformGate::new(
            gate.kernel_major,
            gate.kernel_minor,
            gate.cgroup_kill_available,
        ))
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

// ---------------------------------------------------------------------------
// Production local User discovery (moved with the family's daemon-side
// effects)
// ---------------------------------------------------------------------------

/// Production effects over the local host for the `User` type: the NSS
/// discovery adapter the old core runner wired.
pub(crate) struct ProductionUserDriverEffects;

#[async_trait]
impl UserDriverEffects for ProductionUserDriverEffects {
    async fn observe_user(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<UserStatusReport, String> {
        UserReconciler::new(LocalUserDiscovery)
            .reconcile(user_ref, spec)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LocalUserDiscovery;

impl UserDiscoveryEffectPort for LocalUserDiscovery {
    async fn discover(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<Option<DiscoveredUser>, SystemCoreError> {
        discover_local_user(user_ref, spec).await
    }
}

async fn discover_local_user(
    user_ref: &ResourceRef,
    spec: &UserSpec,
) -> Result<Option<DiscoveredUser>, SystemCoreError> {
    use nix::unistd::{Group, User};
    use sha2::{Digest, Sha256};

    let username = spec.os_username().as_str();
    let user = User::from_name(username).map_err(|_| SystemCoreError::DiscoveryUnavailable)?;
    let Some(user) = user else {
        return Ok(None);
    };

    let mut digest = Sha256::new();
    digest.update(b"d2b-system-core-user-v1");
    digest.update(user_ref.name().as_str().as_bytes());
    digest.update([0]);
    digest.update(username.as_bytes());
    digest.update([0]);
    digest.update(user.uid.as_raw().to_le_bytes());
    digest.update(user.gid.as_raw().to_le_bytes());

    let mut verified = std::collections::BTreeSet::from([UserBinding::NssRecord]);
    if Group::from_gid(user.gid)
        .map_err(|_| SystemCoreError::DiscoveryUnavailable)?
        .is_some()
    {
        verified.insert(UserBinding::PrimaryGroup);
    }

    let mut groups_verified = true;
    for group in spec.groups() {
        let Some(group_record) = Group::from_name(group.as_str())
            .map_err(|_| SystemCoreError::DiscoveryUnavailable)?
        else {
            groups_verified = false;
            tracing::debug!(
                user = %username,
                group = %group.as_str(),
                "system-core user group record missing; membership unverified",
            );
            continue;
        };
        digest.update([0]);
        digest.update(group.as_str().as_bytes());
        if !group_record.mem.iter().any(|member| member == username) {
            groups_verified = false;
        }
    }
    if groups_verified && !spec.groups().is_empty() {
        verified.insert(UserBinding::GroupMemberships);
    }

    Ok(Some(DiscoveredUser {
        identity: UserIdentityDigest::from_bytes(digest.finalize().into()),
        observed: UserObservation::from_verified(verified),
    }))
}

#[cfg(test)]
mod tests {
    use d2b_provider_system_core::{HostCapabilityClass, HostProbeEffectPort};

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn production_host_probe_returns_bounded_host_observations() {
        let probe = super::HostProbe::current();
        let metadata = probe
            .metadata()
            .await
            .expect("the local host metadata probe succeeds");
        assert!(!metadata.kernel_release.is_empty());
        assert!(metadata.kernel_release.len() <= 64);
        assert!(metadata.os_name.len() <= 128);
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
