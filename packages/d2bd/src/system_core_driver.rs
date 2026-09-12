//! System-core resource driver (U12): the v3 `ResourceDriver` conversion of
//! the Host/User handler the shared Core Runner used to execute.
//!
//! The family owns exactly the two bootstrap ResourceTypes `system-core`
//! observes on the local machine: `Host` (the posture decision plus bounded
//! capability/platform/proc observations) and `User` (local NSS discovery).
//! Neither realizes anything on a target and neither owns child rows; the
//! driver is an observation surface. `recover` therefore adopts without
//! effects, `reconcile` publishes the typed in-memory status (R11), and
//! `delete` converges without effects (the manager already cascaded the
//! row's - empty - owned-child set).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`SystemCoreDriverFactory`] registration under `Host` and
//!   `User`.
//! - `validate_spec` -> [`ResourceDriver::validate`]: typed spec decode plus
//!   the `Host.spec.providerRef` fence (`Provider/system-core` is the only
//!   Provider the Host contract admits).
//! - `plan` -> the preserved `observedGeneration` short-circuit: a status
//!   observed at the current generation skips re-observing (the old
//!   `ResourceReconciler::plan` returned a converged plan in that case).
//! - `observe` -> [`ResourceDriver::recover`] (old `ObservationResult` was
//!   converged: nothing to adopt).
//! - `execute_effect` (old `status_candidate`) -> [`ResourceDriver::reconcile`]
//!   over the [`SystemCoreDriverEffects`] probe/discovery port.
//! - `finalize` -> [`ResourceDriver::delete`] (old `FinalizeResult` was
//!   converged: the family owns no children and carries no finalizer).
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
//!
//! Deliberately not carried from the old handler: the durable
//! `status.resource` JSON projection and its `lastReconciledAt` /
//! `observedGeneration` writes (status is runtime-only now, R11), and the
//! `assess_update` / `plan_upgrade` runner path (no KTD3 driver equivalent;
//! the family never planned an upgrade - recycle is the manager's delete
//! path). The old runner's 5s resync relisted and did nothing whenever the
//! status was current, so no periodic re-probe is reproduced.
//!
//! KTD13: the driver has no spawn surface at all. It observes the local host
//! directly through the effects port and owns no Process.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourcePhase, ResourceRef, ResourceSpec,
    host::{HOST_PROVIDER_REF, HOST_RESOURCE_TYPE, HostSpec},
    user::{USER_RESOURCE_TYPE, UserSpec},
};
use d2b_provider_system_core::{
    DiscoveredUser, HostCapabilityClass, HostObservationReport, HostProbeEffectPort,
    HostProbeMetadata, HostReconciler, MinijailPlatformGate, UserBinding, UserDiscoveryEffectPort,
    UserIdentityDigest, UserObservation, UserReconciler, UserStatusReport,
};
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemCoreDriverErrorKind {
    /// The durable spec did not decode as the closed Host/User contract, or
    /// names a Provider the Host contract does not admit.
    SpecInvalid,
    /// The bounded host probe failed transiently.
    HostObservation,
    /// Local NSS discovery failed transiently.
    UserDiscovery,
    /// Owned children are still retiring; the delete pass requeues.
    DrainPending,
}

impl SystemCoreDriverErrorKind {
    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::SYSTEM_CORE_SPEC_INVALID,
            Self::HostObservation => FailureKinds::SYSTEM_CORE_HOST_OBSERVATION_FAILED,
            Self::UserDiscovery => FailureKinds::SYSTEM_CORE_USER_DISCOVERY_FAILED,
            Self::DrainPending => FailureKinds::SYSTEM_CORE_DRAIN_PENDING,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct SystemCoreDriverError {
    kind: SystemCoreDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl SystemCoreDriverError {
    const fn new(kind: SystemCoreDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for SystemCoreDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            SystemCoreDriverErrorKind::SpecInvalid => "system-core-spec-invalid",
            SystemCoreDriverErrorKind::HostObservation => "system-core-host-observation-failed",
            SystemCoreDriverErrorKind::UserDiscovery => "system-core-user-discovery-failed",
            SystemCoreDriverErrorKind::DrainPending => "system-core-drain-pending",
        })
    }
}

impl std::error::Error for SystemCoreDriverError {}

