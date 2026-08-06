//! Core-derived physical USB authority and hidraw effect boundary.

use core::fmt;
use d2b_contracts::v3::ResourceUid;

/// Core-derived opaque physical USB backing identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalUsbBackingToken([u8; 32]);

impl PhysicalUsbBackingToken {
    /// Construct a token at Core's trusted physical-device authority index.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the token for equality with another USB Provider claim.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for PhysicalUsbBackingToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PhysicalUsbBackingToken(<redacted>)")
    }
}

/// The exact authority tuple shared with every USB Provider.
#[derive(Clone, PartialEq, Eq)]
pub struct PhysicalUsbBackingClaim {
    /// Host authority scope, fixed by the Device contract.
    pub authority_scope: &'static str,
    /// Physical backing class, fixed by the Device contract.
    pub backing_class: &'static str,
    token: PhysicalUsbBackingToken,
}

impl PhysicalUsbBackingClaim {
    /// Construct the Host physical-device authority tuple.
    pub const fn from_core(token: PhysicalUsbBackingToken) -> Self {
        Self {
            authority_scope: "Host",
            backing_class: "physical-usb-backing",
            token,
        }
    }

    /// Borrow the Core-derived opaque key.
    pub const fn token(&self) -> &PhysicalUsbBackingToken {
        &self.token
    }
}

impl fmt::Debug for PhysicalUsbBackingClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhysicalUsbBackingClaim")
            .field("authority_scope", &self.authority_scope)
            .field("backing_class", &self.backing_class)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Opaque hidraw open request. No path, bus ID, selector string, or fd is
/// represented here.
#[derive(Clone, PartialEq, Eq)]
pub struct SecurityKeyOpenIntent {
    device_uid: ResourceUid,
    session_id: super::SecurityKeySessionId,
    backing: PhysicalUsbBackingClaim,
}

impl SecurityKeyOpenIntent {
    /// Construct a request after Core has admitted the physical authority.
    pub const fn from_core(
        device_uid: ResourceUid,
        session_id: super::SecurityKeySessionId,
        backing: PhysicalUsbBackingClaim,
    ) -> Self {
        Self {
            device_uid,
            session_id,
            backing,
        }
    }

    /// Borrow the opaque Device UID.
    pub const fn device_uid(&self) -> &ResourceUid {
        &self.device_uid
    }

    /// Borrow the opaque session ID.
    pub const fn session_id(&self) -> &super::SecurityKeySessionId {
        &self.session_id
    }

    /// Borrow the exact physical backing tuple.
    pub const fn backing(&self) -> &PhysicalUsbBackingClaim {
        &self.backing
    }
}

impl fmt::Debug for SecurityKeyOpenIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecurityKeyOpenIntent(<redacted>)")
    }
}

/// Closed effect failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyEffectError {
    /// A second authority owns the physical backing.
    PhysicalUsbBackingConflict,
    /// The broker could not open the trusted physical device.
    BrokerInaccessible,
    /// The effect was rejected before any fd was returned.
    EffectRejected,
    /// The operation can be retried.
    Transient,
}

impl SecurityKeyEffectError {
    /// Return the stable Device error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PhysicalUsbBackingConflict => "physical-usb-backing-conflict",
            Self::BrokerInaccessible => "device-broker-inaccessible",
            Self::EffectRejected => "effect-rejected",
            Self::Transient => "transient",
        }
    }
}

impl fmt::Display for SecurityKeyEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SecurityKeyEffectError {}

/// Core effect port. The returned fd is represented only by an opaque
/// LaunchTicket token and is never exposed as a path.
pub trait SecurityKeyEffectPort {
    /// Acquire the single Host physical-device authority.
    fn claim_physical_backing(
        &mut self,
        claim: &PhysicalUsbBackingClaim,
    ) -> Result<PhysicalAuthorityLease, SecurityKeyEffectError>;
    /// Open the exact hidraw node and place the fd in the relay LaunchTicket.
    fn open_hidraw(
        &mut self,
        intent: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError>;
    /// Release the authority after the session ends.
    fn release_physical_backing(
        &mut self,
        lease: PhysicalAuthorityLease,
    ) -> Result<(), SecurityKeyEffectError>;
}

/// Opaque physical authority lease.
#[derive(Clone, PartialEq, Eq)]
pub struct PhysicalAuthorityLease([u8; 16]);

impl PhysicalAuthorityLease {
    /// Construct at the Core adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for PhysicalAuthorityLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PhysicalAuthorityLease(<redacted>)")
    }
}

/// Opaque relay LaunchTicket containing the broker-returned fd.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayLaunchTicket([u8; 16]);

impl RelayLaunchTicket {
    /// Construct at the Core adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for RelayLaunchTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelayLaunchTicket(<redacted>)")
    }
}
