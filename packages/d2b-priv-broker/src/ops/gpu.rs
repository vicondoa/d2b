//! GPU-specific broker preflight and restart-adoption contracts.
//!
//! The live broker still resolves paths and runner details from its signed
//! bundle. This module keeps the GPU role matrix and identity checks in one
//! pure, testable operation surface.

use core::fmt;

use super::spawn_runner::SpawnRunnerPlan;

/// Closed GPU worker roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBrokerRole {
    /// Full virtio-gpu worker.
    Full,
    /// Render-node-only worker.
    RenderNode,
    /// Hardware video decoder worker.
    Video,
}

/// Closed GPU device grant classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GpuDeviceClass {
    /// KVM device.
    Kvm,
    /// DRM render node.
    Dri,
    /// DMA buffer device.
    Udmabuf,
    /// NVIDIA control device.
    NvidiaCtl,
    /// NVIDIA device node.
    NvidiaDevice,
    /// NVIDIA UVM device.
    NvidiaUvm,
}

/// Opaque broker-side identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GpuOpaqueIdentity([u8; 32]);

impl GpuOpaqueIdentity {
    /// Construct an identity at the trusted bundle/adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Whether this is the forbidden all-zero identity.
    pub fn is_zero(self) -> bool {
        self.0 == [0; 32]
    }
}

impl fmt::Debug for GpuOpaqueIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuOpaqueIdentity(<redacted>)")
    }
}

/// Opaque GPU launch request validated before a device open or clone.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuLaunchRequest {
    role: GpuBrokerRole,
    backing: GpuOpaqueIdentity,
    platform: GpuOpaqueIdentity,
    principal: GpuOpaqueIdentity,
    generation: u64,
    device_classes: Vec<GpuDeviceClass>,
}

impl GpuLaunchRequest {
    /// Construct a request from Core-resolved opaque identities.
    pub fn from_core(
        role: GpuBrokerRole,
        backing: GpuOpaqueIdentity,
        platform: GpuOpaqueIdentity,
        principal: GpuOpaqueIdentity,
        generation: u64,
        device_classes: Vec<GpuDeviceClass>,
    ) -> Result<Self, GpuBrokerError> {
        if backing.is_zero() || platform.is_zero() || principal.is_zero() || generation == 0 {
            return Err(GpuBrokerError::StaleIdentity);
        }
        let request = Self {
            role,
            backing,
            platform,
            principal,
            generation,
            device_classes,
        };
        request.validate()?;
        Ok(request)
    }

    /// Validate the closed role-to-device matrix.
    pub fn validate(&self) -> Result<(), GpuBrokerError> {
        let has = |class| self.device_classes.contains(&class);
        match self.role {
            GpuBrokerRole::Full
                if has(GpuDeviceClass::Kvm)
                    && has(GpuDeviceClass::Dri)
                    && has(GpuDeviceClass::Udmabuf)
                    && self.device_classes.iter().all(|class| {
                        matches!(
                            class,
                            GpuDeviceClass::Kvm
                                | GpuDeviceClass::Dri
                                | GpuDeviceClass::Udmabuf
                                | GpuDeviceClass::NvidiaCtl
                                | GpuDeviceClass::NvidiaDevice
                                | GpuDeviceClass::NvidiaUvm
                        )
                    }) =>
            {
                Ok(())
            }
            GpuBrokerRole::RenderNode if self.device_classes == [GpuDeviceClass::Dri] => Ok(()),
            GpuBrokerRole::Video
                if has(GpuDeviceClass::Dri)
                    && self.device_classes.iter().all(|class| {
                        matches!(
                            class,
                            GpuDeviceClass::Dri
                                | GpuDeviceClass::NvidiaCtl
                                | GpuDeviceClass::NvidiaDevice
                                | GpuDeviceClass::NvidiaUvm
                        )
                    }) =>
            {
                Ok(())
            }
            _ => Err(GpuBrokerError::RoleDeviceMismatch),
        }
    }

    /// Borrow the opaque backing identity.
    pub const fn backing(&self) -> GpuOpaqueIdentity {
        self.backing
    }

    /// Borrow the opaque platform identity.
    pub const fn platform(&self) -> GpuOpaqueIdentity {
        self.platform
    }

