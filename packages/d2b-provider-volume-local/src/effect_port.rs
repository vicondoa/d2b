//! Provider-side opaque bindings for Volume state effects.
//!
//! No type in this module carries a host path, numeric identity, descriptor,
//! command, or broker operation. The core adapter resolves opaque IDs and
//! routes named view descriptors out-of-band to the target supervisor.

use std::fmt;
use std::future::Future;

use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::{MarkerStatus, ResourceUid};

use crate::marker::MarkerError;

/// A stable Provider-side Volume effect identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VolumeEffectId(ResourceUid);

impl VolumeEffectId {
    /// Derive an opaque effect identity from the immutable Volume UID.
    pub const fn from_resource_uid(volume_uid: ResourceUid) -> Self {
        Self(volume_uid)
    }

    /// Borrow the UID for serialization into the neutral effect contract.
    pub const fn resource_uid(&self) -> &ResourceUid {
        &self.0
    }
}

impl fmt::Debug for VolumeEffectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VolumeEffectId(<redacted>)")
    }
}

/// One declared named view.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamedViewId(BoundedToken);

impl NamedViewId {
    /// Parse a bounded view name.
    pub fn parse(value: impl Into<String>) -> Result<Self, VolumeEffectError> {
        BoundedToken::parse(value)
            .map(Self)
            .map_err(|_| VolumeEffectError::InvalidRequest)
    }
}

impl fmt::Debug for NamedViewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NamedViewId(<redacted>)")
    }
}

/// The execution domain in which a volume-local controller runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionDomain {
    /// One Host domain.
    Host(BoundedToken),
    /// One Guest-local domain.
    Guest(BoundedToken),
}

/// Opaque result of provisioning one Volume root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionDisposition {
    /// A root and external marker were created.
    Created,
    /// Existing state and marker were verified without replacement.
    Verified,
}

/// Marker verification result with no path or marker bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerObservation {
    /// Public marker status.
    pub status: MarkerStatus,
    /// Stable failure, when status is not verified.
    pub error: Option<MarkerError>,
}

/// Closed error set for the Provider-side effect seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeEffectError {
    /// A request violated a closed semantic bound.
    InvalidRequest,
    /// A Guest-local controller was asked to observe another domain.
    DomainMismatch,
    /// The adapter could not reserve the declared quota.
    QuotaInsufficient,
    /// The Volume is already at or above its soft quota.
    QuotaExceeded,
    /// Marker verification failed closed.
    MarkerFailed,
    /// The core adapter failed without exposing backend detail.
    BackendUnavailable,
}

impl VolumeEffectError {
    /// Return the stable redacted code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "volume-effect-request-invalid",
            Self::DomainMismatch => "volume-domain-mismatch",
            Self::QuotaInsufficient => "quota-insufficient",
            Self::QuotaExceeded => "volume-quota-exceeded",
            Self::MarkerFailed => "volume-marker-verification-failed",
            Self::BackendUnavailable => "volume-effect-backend-unavailable",
        }
    }
}

impl fmt::Display for VolumeEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for VolumeEffectError {}

/// Ensure this Provider instance can act on the source execution domain.
pub fn validate_domain(
    controller: &ExecutionDomain,
    source: &ExecutionDomain,
) -> Result<(), VolumeEffectError> {
    if controller == source {
        Ok(())
    } else {
        Err(VolumeEffectError::DomainMismatch)
    }
}

/// Provider-side state effect operations.
///
/// A production implementation is the fixed core adapter. Methods return only
/// opaque dispositions and named-view authorizations. No method returns a raw
/// filesystem descriptor to Provider code.
pub trait VolumeStateEffectPort: Send + Sync {
    /// Provision or verify one anchored Volume root and external marker.
    fn provision_volume(
        &self,
        volume: &VolumeEffectId,
        domain: &ExecutionDomain,
        quota_bytes: Option<u64>,
    ) -> impl Future<Output = Result<ProvisionDisposition, VolumeEffectError>> + Send;

    /// Verify the external marker before restart adoption or Process launch.
    fn verify_marker(
        &self,
        volume: &VolumeEffectId,
    ) -> impl Future<Output = Result<MarkerObservation, VolumeEffectError>> + Send;

    /// Ask core to route one named view descriptor to a target supervisor.
    fn route_named_view(
        &self,
        volume: &VolumeEffectId,
        view: &NamedViewId,
        target_domain: &ExecutionDomain,
    ) -> impl Future<Output = Result<NamedViewAuthorization, VolumeEffectError>> + Send;

    /// Remove the marker and root after ordered leaf-first cleanup.
    fn destroy_volume(
        &self,
        volume: &VolumeEffectId,
    ) -> impl Future<Output = Result<(), VolumeEffectError>> + Send;
}

/// Opaque proof that core authorized and routed one named view.
pub struct NamedViewAuthorization {
    _private: (),
}

impl NamedViewAuthorization {
    /// Issue an authorization after core has routed the descriptor.
    pub const fn routed() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for NamedViewAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NamedViewAuthorization(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).unwrap()
    }

    #[test]
    fn guest_local_domain_rejects_host_and_other_guest_sources() {
        let local = ExecutionDomain::Guest(token("work-vm"));
        assert!(validate_domain(&local, &ExecutionDomain::Guest(token("work-vm"))).is_ok());
        assert_eq!(
            validate_domain(&local, &ExecutionDomain::Host(token("host-system"))),
            Err(VolumeEffectError::DomainMismatch)
        );
        assert_eq!(
            validate_domain(&local, &ExecutionDomain::Guest(token("personal-vm"))),
            Err(VolumeEffectError::DomainMismatch)
        );
    }
}
