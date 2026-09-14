//! What the pre-v3 plane still reads from the Process family.
//!
//! The Process/EphemeralProcess reconciliation itself lives in the family's
//! own crate, together with the one canonical launch-identity resolver every
//! ticket consumer shares. What stays here is exactly what other pre-v3
//! surfaces consume: the stable runtime failures the controller-session
//! mapping matches, and the restart annotation the interaction composition's
//! Process specs still carry.


pub(crate) const PROCESS_RESTART_ANNOTATION: &str = "d2b.d2bus.org/restart-generation";

/// Stable failures for the daemon-owned generic process path.
///
/// The classification surface stays intact for the controller-session
/// mapping (`resource_runtime::map_process_runtime_error` matches every
/// variant); U14 retired the durable list readers that constructed it, so the
/// variants are mapped to the daemon's runtime errors only.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessResourceRuntimeError {
    /// A durable resource did not decode as the closed Process contract.
    InvalidResource,
    /// The resource selected a Provider not owned by this runtime.
    UnsupportedProvider,
    /// The trusted bundle did not contain the requested template binding.
    TemplateUnavailable,
    /// A process identity was ambiguous during adoption or stop.
    IdentityAmbiguous,
    /// A Provider effect failed.
    ProviderEffect,
    /// A Provider controller bootstrap endpoint did not become readable.
    ControllerBootstrapUnavailable,
    /// A required Process Provider has no committed identity projection.
    ProviderIdentityUnavailable,
    /// A semantic Process owner has no committed identity projection.
    OwnerIdentityUnavailable,
    /// The durable store could not be listed or watched.
    Store,
}

impl core::fmt::Display for ProcessResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "process-resource-invalid",
            Self::UnsupportedProvider => "process-resource-provider-unsupported",
            Self::TemplateUnavailable => "process-resource-template-unavailable",
            Self::IdentityAmbiguous => "process-resource-identity-ambiguous",
            Self::ProviderEffect => "process-resource-provider-effect-failed",
            Self::ControllerBootstrapUnavailable => {
                "process-resource-controller-bootstrap-unavailable"
            }
            Self::ProviderIdentityUnavailable => "process-resource-provider-identity-unavailable",
            Self::OwnerIdentityUnavailable => "process-resource-owner-identity-unavailable",
            Self::Store => "process-resource-store-failed",
        })
    }
}

impl std::error::Error for ProcessResourceRuntimeError {}