    /// Borrow the expected worker principal.
    pub const fn principal(&self) -> GpuOpaqueIdentity {
        self.principal
    }

    /// Return the expected resource generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Return the worker role.
    pub const fn role(&self) -> GpuBrokerRole {
        self.role
    }
}

impl fmt::Debug for GpuLaunchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuLaunchRequest")
            .field("role", &self.role)
            .field("generation", &self.generation)
            .field("device_class_count", &self.device_classes.len())
            .finish()
    }
}

/// Broker-side process observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuProcessObservation {
    /// One exact process matched.
    Matching,
    /// No exact process was found.
    Missing,
    /// Process identity was stale or reused.
    StaleIdentity,
    /// More than one process matched.
    Ambiguous,
}

/// Closed GPU broker failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBrokerError {
    /// A persisted identity is missing or stale.
    StaleIdentity,
    /// The role and device allowlist disagree.
    RoleDeviceMismatch,
    /// The caller's principal is not the signed worker principal.
    WrongPrincipal,
    /// The observed platform differs from the admitted platform.
    PlatformMismatch,
    /// The observed generation differs from the admitted generation.
    GenerationMismatch,
    /// Restart found more than one matching process.
    AmbiguousIdentity,
    /// A trusted runner plan does not match its GPU isolation profile.
    PlanShapeMismatch,
}

impl GpuBrokerError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::StaleIdentity => "gpu-device-identity-stale",
            Self::RoleDeviceMismatch => "gpu-role-device-mismatch",
            Self::WrongPrincipal => "gpu-process-principal-mismatch",
            Self::PlatformMismatch => "gpu-platform-mismatch",
            Self::GenerationMismatch => "gpu-device-generation-stale",
            Self::AmbiguousIdentity => "gpu-process-identity-ambiguous",
            Self::PlanShapeMismatch => "gpu-runner-shape-mismatch",
        }
    }
}

impl fmt::Display for GpuBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GpuBrokerError {}

/// Validate identity evidence before adopting a worker.
pub fn validate_observed_identity(
    request: &GpuLaunchRequest,
    observed_principal: GpuOpaqueIdentity,
    observed_platform: GpuOpaqueIdentity,
    observed_generation: u64,
    observations: &[GpuProcessObservation],
) -> Result<GpuProcessObservation, GpuBrokerError> {
    if observed_principal != request.principal {
        return Err(GpuBrokerError::WrongPrincipal);
    }
    if observed_platform != request.platform {
        return Err(GpuBrokerError::PlatformMismatch);
    }
    if observed_generation != request.generation {
        return Err(GpuBrokerError::GenerationMismatch);
    }
    let matches = observations
        .iter()
        .filter(|observation| **observation == GpuProcessObservation::Matching)
        .count();
    if matches > 1 {
        return Err(GpuBrokerError::AmbiguousIdentity);
    }
    Ok(observations
        .iter()
        .find(|observation| **observation == GpuProcessObservation::Matching)
        .copied()
        .unwrap_or(GpuProcessObservation::Missing))
}

/// Validate a resolved SpawnRunner plan against the closed GPU isolation
/// profile before the broker opens devices or clones a child.
pub fn validate_spawn_plan(
    plan: &SpawnRunnerPlan,
    pre_opened_device_fds: usize,
) -> Result<(), GpuBrokerError> {
    validate_spawn_plan_shape(plan, Some(pre_opened_device_fds))
}

/// Validate a GPU runner shape before the broker opens any device.
pub fn validate_spawn_plan_preflight(plan: &SpawnRunnerPlan) -> Result<(), GpuBrokerError> {
    validate_spawn_plan_shape(plan, None)
}

