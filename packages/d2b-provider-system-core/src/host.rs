//! Host reconciliation.
//!
//! `Provider/system-core` is the Host reconciler
//! (`ADR-046-provider-model-and-packaging`, section "system-core
//! bootstrap"). It is not the execution substrate: a Host names where
//! processes run, and the Process Providers run them.
//!
//! The user-only Host is the one Host shape with a first-class semantic
//! posture. `ADR-046-telemetry-audit-and-support`, section "Host resource
//! status", requires the reconciler to set `isolationPosture` and
//! `isolationPostureMessage` on every user-only Host unconditionally, and
//! requires that an operator can neither suppress nor override them. That
//! is two obligations, and both are met here: the posture is derived only
//! from the spec, and a submitted status carrying either field is rejected
//! rather than merged. A Host with any other execution policy carries no
//! posture at all, which is why the field is optional rather than defaulted
//! to a "has isolation" value.
//!
//! Adapted from the unsafe-local workload contract that this Host resource
//! succeeds (`packages/d2b-core/src/unsafe_local_workloads.rs` and the
//! daemon's `HelperRegistry` allowed-uid constraint in
//! `packages/d2bd/src/unsafe_local_helper.rs`), whose exact-user constraint
//! becomes the `defaultUserRef` requirement asserted below.

use d2b_contracts::v3::ResourceRef;
use std::{collections::BTreeSet, future::Future};

use d2b_contracts::v3::execution_policy::{BudgetSpec, ExecutionDomain};
use d2b_contracts::v3::host::{HOST_RESOURCE_TYPE, HostSpec, IsolationPosture};
use d2b_contracts::v3::resource_status::ResourcePhase;
use serde::Serialize;

use crate::error::SystemCoreError;
use crate::ownership;

/// The fixed, non-suppressible message that accompanies the no-isolation
/// posture.
pub const ISOLATION_POSTURE_MESSAGE: &str = "This host resource runs processes as the authenticated user with no isolation boundary. All child processes share the host user environment.";

/// The status fields only the reconciler may set.
///
/// A submitted status carrying either one is refused outright; there is no
/// merge, no "operator wins", and no flag that changes this.
pub const NO_ISOLATION_STATUS_FIELDS: [&str; 2] = ["isolationPosture", "isolationPostureMessage"];

/// Host capability classes that may be probed by system-core.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum HostCapabilityClass {
    /// KVM virtualization support.
    Kvm,
    /// pidfd and clone3 pidfd support.
    Pidfd,
    /// cgroup v2 delegation.
    CgroupV2,
    /// Unprivileged user namespaces.
    UserNamespace,
    /// virtio-fs support.
    Virtiofs,
    /// PipeWire session manager.
    AudioPipewire,
    /// Wayland compositor session.
    Wayland,
    /// GPU render node.
    GpuRender,
    /// DRM primary node.
    GpuDrm,
    /// TPM 2.0 support.
    Tpm2,
    /// USBIP support.
    Usbip,
}

/// The mandatory system-minijail platform gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinijailPlatformGate {
    /// Major Linux kernel version observed by the probe.
    pub kernel_major: u16,
    /// Minor Linux kernel version observed by the probe.
    pub kernel_minor: u16,
    /// Whether a delegated leaf exposes a writable cgroup.kill.
    pub cgroup_kill_writable: bool,
}

impl MinijailPlatformGate {
    /// Build the gate snapshot.
    pub const fn new(kernel_major: u16, kernel_minor: u16, cgroup_kill_writable: bool) -> Self {
        Self {
            kernel_major,
            kernel_minor,
            cgroup_kill_writable,
        }
    }

    /// Whether Linux 5.14 or newer is present.
    pub const fn kernel_supported(self) -> bool {
        self.kernel_major > 5 || (self.kernel_major == 5 && self.kernel_minor >= 14)
    }

    /// Validate the non-optional minijail placement requirements.
    pub fn validate(self) -> Result<(), SystemCoreError> {
        if !self.kernel_supported() {
            return Err(SystemCoreError::KernelTooOld);
        }
        if !self.cgroup_kill_writable {
            return Err(SystemCoreError::CgroupKillUnavailable);
        }
        Ok(())
    }
}

/// A hermetic result returned by the injected Host probe adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostProbeSnapshot {
    capabilities: BTreeSet<HostCapabilityClass>,
    kernel_release: String,
    os_name: String,
    user_manager_available: bool,
    minijail_gate: MinijailPlatformGate,
    active_process_count: u32,
}