/// Typed in-memory status projection (R11: never persisted).
///
/// The observation is kept with the generation it was taken at, which is the
/// runtime-only successor of the old durable
/// `status.observedGeneration` plan short-circuit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SystemCoreDriverStatus {
    /// The bounded Host observation (`HostReconciler` report).
    Host {
        observed_generation: u64,
        report: HostObservationReport,
    },
    /// The local User discovery report (`UserReconciler` report).
    User {
        observed_generation: u64,
        report: UserStatusReport,
    },
}

impl SystemCoreDriverStatus {
    /// The desired generation this observation was taken at.
    pub(crate) const fn observed_generation(&self) -> u64 {
        match self {
            Self::Host {
                observed_generation, ..
            }
            | Self::User {
                observed_generation, ..
            } => *observed_generation,
        }
    }
}

// ---------------------------------------------------------------------------
// Spec decode hook
// ---------------------------------------------------------------------------

/// The manager-wired decode hook for Host/User rows. The universal spec
/// layer carries `providerRef` / `updatePolicy`; the base keeps exactly the
/// typed Host/User contract fields, so the decoder hands the driver the
/// complete desired state (`ResourceSpec::base()` is what the old handler
/// decoded into `HostSpec`/`UserSpec`).
pub(crate) fn system_core_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<ResourceSpec>(bytes))
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The host-observation surface the system-core driver needs: the preserved
/// `system-core` Provider behavior (bounded capability/platform/metadata
/// probe, NSS discovery), behind the erased seam driver tests script (R4).
#[async_trait]
pub(crate) trait SystemCoreDriverEffects: Send + Sync + 'static {
    /// Observe one Host and compute its public status. The production
    /// implementation is the preserved `HostReconciler` probe with its
    /// degraded fallback; it never exposes a raw host handle.
    async fn observe_host(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
    ) -> Result<HostObservationReport, SystemCoreDriverError>;

    /// Discover one declared User and compute its public status (preserved
    /// `UserReconciler` over the local NSS adapter).
    async fn observe_user(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<UserStatusReport, SystemCoreDriverError>;
}

/// Production effects over the local host: the bounded probe adapter and the
/// NSS discovery adapter the old core runner wired.
pub(crate) struct ProductionSystemCoreDriverEffects;

#[async_trait]
impl SystemCoreDriverEffects for ProductionSystemCoreDriverEffects {
    async fn observe_host(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
    ) -> Result<HostObservationReport, SystemCoreDriverError> {
        let retryable = |op: DriverOp| {
            SystemCoreDriverError::new(SystemCoreDriverErrorKind::HostObservation, op)
        };
        match HostReconciler::new()
            .reconcile_with_probe(
                host_ref,
                provider_ref,
                spec,
                &SystemCoreHostProbe::current(),
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
                    .map_err(|error| {
                        retryable(DriverOp::Reconcile).with_detail(
                            FailureDetail::at("host/probe")
                                .comparison(FailureComparison::new(
                                    "host.probe",
                                    "completed",
                                    "failed",
                                ))
                                .with_note(format!("{probe_error}; {error}")),
                        )
                    })?;
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

    async fn observe_user(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<UserStatusReport, SystemCoreDriverError> {
        UserReconciler::new(SystemCoreUserDiscovery)
            .reconcile(user_ref, spec)
            .await
            .map_err(|error| {
                SystemCoreDriverError::new(
                    SystemCoreDriverErrorKind::UserDiscovery,
                    DriverOp::Reconcile,
                )
                .with_detail(
                    FailureDetail::at("user/discovery")
                        .comparison(FailureComparison::new(
                            "user.discovery",
                            "discovered",
                            "failed",
                        ))
                        .with_note(error.to_string()),
                )
            })
    }
}

// ---------------------------------------------------------------------------
// Production host probe / NSS discovery adapters (moved with the family)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct SystemCoreUserDiscovery;

#[derive(Debug, Clone, Copy)]
struct SystemCoreHostProbe {
    user_uid: u32,
}

impl SystemCoreHostProbe {
    fn current() -> Self {
        Self {
            user_uid: nix::unistd::Uid::current().as_raw(),
        }
    }

    fn kernel_release() -> Result<String, d2b_provider_system_core::SystemCoreError> {
        d2bd_runtime::resource_runtime_support::read_bounded("/proc/sys/kernel/osrelease", 64)
            .map(|release| release.trim().to_owned())
            .map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)
    }

    fn os_name() -> Result<String, d2b_provider_system_core::SystemCoreError> {
        let release =
            d2bd_runtime::resource_runtime_support::read_bounded("/etc/os-release", 16 * 1024)
                .map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)?;
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

    fn has_render_node() -> bool {
        std::fs::read_dir("/dev/dri")
            .map(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("renderD"))
                })
            })
            .unwrap_or(false)
    }

    fn has_primary_drm_node() -> bool {
        std::fs::read_dir("/dev/dri")
            .map(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("card"))
                })
            })
            .unwrap_or(false)
    }

    fn active_process_count() -> Result<u32, d2b_provider_system_core::SystemCoreError> {
        let mut count = 0_u32;
        for entry in std::fs::read_dir("/proc")
            .map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)?
        {
            let entry =
                entry.map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)?;
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

