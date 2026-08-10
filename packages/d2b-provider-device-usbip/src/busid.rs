//! USB bus-ID and ownership-marker contracts.

use core::fmt;
use d2b_contracts::usbip::{BusIdError, SYSFS_BUS_ID_MAX, validate_bus_id};

/// A validated sysfs USB bus ID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BusId(String);

impl BusId {
    /// Parse the shared canonical USB bus-ID grammar.
    pub fn parse(value: impl Into<String>) -> Result<Self, BusIdError> {
        let value = value.into();
        validate_bus_id(&value)?;
        Ok(Self(value))
    }

    /// Borrow the validated bus ID for the Core adapter.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BusId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BusId(<redacted>)")
    }
}

/// Maximum printable USB bus-ID bytes.
pub const MAX_BUS_ID_BYTES: usize = SYSFS_BUS_ID_MAX;

/// Opaque Core-derived physical USB backing identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalUsbBackingToken([u8; 32]);

impl PhysicalUsbBackingToken {
    /// Construct a token at the Core authority adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the token for an exact cross-Provider comparison.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for PhysicalUsbBackingToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PhysicalUsbBackingToken(<redacted>)")
    }
}

/// Opaque ownership marker for one exact USBIP projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallOwnershipMarker {
    projection: [u8; 32],
}

impl FirewallOwnershipMarker {
    /// Construct a marker from the Core-resolved projection digest.
    pub const fn from_core(projection: [u8; 32]) -> Self {
        Self { projection }
    }

    /// Borrow the marker digest for ownership confirmation.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.projection
    }

    /// Return the fixed marker namespace used by the broker.
    pub const fn namespace() -> &'static str {
        "d2b managed: usbip:"
    }
}

impl fmt::Debug for FirewallOwnershipMarker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FirewallOwnershipMarker(<redacted>)")
    }
}