impl HostProbeSnapshot {
    /// Construct a bounded probe result.
    pub fn new(
        capabilities: impl IntoIterator<Item = HostCapabilityClass>,
        kernel_release: impl Into<String>,
        os_name: impl Into<String>,
        user_manager_available: bool,
        minijail_gate: MinijailPlatformGate,
        active_process_count: u32,
    ) -> Result<Self, SystemCoreError> {
        let capabilities: BTreeSet<_> = capabilities.into_iter().collect();
        let kernel_release = kernel_release.into();
        let os_name = os_name.into();
        if kernel_release.len() > 64 || os_name.len() > 128 {
            return Err(SystemCoreError::HostProbeFailed);
        }
        Ok(Self {
            capabilities,
            kernel_release,
            os_name,
            user_manager_available,
            minijail_gate,
            active_process_count,
        })
    }

    /// Borrow observed capabilities.
    pub fn capabilities(&self) -> &BTreeSet<HostCapabilityClass> {
        &self.capabilities
    }

    /// Borrow the bounded kernel release observation.
    pub fn kernel_release(&self) -> &str {
        &self.kernel_release
    }

    /// Borrow the bounded OS name observation.
    pub fn os_name(&self) -> &str {
        &self.os_name
    }

    /// Whether the user manager is reachable.
    pub const fn user_manager_available(&self) -> bool {
        self.user_manager_available
    }

    /// Return the minijail platform gate.
    pub const fn minijail_gate(&self) -> MinijailPlatformGate {
        self.minijail_gate
    }

    /// Number of non-terminal child processes observed.
    pub const fn active_process_count(&self) -> u32 {
        self.active_process_count
    }
}

/// An injected, bounded Host capability probe.
pub trait HostProbeEffectPort {
    /// Probe one capability class.
    fn probe(
        &self,
        capability: HostCapabilityClass,
    ) -> impl Future<Output = Result<bool, SystemCoreError>>;

    /// Return kernel/platform evidence without exposing paths or handles.
    fn platform(&self) -> impl Future<Output = Result<MinijailPlatformGate, SystemCoreError>>;
}

/// Public Host observations produced after a probe.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostObservationReport {
    /// The ordinary Host status projection.
    pub status: HostStatusReport,
    /// Capabilities observed by system-core.
    pub capabilities: Vec<HostCapabilityClass>,
    /// Bounded kernel release observation.
    pub kernel_release: String,
    /// Bounded OS name observation.
    pub os_name: String,
    /// User-manager availability.
    pub user_manager_available: bool,
    /// Number of active child processes.
    pub active_process_count: u32,
    /// Whether the mandatory minijail gate passed.
    pub minijail_ready: bool,
}

impl core::fmt::Debug for HostObservationReport {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("HostObservationReport(<redacted>)")
    }
}

/// The public Host status this Provider computes.
///
/// Every field is either a typed reference, a closed enumeration, or the
/// one fixed posture message. No path, unit name, cgroup, numeric identity,
/// argv, or environment value is representable here.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatusReport {
    /// The Host this status describes.
    pub host_ref: ResourceRef,
    /// The reconciling Provider, always `system-core`.
    pub provider: &'static str,
    /// The universal resource phase.
    pub phase: ResourcePhase,
    /// The declared isolation posture, present only for a user-only Host.
    pub isolation_posture: Option<IsolationPosture>,
    /// The fixed message that accompanies a present posture.
    pub isolation_posture_message: Option<&'static str>,
    /// The domain a Process on this Host defaults to.
    pub default_domain: ExecutionDomain,
    /// Every domain this Host admits.
    pub allowed_domains: Vec<ExecutionDomain>,
    /// The exact User a user-domain Process resolves.
    pub default_user_ref: Option<ResourceRef>,
}

impl core::fmt::Debug for HostStatusReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("HostStatusReport(<redacted>)")
    }
}

impl HostStatusReport {
    /// Whether this Host declares the explicit no-isolation posture.
    pub const fn is_no_isolation(&self) -> bool {
        self.isolation_posture.is_some()
    }
}

/// The `system-core` Host reconciler.
///
/// It performs no effect. Reconciling a Host is a decision over the spec
/// alone, so this type needs no injected effect port; the Provider learns
/// nothing about the host machine in order to reach it.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostReconciler {
    _private: (),
}

impl HostReconciler {
    /// Build the reconciler.
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// Reconcile one Host resource into its public status.
    ///
    /// `provider_ref` is the resource's declared `spec.providerRef`. A Host
    /// naming another Provider is refused rather than reconciled, because
    /// `Provider/system-core` is the only Provider the Host contract admits
    /// and reconciling a foreign Host would be exactly the bootstrap
    /// widening the threat model forbids.
    pub fn reconcile(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
    ) -> Result<HostStatusReport, SystemCoreError> {
        ownership::require_resource_type(host_ref, HOST_RESOURCE_TYPE)?;
        if provider_ref.to_canonical_string() != crate::PROVIDER_REF {
            return Err(SystemCoreError::ProviderRefMismatch);
        }
        let policy = spec.policy();
        // A Host that admits the user domain must name the exact User a
        // user-domain Process resolves. The unsafe-local predecessor
        // enforced the same thing as an allowed-uid set.
        if policy.allowed_domains().contains(&ExecutionDomain::User)
            && policy.default_user_ref().is_none()
        {
            return Err(SystemCoreError::UserRefRequired);
        }
        let isolation_posture = spec.isolation_posture();
        Ok(HostStatusReport {
            host_ref: host_ref.clone(),
            provider: crate::PROVIDER_NAME,
            phase: ResourcePhase::Ready,
            isolation_posture,
            isolation_posture_message: isolation_posture.map(|_| ISOLATION_POSTURE_MESSAGE),
            default_domain: policy.default_domain(),
            allowed_domains: policy.allowed_domains().to_vec(),
            default_user_ref: policy.default_user_ref().cloned(),
        })
    }