impl HostProbeEffectPort for SystemCoreHostProbe {
    async fn probe(
        &self,
        capability: HostCapabilityClass,
    ) -> Result<bool, d2b_provider_system_core::SystemCoreError> {
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
            HostCapabilityClass::GpuRender => Self::has_render_node(),
            HostCapabilityClass::GpuDrm => Self::has_primary_drm_node(),
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

    async fn platform(
        &self,
    ) -> Result<MinijailPlatformGate, d2b_provider_system_core::SystemCoreError> {
        let gate = crate::process_provider_runtime::detect_minijail_platform_gate();
        Ok(MinijailPlatformGate::new(
            gate.kernel_major,
            gate.kernel_minor,
            gate.cgroup_kill_available,
        ))
    }

    async fn metadata(
        &self,
    ) -> Result<HostProbeMetadata, d2b_provider_system_core::SystemCoreError> {
        Ok(HostProbeMetadata {
            kernel_release: Self::kernel_release()?,
            os_name: Self::os_name()?,
            user_manager_available: self.runtime_path("systemd").is_dir(),
            active_process_count: Self::active_process_count()?,
        })
    }
}

impl UserDiscoveryEffectPort for SystemCoreUserDiscovery {
    async fn discover(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<Option<DiscoveredUser>, d2b_provider_system_core::SystemCoreError> {
        discover_local_user(user_ref, spec).await
    }
}

async fn discover_local_user(
    user_ref: &ResourceRef,
    spec: &UserSpec,
) -> Result<Option<DiscoveredUser>, d2b_provider_system_core::SystemCoreError> {
    use nix::unistd::{Group, User};
    use sha2::{Digest, Sha256};

    let username = spec.os_username().as_str();
    let user = User::from_name(username)
        .map_err(|_| d2b_provider_system_core::SystemCoreError::DiscoveryUnavailable)?;
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
        .map_err(|_| d2b_provider_system_core::SystemCoreError::DiscoveryUnavailable)?
        .is_some()
    {
        verified.insert(UserBinding::PrimaryGroup);
    }

    let mut groups_verified = true;
    for group in spec.groups() {
        let Some(group_record) = Group::from_name(group.as_str())
            .map_err(|_| d2b_provider_system_core::SystemCoreError::DiscoveryUnavailable)?
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

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the `Host` and `User` resource types.
/// Construction is infallible by contract (R3): the production effects carry
/// no fallible setup.
pub(crate) struct SystemCoreDriverFactory {
    types: [ResourceTypeName; 2],
    effects: Arc<dyn SystemCoreDriverEffects>,
}

impl SystemCoreDriverFactory {
    pub(crate) fn new() -> Self {
        Self::with_effects(Arc::new(ProductionSystemCoreDriverEffects))
    }

    /// Construct over an injected port (driver tests; the production
    /// constructor above is the composition path).
    pub(crate) fn with_effects(effects: Arc<dyn SystemCoreDriverEffects>) -> Self {
        Self {
            types: [
                ResourceTypeName::new(HOST_RESOURCE_TYPE),
                ResourceTypeName::new(USER_RESOURCE_TYPE),
            ],
            effects,
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for SystemCoreDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(SystemCoreDriver {
            effects: Arc::clone(&self.effects),
        })
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One Host or User resource's driver.
pub(crate) struct SystemCoreDriver {
    effects: Arc<dyn SystemCoreDriverEffects>,
}

impl SystemCoreDriver {
    fn error(&self, kind: SystemCoreDriverErrorKind, op: DriverOp) -> SystemCoreDriverError {
        SystemCoreDriverError::new(kind, op)
    }

    fn resource_ref(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceRef, SystemCoreDriverError> {
        let type_name = d2b_contracts_resource::v3::ResourceTypeName::parse(
            ctx.key().type_name.clone(),
        )
        .map_err(|_| self.error(SystemCoreDriverErrorKind::SpecInvalid, op))?;
        let name = d2b_contracts_resource::v3::ResourceName::parse(ctx.key().name.clone())
            .map_err(|_| self.error(SystemCoreDriverErrorKind::SpecInvalid, op))?;
        Ok(ResourceRef::new(type_name, name))
    }

    fn decoded_spec<'a>(
        &self,
        ctx: &'a ResourceContext,
        op: DriverOp,
    ) -> Result<&'a ResourceSpec, SystemCoreDriverError> {
        ctx.spec::<ResourceSpec>()
            .map_err(|_| self.error(SystemCoreDriverErrorKind::SpecInvalid, op))
    }

    fn host_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<(ResourceRef, HostSpec), SystemCoreDriverError> {
        let envelope = self.decoded_spec(ctx, op)?;
        let provider_ref = envelope
            .provider_ref()
            .cloned()
            .filter(|provider| provider.to_canonical_string() == HOST_PROVIDER_REF)
            .ok_or_else(|| self.error(SystemCoreDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<HostSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| self.error(SystemCoreDriverErrorKind::SpecInvalid, op))?;
        Ok((provider_ref, spec))
    }

    fn user_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<UserSpec, SystemCoreDriverError> {
        let envelope = self.decoded_spec(ctx, op)?;
        serde_json::from_slice::<UserSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| self.error(SystemCoreDriverErrorKind::SpecInvalid, op))
    }

    /// The generation this actor's in-memory status was observed at, if the
    /// status belongs to this driver family and the same generation.
    fn observed_generation(&self, ctx: &ResourceContext) -> Option<u64> {
        ctx.status::<SystemCoreDriverStatus>()
            .map(SystemCoreDriverStatus::observed_generation)
    }
}

#[async_trait]
impl ResourceDriver for SystemCoreDriver {
    type Error = SystemCoreDriverError;

    fn classify_error(&self, error: &SystemCoreDriverError) -> DriverFailure {
        let failure = match error.kind {
            SystemCoreDriverErrorKind::SpecInvalid => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            SystemCoreDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            SystemCoreDriverErrorKind::HostObservation
            | SystemCoreDriverErrorKind::UserDiscovery => {
                DriverFailure::error(error.op, error.kind.failure_kind(), FailureClass::Retryable)
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Typed spec decode plus the Host Provider fence (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        match ctx.key().type_name.as_str() {
            HOST_RESOURCE_TYPE => {
                let (_provider_ref, _spec) = self.host_spec(ctx, DriverOp::Validate)?;
            }
            USER_RESOURCE_TYPE => {
                let _spec = self.user_spec(ctx, DriverOp::Validate)?;
            }
            _ => return Err(self.error(SystemCoreDriverErrorKind::SpecInvalid, DriverOp::Validate)),
        }
        Ok(())
    }

    /// The old `observe` was converged with nothing to adopt: the family
    /// realizes no target-local state and owns no child rows.
    async fn recover(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<RecoveryOutcome, Self::Error> {
        match ctx.key().type_name.as_str() {
            HOST_RESOURCE_TYPE => {
                self.host_spec(ctx, DriverOp::Recover)?;
            }
            USER_RESOURCE_TYPE => {
                self.user_spec(ctx, DriverOp::Recover)?;
            }
            _ => return Err(self.error(SystemCoreDriverErrorKind::SpecInvalid, DriverOp::Recover)),
        }
        Ok(RecoveryOutcome::Adopted)
    }

    /// Observe the local host through the effects port and publish the typed
    /// status (R11). The preserved `observedGeneration` short-circuit keeps
    /// one observation per desired generation; a degraded observation still
    /// converges (the old handler persisted the degraded projection and the
    /// runner classified it converged).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let generation = ctx.generation();
        if self.observed_generation(ctx) == Some(generation) {
            return Ok(ReconcileOutcome::Satisfied);
        }
        match ctx.key().type_name.as_str() {
            HOST_RESOURCE_TYPE => {
                let (provider_ref, spec) = self.host_spec(ctx, DriverOp::Reconcile)?;
                let host_ref = self.resource_ref(ctx, DriverOp::Reconcile)?;
                let report = self
                    .effects
                    .observe_host(&host_ref, &provider_ref, &spec)
                    .await?;
                ctx.set_status(SystemCoreDriverStatus::Host {
                    observed_generation: generation,
                    report,
                });
            }
            USER_RESOURCE_TYPE => {
                let spec = self.user_spec(ctx, DriverOp::Reconcile)?;
                let user_ref = self.resource_ref(ctx, DriverOp::Reconcile)?;
                let report = self.effects.observe_user(&user_ref, &spec).await?;
                ctx.set_status(SystemCoreDriverStatus::User {
                    observed_generation: generation,
                    report,
                });
            }
            _ => return Err(self.error(SystemCoreDriverErrorKind::SpecInvalid, DriverOp::Reconcile)),
        }
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The family owns no child in production, so
    /// this converges immediately; an owned row still live requeues the pass.
    /// Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(SystemCoreDriverErrorKind::DrainPending, DriverOp::Delete))?;
        Ok(())
    }

    /// The old `finalize` was converged: the family owns no children and
    /// carries no finalizer, so teardown is the manager's row removal (R10).
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a recording effects/manager double (R4; the
// no-spawn and one-observation-per-generation invariants are asserted
// through the recorded calls).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use d2b_contracts_resource::v3::{
        ResourcePhase, ResourceRef, ResourceSpec,
        execution_policy::to_base_object,
        host::{HOST_PROVIDER_REF, HostSpec},
        user::{OsUsername, UserSpec},
    };
    use d2b_provider_system_core::{
        HostCapabilityClass, HostObservationReport, HostReconciler, UserDiscoveryCondition,
        UserStatusReport,
    };
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext,
        WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, StoredDesiredResource,
    };
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use super::{
        SystemCoreDriverError, SystemCoreDriverFactory, SystemCoreDriverStatus,
        system_core_spec_decoder,
    };

    // -- fakes ---------------------------------------------------------------

    /// Scripted observation port: records every call order-preservingly and
    /// can fail User discovery.
    struct RecordingEffects {
        calls: parking_lot::Mutex<Vec<String>>,
        host_phase: parking_lot::Mutex<ResourcePhase>,
        user_phase: parking_lot::Mutex<ResourcePhase>,
        fail_user: AtomicBool,
    }

    impl RecordingEffects {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                host_phase: parking_lot::Mutex::new(ResourcePhase::Ready),
                user_phase: parking_lot::Mutex::new(ResourcePhase::Ready),
                fail_user: AtomicBool::new(false),
            })
        }

        fn call_order(&self) -> Vec<String> {
            self.calls.lock().clone()
        }

        fn set_host_phase(&self, phase: ResourcePhase) {
            *self.host_phase.lock() = phase;
        }
    }

    #[async_trait::async_trait]
    impl super::SystemCoreDriverEffects for RecordingEffects {
        async fn observe_host(
            &self,
            host_ref: &ResourceRef,
            provider_ref: &ResourceRef,
            spec: &HostSpec,
        ) -> Result<HostObservationReport, SystemCoreDriverError> {
            self.calls.lock().push("observe-host".to_owned());
            let mut status = HostReconciler::new()
                .reconcile(host_ref, provider_ref, spec)
                .expect("the scripted host spec is admitted");
            status.phase = *self.host_phase.lock();
            Ok(HostObservationReport {
                status,
                capabilities: vec![HostCapabilityClass::Kvm],
                kernel_release: "6.9.0-test".to_owned(),
                os_name: "Linux".to_owned(),
                user_manager_available: true,
                active_process_count: 3,
                minijail_ready: true,
            })
        }

        async fn observe_user(
            &self,
            user_ref: &ResourceRef,
            _spec: &UserSpec,
        ) -> Result<UserStatusReport, SystemCoreDriverError> {
            self.calls.lock().push("observe-user".to_owned());
            if self.fail_user.load(Ordering::SeqCst) {
                return Err(SystemCoreDriverError::new(
                    super::SystemCoreDriverErrorKind::UserDiscovery,
                    d2b_resource_runtime::error::DriverOp::Reconcile,
                ));
            }
            Ok(UserStatusReport {
                user_ref: user_ref.clone(),
                provider: "system-core",
                phase: *self.user_phase.lock(),
                discovery: UserDiscoveryCondition::Discovered,
                identity: None,
            })
        }
    }

    /// Recording manager: the family must never mutate children or
    /// registers; any manager call fails the test loudly through the
    /// recorded call list.
    struct RecordingManager {
        calls: parking_lot::Mutex<Vec<&'static str>>,
        owned: parking_lot::Mutex<Vec<StoredDesiredResource>>,
    }

    impl RecordingManager {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                owned: parking_lot::Mutex::new(Vec::new()),
            })
        }

        /// Seed one owned child row (the finalize gate's input).
        fn seed_owned(&self, key: ResourceKey) {
            self.owned.lock().push(StoredDesiredResource {
                key,
                uid: [0x77; 16],
                generation: 1,
                owner_uid: Some([0x42; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: Vec::new(),
                metadata: Vec::new(),
                created_at: 0,
            });
        }

        fn call_order(&self) -> Vec<&'static str> {
            self.calls.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.calls.lock().push("ensure-child");
            Err(ResourceError::ManagerRpc("unexpected ensure_child".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            self.calls.lock().push("get");
            Ok(None)
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            self.calls.lock().push("view");
            Err(ResourceError::ManagerRpc("unexpected view".into()))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().push("delete");
            let mut owned = self.owned.lock();
            if owned.iter().any(|row| row.key == *key) {
                owned.retain(|row| row.key != *key);
                Ok(())
            } else {
                Err(ResourceError::ManagerRpc("unexpected delete".into()))
            }
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.calls.lock().push("list-owned");
            Ok(self.owned.lock().clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            self.calls.lock().push("register-watch");
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            self.calls.lock().push("cancel-watch");
            Ok(())
        }
    }

    struct RecordingRequeue {
        calls: parking_lot::Mutex<Vec<u64>>,
    }

    impl RecordingRequeue {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn call_count(&self) -> usize {
            self.calls.lock().len()
        }
    }

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, after: std::time::Duration) -> RequeueId {
            self.calls.lock().push(after.as_millis() as u64);
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    fn host_spec_bytes(provider_ref: Option<&str>) -> Vec<u8> {
        let base = to_base_object(&HostSpec::system_default()).expect("host base");
        let provider = provider_ref.map(|reference| ResourceRef::parse(reference).expect("ref"));
        ResourceSpec::new(provider, None, base, None)
            .expect("admitted resource spec")
            .canonical_bytes()
            .expect("canonical spec bytes")
    }

    fn user_spec_bytes() -> Vec<u8> {
        let spec = UserSpec::minimal(OsUsername::parse("alice").expect("username"));
        let base = to_base_object(&spec).expect("user base");
        ResourceSpec::new(None, None, base, None)
            .expect("admitted resource spec")
            .canonical_bytes()
            .expect("canonical spec bytes")
    }

    fn row(type_name: &str, name: &str, spec: Vec<u8>) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", type_name, name),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec,
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    fn fixture(
        row: StoredDesiredResource,
        manager: Arc<RecordingManager>,
        requeue: Arc<RecordingRequeue>,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            TargetHandle::Host,
            system_core_spec_decoder(),
            manager,
            requeue,
            effects_tx,
            notify_tx,
        )
    }

    async fn build_driver(
        type_name: &str,
        name: &str,
        effects: Arc<RecordingEffects>,
    ) -> Box<dyn DynResourceDriver> {
        SystemCoreDriverFactory::with_effects(effects)
            .create(&ResourceKey::new("work", type_name, name))
            .await
    }

    async fn host_fixture() -> (
        ResourceContext,
        Arc<RecordingEffects>,
        Arc<RecordingManager>,
        Arc<RecordingRequeue>,
        Box<dyn DynResourceDriver>,
    ) {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        let requeue = RecordingRequeue::new();
        let ctx = fixture(
            row("Host", "host-system", host_spec_bytes(Some(HOST_PROVIDER_REF))),
            Arc::clone(&manager),
            Arc::clone(&requeue),
        );
        let driver = build_driver("Host", "host-system", Arc::clone(&effects)).await;
        (ctx, effects, manager, requeue, driver)
    }

    async fn user_fixture() -> (
        ResourceContext,
        Arc<RecordingEffects>,
        Arc<RecordingManager>,
        Box<dyn DynResourceDriver>,
    ) {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        let requeue = RecordingRequeue::new();
        let ctx = fixture(user_row(), Arc::clone(&manager), requeue);
        let driver = build_driver("User", "alice", Arc::clone(&effects)).await;
        (ctx, effects, manager, driver)
    }

    fn user_row() -> StoredDesiredResource {
        row("User", "alice", user_spec_bytes())
    }

    // -- factory -------------------------------------------------------------

    #[tokio::test]
    async fn factory_registers_exactly_host_and_user() {
        let factory = SystemCoreDriverFactory::new();
        assert_eq!(factory.resource_types().len(), 2);
        assert_eq!(factory.resource_types()[0].as_str(), "Host");
        assert_eq!(factory.resource_types()[1].as_str(), "User");
        factory
            .create(&ResourceKey::new("work", "Host", "host-system"))
            .await;
        factory
            .create(&ResourceKey::new("work", "User", "alice"))
            .await;
    }

    // -- validate ------------------------------------------------------------

    #[tokio::test]
    async fn validate_accepts_the_bootstrap_host_and_user_rows() {
        let (mut host_ctx, _effects, _manager, _requeue, mut host_driver) = host_fixture().await;
        host_driver
            .validate(&mut host_ctx)
            .await
            .expect("bootstrap Host validates");

        let (mut user_ctx, _effects, _manager, mut user_driver) = user_fixture().await;
        user_driver
            .validate(&mut user_ctx)
            .await
            .expect("User spec validates without a provider ref (the old exact-fixture shape)");
    }

    #[tokio::test]
    async fn validate_rejects_a_host_with_a_foreign_provider() {
        let effects = RecordingEffects::new();
        let mut ctx = fixture(
            row(
                "Host",
                "host-system",
                host_spec_bytes(Some("Provider/network-local")),
            ),
            RecordingManager::new(),
            RecordingRequeue::new(),
        );
        let mut driver = SystemCoreDriverFactory::with_effects(effects)
            .create(&ResourceKey::new("work", "Host", "host-system"))
            .await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    #[tokio::test]
    async fn validate_rejects_a_host_without_a_provider_ref() {
        let effects = RecordingEffects::new();
        let mut ctx = fixture(
            row("Host", "host-system", host_spec_bytes(None)),
            RecordingManager::new(),
            RecordingRequeue::new(),
        );
        let mut driver = SystemCoreDriverFactory::with_effects(effects)
            .create(&ResourceKey::new("work", "Host", "host-system"))
            .await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    #[tokio::test]
    async fn validate_rejects_a_malformed_user_spec() {
        let effects = RecordingEffects::new();
        let mut ctx = fixture(
            row("User", "alice", br#"{"nonsense":true}"#.to_vec()),
            RecordingManager::new(),
            RecordingRequeue::new(),
        );
        let mut driver = SystemCoreDriverFactory::with_effects(effects)
            .create(&ResourceKey::new("work", "User", "alice"))
            .await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    // -- recover -------------------------------------------------------------

    #[tokio::test]
    async fn recover_adopts_without_touching_the_target() {
        let (mut ctx, effects, _manager, _requeue, mut driver) = host_fixture().await;
        assert_eq!(
            driver.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
        assert!(effects.call_order().is_empty());
        assert!(ctx.status::<SystemCoreDriverStatus>().is_none());
    }

    // -- reconcile -----------------------------------------------------------

    #[tokio::test]
    async fn reconcile_observes_once_per_desired_generation() {
        let (mut ctx, effects, manager, _requeue, mut driver) = host_fixture().await;
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(effects.call_order(), vec!["observe-host".to_owned()]);
        let status = ctx
            .status::<SystemCoreDriverStatus>()
            .expect("status published");
        assert_eq!(status.observed_generation(), 1);
        let SystemCoreDriverStatus::Host { report, .. } = status else {
            panic!("expected host status, got {status:?}");
        };
        assert_eq!(
            report.status.phase,
            ResourcePhase::Ready,
            "the published projection is the typed Host report"
        );

        // The preserved plan short-circuit: a status observed at the current
        // generation does not re-probe (the old runner's 5s relist was a
        // no-op in exactly this state).
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("second reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            effects.call_order(),
            vec!["observe-host".to_owned()],
            "one observation per desired generation"
        );
        assert!(manager.call_order().is_empty());
    }

    #[tokio::test]
    async fn reconcile_publishes_a_degraded_host_observation() {
        let (mut ctx, effects, _manager, _requeue, mut driver) = host_fixture().await;
        effects.set_host_phase(ResourcePhase::Degraded);
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied,
            "a degraded observation still converges, exactly as the old handler did"
        );
        let SystemCoreDriverStatus::Host { report, .. } =
            ctx.status::<SystemCoreDriverStatus>().expect("status")
        else {
            panic!("expected host status");
        };
        assert_eq!(report.status.phase, ResourcePhase::Degraded);
        assert_eq!(report.capabilities, vec![HostCapabilityClass::Kvm]);
        assert!(report.minijail_ready);
    }

    #[tokio::test]
    async fn reconcile_publishes_the_user_discovery_projection() {
        let (mut ctx, effects, _manager, mut driver) = user_fixture().await;
        *effects.user_phase.lock() = ResourcePhase::Pending;
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        let SystemCoreDriverStatus::User {
            observed_generation,
            report,
        } = ctx.status::<SystemCoreDriverStatus>().expect("status")
        else {
            panic!("expected user status");
        };
        assert_eq!(*observed_generation, 1);
        assert_eq!(report.phase, ResourcePhase::Pending);
        assert_eq!(report.discovery, UserDiscoveryCondition::Discovered);
    }

    #[tokio::test]
    async fn reconcile_maps_a_discovery_failure_to_a_retryable_failure() {
        let (mut ctx, effects, _manager, mut driver) = user_fixture().await;
        effects.fail_user.store(true, Ordering::SeqCst);
        let failure = driver.reconcile(&mut ctx).await.expect_err("retryable");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(ctx.status::<SystemCoreDriverStatus>().is_none());
    }

    // -- finalize: owned children retire before the delete no-op (F3) ---------

    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_delete_noop() {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        manager.seed_owned(ResourceKey::new("work", "Process", "system-core-child"));
        let requeue = RecordingRequeue::new();
        let mut ctx = fixture(
            row("Host", "host-system", host_spec_bytes(Some(HOST_PROVIDER_REF))),
            Arc::clone(&manager),
            requeue,
        );
        let mut d = build_driver("Host", "host-system", effects).await;

        // A live owned child: the pass requeues instead of converging.
        let failure = d.finalize(&mut ctx).await.expect_err("owned child still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(
            manager.call_order().contains(&"delete"),
            "the owned child is nudged through its own finalize-before-delete pass"
        );

        // The manager removed the retired child row: the same pass converges.
        d.finalize(&mut ctx).await.expect("converged once the child retired");
    }

    // -- delete --------------------------------------------------------------

    #[tokio::test]
    async fn delete_converges_without_effects_or_child_mutation() {
        let (mut ctx, effects, manager, _requeue, mut driver) = host_fixture().await;
        driver.delete(&mut ctx).await.expect("delete");
        assert!(effects.call_order().is_empty());
        assert!(manager.call_order().is_empty());
    }

    // -- no-spawn surface (KTD13) --------------------------------------------

    #[tokio::test]
    async fn driver_operations_stay_off_every_spawn_surface() {
        let (mut ctx, _effects, manager, requeue, mut driver) = host_fixture().await;
        driver.validate(&mut ctx).await.expect("validate");
        driver.recover(&mut ctx).await.expect("recover");
        driver.reconcile(&mut ctx).await.expect("reconcile");
        driver.delete(&mut ctx).await.expect("delete");
        assert!(
            manager.call_order().is_empty(),
            "the family owns no children and mutates nothing through the manager: {:?}",
            manager.call_order()
        );
        assert_eq!(
            requeue.call_count(),
            0,
            "no self-requeue: the old runner's 5s relist never re-observed a current status"
        );
    }

    // -- production probe (moved with the family) ----------------------------

    #[tokio::test]
    async fn production_system_core_probe_returns_bounded_host_observations() {
        use d2b_provider_system_core::HostProbeEffectPort;

        let probe = super::SystemCoreHostProbe::current();
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
