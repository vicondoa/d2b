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
use d2b_contracts::v3::execution_policy::ExecutionDomain;
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
}