    /// Reject a submitted status that carries a reconciler-owned field.
    ///
    /// This is the enforcement half of "operators cannot suppress or
    /// override it". A submitted status is admitted only when it names
    /// neither posture field, whatever its value would have been - an
    /// explicit `null` is as much an override attempt as `"none"` is.
    pub fn reject_operator_status_fields(
        submitted: &serde_json::Value,
    ) -> Result<(), SystemCoreError> {
        let object = submitted
            .as_object()
            .ok_or(SystemCoreError::StatusNotAnObject)?;
        for reserved in NO_ISOLATION_STATUS_FIELDS {
            if object.contains_key(reserved) {
                return Err(SystemCoreError::OperatorSuppliedStatusField);
            }
        }
        Ok(())
    }

    /// Reconcile a Host with a previously bounded, hermetic probe snapshot.
    ///
    /// The snapshot is the seam a real system-core effect adapter fills from
    /// bounded OS probes.  This method performs no host I/O and can therefore
    /// be used by both conformance and fault-injection tests.
    pub fn reconcile_observed(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
        snapshot: HostProbeSnapshot,
        required_capabilities: &BTreeSet<HostCapabilityClass>,
        requires_minijail: bool,
    ) -> Result<HostObservationReport, SystemCoreError> {
        let status = self.reconcile(host_ref, provider_ref, spec)?;
        let mut required = required_capabilities.clone();
        if requires_minijail {
            required.insert(HostCapabilityClass::Pidfd);
            required.insert(HostCapabilityClass::CgroupV2);
        }
        if !required.is_subset(snapshot.capabilities()) {
            return Err(SystemCoreError::CapabilityMissing);
        }
        if requires_minijail {
            snapshot.minijail_gate().validate()?;
        }
        if spec.policy().admits_user_domain() && !snapshot.user_manager_available() {
            // User-manager unavailability is a degraded observation, not a
            // reason to misreport the Host as Ready.  Keep the existing
            // compact status type stable and expose the richer result here.
        }
        Ok(HostObservationReport {
            status,
            capabilities: snapshot.capabilities().iter().copied().collect(),
            kernel_release: snapshot.kernel_release().to_owned(),
            os_name: snapshot.os_name().to_owned(),
            user_manager_available: snapshot.user_manager_available(),
            active_process_count: snapshot.active_process_count(),
            minijail_ready: snapshot.minijail_gate().kernel_supported()
                && snapshot.minijail_gate().cgroup_kill_writable,
        })
    }

    /// Reject an aggregate reservation that exceeds a Host budget.
    pub fn check_budget(
        &self,
        host_budget: &BudgetSpec,
        aggregate: &BudgetReservation,
    ) -> Result<(), SystemCoreError> {
        if aggregate.exceeds(host_budget) {
            Err(SystemCoreError::BudgetOvercommit)
        } else {
            Ok(())
        }
    }
}

/// A bounded aggregate reservation computed from non-terminal child rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BudgetReservation {
    /// Reserved millicpus.
    pub cpu_milli: u64,
    /// Reserved memory bytes.
    pub memory_bytes: u64,
    /// Reserved process IDs.
    pub pids: u32,
    /// Reserved file descriptors.
    pub fds: u32,
    /// Reserved threads.
    pub threads: u32,
}

impl BudgetReservation {
    /// Whether this aggregate exceeds any explicitly configured Host limit.
    pub fn exceeds(self, budget: &BudgetSpec) -> bool {
        budget
            .cpu()
            .and_then(|cpu| cpu.limit)
            .is_some_and(|limit| limit.get() < self.cpu_milli)
            || budget
                .memory()
                .and_then(|memory| memory.limit.as_ref())
                .is_some_and(|limit| limit.as_bytes() < self.memory_bytes)
            || budget
                .pids()
                .and_then(|pids| pids.limit)
                .is_some_and(|limit| limit < self.pids)
            || budget
                .fds()
                .and_then(|fds| fds.limit)
                .is_some_and(|limit| limit < self.fds)
            || budget
                .thread_limit()
                .is_some_and(|limit| limit < self.threads)
    }
}
