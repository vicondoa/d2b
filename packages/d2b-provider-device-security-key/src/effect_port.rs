//! Core-injected inventory observation port for physical security keys.

use core::fmt;
use std::future::Future;

/// Opaque Device identity issued by Core.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    /// Mint at the Core boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeviceId([redacted])")
    }
}

/// Opaque observation-policy identity issued by Core.
#[derive(Clone, PartialEq, Eq)]
pub struct ObservationPolicyId([u8; 32]);

impl ObservationPolicyId {
    /// Mint at the Core boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for ObservationPolicyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ObservationPolicyId([redacted])")
    }
}

/// Redacted physical inventory observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryObservation {
    /// Whether the exact admitted Device is present.
    pub present: bool,
    /// Whether Core revalidated the FIDO usage page.
    pub fido_confirmed: bool,
}

/// Closed inventory effect failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryEffectError {
    /// Core could not observe the exact Device.
    Unavailable,
    /// The observation policy is no longer valid.
    PolicyRejected,
    /// A retry may safely be attempted.
    Transient,
}

impl InventoryEffectError {
    /// Stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "security-key-inventory-unavailable",
            Self::PolicyRejected => "security-key-inventory-policy-rejected",
            Self::Transient => "security-key-inventory-transient",
        }
    }
}

impl fmt::Display for InventoryEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for InventoryEffectError {}

/// Provider-side observation boundary. No path, selector, UID, or broker
/// request is representable.
pub trait SecurityKeyInventoryEffectPort: Send + Sync {
    /// Observe the exact Core-admitted Device.
    fn observe_inventory(
        &self,
        device_id: &DeviceId,
        policy_id: &ObservationPolicyId,
    ) -> impl Future<Output = Result<InventoryObservation, InventoryEffectError>> + Send;
}