fn validate_spawn_plan_shape(
    plan: &SpawnRunnerPlan,
    pre_opened_device_fds: Option<usize>,
) -> Result<(), GpuBrokerError> {
    let Some(policy) = plan.seccomp_policy_ref.as_deref() else {
        return Ok(());
    };
    if !matches!(policy, "w1-gpu" | "w1-gpu-render-node" | "w1-video") {
        return Ok(());
    }
    if !plan.capabilities.is_empty() {
        return Err(GpuBrokerError::PlanShapeMismatch);
    }
    match policy {
        "w1-gpu" => {
            let required = ["/dev/kvm", "/dev/dri/renderD128", "/dev/udmabuf"];
            if !required.iter().all(|path| {
                plan.mount_policy
                    .device_binds
                    .iter()
                    .any(|bind| bind == path)
            }) {
                return Err(GpuBrokerError::PlanShapeMismatch);
            }
        }
        "w1-gpu-render-node" => {
            if !plan.namespaces.user
                || plan.user_namespace.is_none()
                || !plan.mount_policy.device_binds.is_empty()
                || pre_opened_device_fds.is_some_and(|count| count != 1)
            {
                return Err(GpuBrokerError::PlanShapeMismatch);
            }
        }
        "w1-video" => {
            if !plan.namespaces.pid
                || plan.user_namespace.is_some()
                || !plan
                    .mount_policy
                    .device_binds
                    .iter()
                    .any(|bind| bind == "/dev/dri/renderD128")
            {
                return Err(GpuBrokerError::PlanShapeMismatch);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
    use std::path::PathBuf;
    use crate::ops::spawn_runner::UserNamespaceSpec;

    fn identity(value: u8) -> GpuOpaqueIdentity {
        GpuOpaqueIdentity::from_core([value; 32])
    }

    #[test]
    fn render_node_allowlist_is_exact() {
        let request = GpuLaunchRequest::from_core(
            GpuBrokerRole::RenderNode,
            identity(1),
            identity(2),
            identity(3),
            1,
            vec![GpuDeviceClass::Dri],
        )
        .unwrap();
        assert_eq!(request.validate(), Ok(()));
        assert!(
            GpuLaunchRequest::from_core(
                GpuBrokerRole::RenderNode,
                identity(1),
                identity(2),
                identity(3),
                1,
                vec![GpuDeviceClass::Dri, GpuDeviceClass::Kvm],
            )
            .is_err()
        );
    }

    #[test]
    fn wrong_principal_and_ambiguous_identity_fail_closed() {
        let request = GpuLaunchRequest::from_core(
            GpuBrokerRole::Video,
            identity(1),
            identity(2),
            identity(3),
            4,
            vec![GpuDeviceClass::Dri],
        )
        .unwrap();
        assert_eq!(
            validate_observed_identity(
                &request,
                identity(4),
                identity(2),
                4,
                &[GpuProcessObservation::Matching],
            ),
            Err(GpuBrokerError::WrongPrincipal)
        );
        assert_eq!(
            validate_observed_identity(
                &request,
                identity(3),
                identity(2),
                4,
                &[
                    GpuProcessObservation::Matching,
                    GpuProcessObservation::Matching
                ],
            ),
            Err(GpuBrokerError::AmbiguousIdentity)
        );
        assert_eq!(
            validate_observed_identity(
                &request,
                identity(3),
                identity(2),
                4,
                &[
                    GpuProcessObservation::Missing,
                    GpuProcessObservation::Matching
                ],
            ),
            Ok(GpuProcessObservation::Matching)
        );
    }

    fn render_node_plan() -> SpawnRunnerPlan {
        SpawnRunnerPlan {
            binary_path: PathBuf::from("/bin/crosvm"),
            argv: vec!["crosvm".to_owned()],
            uid: 1000,
            gid: 1000,
            supplementary_groups: vec![],
            env: vec![],
            capabilities: vec![],
            namespaces: NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: true,
            },
            seccomp_policy_ref: Some("w1-gpu-render-node".to_owned()),
            mount_policy: MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![],
                nix_store_read_only: false,
                hide_device_nodes_by_default: false,
                device_binds: vec![],
                bind_mounts: vec![],
            },
            cgroup_placement: CgroupPlacement {
                subtree: String::new(),
                controllers: vec![],
                delegated: false,
            },
            user_namespace: Some(UserNamespaceSpec {
                host_uid_for_zero: 1000,
                host_gid_for_zero: 1000,
            }),
            umask: None,
        }
    }

    #[test]
    fn runner_shape_is_rejected_before_device_open() {
        let mut invalid = render_node_plan();
        invalid.namespaces.user = false;
        assert_eq!(
            validate_spawn_plan_preflight(&invalid),
            Err(GpuBrokerError::PlanShapeMismatch)
        );

        let valid = render_node_plan();
        assert_eq!(validate_spawn_plan_preflight(&valid), Ok(()));
        assert_eq!(validate_spawn_plan(&valid, 1), Ok(()));
    }
}
