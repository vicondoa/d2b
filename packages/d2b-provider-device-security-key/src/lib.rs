//! Security-key Device Provider contracts.
//!
//! This crate owns the unprivileged relay/frontend Process declarations and
//! the bounded lease/session protocol. Core alone resolves the physical
//! hidraw effect and places the returned fd in the relay LaunchTicket.

#![deny(missing_docs)]

mod authority;
mod cid;
mod controller;
mod lease;
mod process;
mod session_ring;

pub use authority::{
    PhysicalUsbBackingClaim, PhysicalUsbBackingToken, SecurityKeyEffectError,
    SecurityKeyEffectPort, SecurityKeyOpenIntent,
};
pub use cid::{CidTranslationError, GuestCid, RelayCid, SecurityKeyCidTranslator};
pub use controller::{
    SecurityKeyController, SecurityKeyControllerError, SecurityKeyReconcileOutcome,
};
pub use lease::{LeaseState, SecurityKeyLease, SecurityKeyLeaseError, SecurityKeySessionId};
pub use process::{
    FrontendProcessDeclaration, ProcessDeclarationError, SecurityKeyProcessRole,
    security_key_process_name,
};
pub use session_ring::{SessionRecord, SessionResult, SessionRing, SessionRingError};

/// Provider identity.
pub const PROVIDER_REF: &str = "Provider/device-security-key";
/// Device extension schema identifier.
pub const DEVICE_SECURITY_KEY_SCHEMA_ID: &str = "device-security-key.d2bus.org/Device/spec";
/// Device Provider finalizer.
pub const DEVICE_SECURITY_KEY_FINALIZER: &str = "device-security-key.d2bus.org/lease-released";
/// Stable default Host↔Guest vsock port.
pub const DEFAULT_VSOCK_PORT: u16 = 14_320;
/// Minimum bounded recent-session ring.
pub const MIN_SESSION_RING_SIZE: usize = 8;
/// Maximum bounded recent-session ring.
pub const MAX_SESSION_RING_SIZE: usize = 256;
/// Default bounded recent-session ring.
pub const DEFAULT_SESSION_RING_SIZE: usize = 32;
/// Minimum lease timeout in seconds.
pub const MIN_LEASE_TIMEOUT_SECS: u64 = 30;
/// Maximum lease timeout in seconds.
pub const MAX_LEASE_TIMEOUT_SECS: u64 = 3_600;
/// Default lease timeout in seconds.
pub const DEFAULT_LEASE_TIMEOUT_SECS: u64 = 300;
