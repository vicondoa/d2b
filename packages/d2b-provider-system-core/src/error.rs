//! Typed, value-free `system-core` errors.
//!
//! Every variant is a closed classification. No message carries a resource
//! name, ResourceType string, OS username, path, or any other
//! caller-supplied value, so an error can be logged or audited verbatim.

use std::fmt;

/// One `system-core` reconciliation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SystemCoreError {
    /// The reference names a ResourceType this Provider does not own.
    ResourceTypeNotOwned,
    /// The resource declares a Provider other than `Provider/system-core`.
    ProviderRefMismatch,
    /// The submitted status carried a field only the reconciler may set.
    OperatorSuppliedStatusField,
    /// The submitted status was not a JSON object.
    StatusNotAnObject,
    /// A user-domain Host declared no exact `defaultUserRef`.
    UserRefRequired,
    /// The effect port reported an unusable local discovery result.
    DiscoveryUnavailable,
    /// A bounded host probe returned an invalid observation.
    HostProbeFailed,
    /// A required host capability was not observed.
    CapabilityMissing,
    /// The kernel is below the mandatory system-minijail floor.
    KernelTooOld,
    /// The delegated cgroup leaf has no writable cgroup.kill.
    CgroupKillUnavailable,
    /// Child process reservations exceed the Host aggregate budget.
    BudgetOvercommit,
}

impl fmt::Display for SystemCoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::ResourceTypeNotOwned => "resource type not owned by this provider",
            Self::ProviderRefMismatch => "provider reference mismatch",
            Self::OperatorSuppliedStatusField => "reconciler-owned status field was supplied",
            Self::StatusNotAnObject => "status is not an object",
            Self::UserRefRequired => "an exact user reference is required",
            Self::DiscoveryUnavailable => "local user discovery is unavailable",
            Self::HostProbeFailed => "host capability probe failed",
            Self::CapabilityMissing => "required host capability is absent",
            Self::KernelTooOld => "kernel is below the process-provider floor",
            Self::CgroupKillUnavailable => "delegated cgroup.kill is unavailable",
            Self::BudgetOvercommit => "host process budget is overcommitted",
        };
        f.write_str(text)
    }
}

impl std::error::Error for SystemCoreError {}
